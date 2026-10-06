//! Snapshot content, caching and per-endpoint failures against the mock.

use app_view::{AppClientOptions, AppError};
use common::*;
use std::time::Duration;

mod common;

#[tokio::test]
async fn snapshot_fills_summary_raw_and_query() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    let snap = client.snapshot().await.unwrap();
    let s = &snap.summary;
    assert_eq!(s.connection.as_deref(), Some("online"));
    assert_eq!(s.last_contact.as_deref(), Some("2026-10-06T10:00:00Z"));
    assert_eq!(s.device_status.as_deref(), Some("offline"));
    assert_eq!(s.live_power_w, None, "null live power must stay null");
    assert_eq!(s.geo_state.as_deref(), Some("laedt"));
    assert_eq!(s.geo_headline.as_deref(), Some("Auto lädt"));
    assert_eq!(s.last_quarter_kwh, Some(1.75));
    assert_eq!(s.last_quarter_at.as_deref(), Some("2026-10-06T10:15:00Z"));
    let steps: Vec<(&str, f64)> = s
        .next_schedule_steps
        .iter()
        .map(|x| (x.action.as_str(), x.target_power_w))
        .collect();
    assert_eq!(
        steps,
        [("laden", 7000.0), ("halten", 0.0)],
        "past and non-wallbox steps excluded"
    );

    let mut keys: Vec<&String> = snap.raw.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "/api/app/v1/dashboard",
            "/api/app/v1/device-energy",
            "/api/app/v1/evaluation/inst-1/geo-position",
            "/api/app/v1/schedule/inst-1"
        ]
    );
    let query = shared
        .lock()
        .unwrap()
        .log
        .iter()
        .find(|l| l.1.starts_with("/api/app/v1/device-energy"))
        .unwrap()
        .1
        .clone();
    assert!(
        query.contains("aufloesung=viertelstunde") && query.contains("anlageId=inst-1"),
        "{query}"
    );
    assert!(
        query.contains("von=20") && query.contains("bis=20"),
        "{query}"
    );

    let json = serde_json::to_value(&snap).unwrap();
    assert!(json["fetchedAt"].is_string());
    assert_eq!(json["installationId"], INST);
    assert!(json["summary"]["livePowerW"].is_null());
    assert_eq!(
        json["summary"]["nextScheduleSteps"][0]["targetPowerW"],
        7000.0
    );
}

#[tokio::test]
async fn snapshot_without_wallbox_energy_stays_null() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    shared.lock().unwrap().energy_wallbox = false;
    let s = client.snapshot().await.unwrap().summary;
    assert_eq!(s.last_quarter_kwh, None);
    assert_eq!(s.last_quarter_at, None);
}

#[tokio::test]
async fn snapshot_is_cached_and_login_or_logout_invalidate_the_cache() {
    let (client, url, shared) = logged_in(AppClientOptions::default()).await;
    let first = client.snapshot().await.unwrap();
    let second = client.snapshot().await.unwrap();
    assert_eq!(first, second);
    assert_eq!(count(&shared, "GET", "/api/app/v1/dashboard"), 1);
    client.logout().await.unwrap();
    client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap();
    client.snapshot().await.unwrap();
    assert_eq!(count(&shared, "GET", "/api/app/v1/dashboard"), 2);
}

#[tokio::test]
async fn expired_cache_fetches_again() {
    let options = AppClientOptions {
        snapshot_ttl: Duration::from_millis(100),
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    client.snapshot().await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    client.snapshot().await.unwrap();
    assert_eq!(count(&shared, "GET", "/api/app/v1/dashboard"), 2);
}

#[tokio::test]
async fn one_failing_endpoint_does_not_fail_the_snapshot() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    shared
        .lock()
        .unwrap()
        .path_status
        .insert("/api/app/v1/schedule/inst-1".into(), 500);
    let snap = client.snapshot().await.unwrap();
    let text = snap.raw["/api/app/v1/schedule/inst-1"]["error"]
        .as_str()
        .unwrap();
    assert!(text.contains("HTTP 500"), "{text}");
    assert!(snap.summary.next_schedule_steps.is_empty());
    assert_eq!(snap.summary.connection.as_deref(), Some("online"));
    assert_eq!(snap.summary.last_quarter_kwh, Some(1.75));
}

#[tokio::test]
async fn failing_dashboard_marks_dependent_endpoints_as_skipped() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    shared
        .lock()
        .unwrap()
        .path_status
        .insert("/api/app/v1/dashboard".into(), 502);
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.installation_id, None);
    assert_eq!(snap.summary.connection, None);
    assert_eq!(snap.raw.len(), 4);
    for (path, value) in &snap.raw {
        assert!(value["error"].is_string(), "{path}");
    }
    assert_eq!(count(&shared, "GET", "/api/app/v1/schedule"), 0);
}

#[tokio::test]
async fn oversized_response_is_refused() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    shared
        .lock()
        .unwrap()
        .pad_bytes
        .insert("/api/app/v1/dashboard".into(), 3 * 1024 * 1024);
    let snap = client.snapshot().await.unwrap();
    assert_eq!(
        snap.raw["/api/app/v1/dashboard"]["error"],
        "Antwort des Servers zu groß."
    );
    assert_eq!(snap.summary.connection, None);
}

#[tokio::test]
async fn large_but_accepted_body_is_summarised_and_not_shown_in_raw() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    shared.lock().unwrap().pad_bytes.insert(
        "/api/app/v1/evaluation/inst-1/geo-position".into(),
        400 * 1024,
    );
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.summary.geo_state.as_deref(), Some("laedt"));
    assert_eq!(
        snap.raw["/api/app/v1/evaluation/inst-1/geo-position"]["error"],
        "Antwort zu groß für die Anzeige"
    );
}

#[tokio::test]
async fn failed_snapshot_is_cached_too_but_a_lost_session_is_not() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    shared
        .lock()
        .unwrap()
        .path_status
        .insert("/api/app/v1/dashboard".into(), 500);
    client.snapshot().await.unwrap();
    client.snapshot().await.unwrap();
    assert_eq!(
        count(&shared, "GET", "/api/app/v1/dashboard"),
        1,
        "failed attempt served from cache"
    );

    // Total failure: server unreachable for the refresh path is a session loss and is retried.
    let options = AppClientOptions {
        snapshot_ttl: Duration::from_secs(30),
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    {
        let mut m = shared.lock().unwrap();
        m.valid_access.clear();
        m.valid_refresh.clear();
    }
    assert_eq!(
        client.snapshot().await.unwrap_err(),
        AppError::SessionExpired
    );
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);
}
