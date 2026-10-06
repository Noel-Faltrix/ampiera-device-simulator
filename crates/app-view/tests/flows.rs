//! HTTP flows against a local mock of the customer API. The mock enforces token rotation, so a
//! reused refresh token shows up as a recorded violation.

use app_view::{AppClient, AppClientOptions, AppError, AppLoginResult};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use chrono::{Duration as Delta, SecondsFormat, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const EMAIL: &str = "tester@example.de";
const PASSWORD: &str = "Geheimes-Testpasswort-42";
const DEVICE: &str = "sim-device-0123456789";
const CODE: &str = "123456";
const INVITE: &str = "invite-geheim-abc";
const INST: &str = "inst-1";

#[derive(Default)]
struct Mock {
    counter: u32,
    known_devices: HashSet<String>,
    require_code: bool,
    code_attempts: u32,
    login_status_override: Option<u16>,
    login_delay_ms: u64,
    expire_access_after_dashboard: bool,
    valid_access: HashSet<String>,
    valid_refresh: HashSet<String>,
    refresh_tokens_received: Vec<String>,
    refresh_reuse: Vec<String>,
    logout_tokens_received: Vec<String>,
    path_status: HashMap<String, u16>,
    log: Vec<(String, String, Option<String>, Value)>,
    energy_wallbox: bool,
}

type Shared = Arc<Mutex<Mock>>;

fn issue(m: &mut Mock) -> Value {
    m.counter += 1;
    let access = format!("acc-{}", m.counter);
    let refresh = format!("ref-{}", m.counter);
    m.valid_access.insert(access.clone());
    m.valid_refresh.insert(refresh.clone());
    json!({"access_token": access, "refresh_token": refresh, "expires_in": 900})
}

fn reply(status: u16, body: Value) -> Response {
    let status = StatusCode::from_u16(status).unwrap();
    if body.is_null() {
        return status.into_response();
    }
    (status, axum::Json(body)).into_response()
}

fn err(status: u16, code: &str) -> Response {
    reply(status, json!({ "error": code }))
}

fn dashboard() -> Value {
    json!({"anlagen": [{
        "id": INST, "bezeichnung": "Testhaus", "verbindung": "online",
        "letzter_kontakt": "2026-10-06T10:00:00Z",
        "geraete": [
            {"id": "g-pv", "typ": "pv", "status": "ok", "anbindung": "api"},
            {"id": "g-wb", "typ": "wallbox", "status": "offline", "anbindung": "ocpp"}],
        "live": {"wallbox_leistung_w": null}}], "wetter": null})
}

fn schedule() -> Value {
    let at = |h: i64| (Utc::now() + Delta::hours(h)).to_rfc3339_opts(SecondsFormat::Secs, true);
    json!({"fahrplan": {"schritte": [
        {"beginn": at(-1), "geraete_id": "g-wb", "geraete_typ": "wallbox", "aktion": "laden", "soll_leistung_w": 1},
        {"beginn": at(2), "geraete_id": "g-wb", "geraete_typ": "wallbox", "aktion": "laden", "soll_leistung_w": 7000},
        {"beginn": at(1), "geraete_id": "g-pv", "geraete_typ": "speicher", "aktion": "halten", "soll_leistung_w": 5},
        {"beginn": at(3), "geraete_id": "g-wb", "geraete_typ": "wallbox", "aktion": "halten", "soll_leistung_w": 0}]},
        "hinweise": []})
}

async fn handle(
    State(shared): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let path = uri.path().to_owned();
    let delay = {
        let mut m = shared.lock().unwrap();
        m.log.push((
            method.to_string(),
            uri.path_and_query()
                .map(|p| p.to_string())
                .unwrap_or_default(),
            auth.clone(),
            body.clone(),
        ));
        if path.ends_with("/auth/login") {
            m.login_delay_ms
        } else {
            0
        }
    };
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let mut m = shared.lock().unwrap();
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match (method.as_str(), path.as_str()) {
        ("POST", "/api/app/v1/auth/login") => {
            if let Some(status) = m.login_status_override {
                return err(
                    status,
                    if status == 503 {
                        "code_nicht_zustellbar"
                    } else {
                        "x"
                    },
                );
            }
            let id = text("geraete_id");
            if text("email") != EMAIL || text("password") != PASSWORD || id.len() < 8 {
                return err(401, "invalid_credentials");
            }
            if m.require_code && !m.known_devices.contains(&id) {
                return reply(
                    200,
                    json!({"mfa_required": true, "mfa_token": "mfa-secret-1"}),
                );
            }
            reply(200, issue(&mut m))
        }
        ("POST", "/api/app/v1/auth/verify-device") => {
            if text("mfa_token") != "mfa-secret-1" {
                return err(401, "mfa_token_ungueltig");
            }
            if text("code") != CODE {
                m.code_attempts += 1;
                return err(
                    401,
                    if m.code_attempts >= 3 {
                        "code_zu_viele_versuche"
                    } else {
                        "code_falsch"
                    },
                );
            }
            m.known_devices.insert(text("geraete_id"));
            reply(200, issue(&mut m))
        }
        ("POST", "/api/app/v1/auth/refresh") => {
            let token = text("refresh_token");
            m.refresh_tokens_received.push(token.clone());
            if m.refresh_tokens_received
                .iter()
                .filter(|t| **t == token)
                .count()
                > 1
            {
                m.refresh_reuse.push(token.clone());
            }
            if m.valid_refresh.remove(&token) {
                m.valid_access.clear();
                reply(200, issue(&mut m))
            } else {
                err(401, "refresh_ungueltig")
            }
        }
        ("POST", "/api/app/v1/auth/logout") => {
            let token = text("refresh_token");
            m.logout_tokens_received.push(token.clone());
            m.valid_refresh.remove(&token);
            reply(204, Value::Null)
        }
        ("POST", "/api/app/v1/auth/redeem-invite") => {
            let pw = text("password");
            if text("invite_token") != INVITE {
                err(400, "invalid_invite")
            } else if pw.chars().count() < 15 {
                err(400, "password_zu_kurz")
            } else if pw.chars().count() > 128 {
                err(400, "password_zu_lang")
            } else if pw == "passwortpasswort123" {
                err(400, "password_geleakt")
            } else {
                reply(204, Value::Null)
            }
        }
        ("GET", p) if p.starts_with("/api/app/v1/") => {
            let bearer = auth
                .as_deref()
                .and_then(|a| a.strip_prefix("Bearer "))
                .unwrap_or("");
            if !m.valid_access.contains(bearer) {
                return err(401, "unauthorized");
            }
            if let Some(status) = m.path_status.get(p) {
                return err(*status, "kaputt");
            }
            let body = match p.trim_start_matches("/api/app/v1/") {
                "dashboard" => {
                    if m.expire_access_after_dashboard {
                        m.valid_access.clear();
                    }
                    dashboard()
                }
                s if s == format!("evaluation/{INST}/geo-position") => json!({"geraete": [
                    {"typ": "wallbox", "zustand": "laedt", "leistungW": 7000, "ueberschrift": "Auto lädt"}]}),
                s if s == format!("schedule/{INST}") => schedule(),
                "device-energy" => {
                    let typ = if m.energy_wallbox { "wallbox" } else { "pv" };
                    json!({"geraete": [{"typ": typ, "werte": [
                        {"zeit": "2026-10-06T10:00:00Z", "kwhPositiv": 1.5, "kwhNegativ": 0, "abdeckungPct": 100},
                        {"zeit": "2026-10-06T10:15:00Z", "kwhPositiv": 1.75, "kwhNegativ": 0, "abdeckungPct": 100}]}]})
                }
                _ => return err(404, "nicht_gefunden"),
            };
            reply(200, body)
        }
        _ => err(404, "nicht_gefunden"),
    }
}

async fn start() -> (String, Shared) {
    let (url, shared, _handle) = start_abortable().await;
    (url, shared)
}

async fn start_abortable() -> (String, Shared, tokio::task::JoinHandle<()>) {
    let shared: Shared = Arc::new(Mutex::new(Mock {
        energy_wallbox: true,
        ..Mock::default()
    }));
    let app = Router::new().fallback(handle).with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, shared, handle)
}

fn count(shared: &Shared, method: &str, path_prefix: &str) -> usize {
    shared
        .lock()
        .unwrap()
        .log
        .iter()
        .filter(|(m, p, _, _)| m == method && p.starts_with(path_prefix))
        .count()
}

async fn logged_in(options: AppClientOptions) -> (AppClient, String, Shared) {
    let (url, shared) = start().await;
    let client = AppClient::with_options(options);
    let result = client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap();
    assert_eq!(result, AppLoginResult::Ok);
    (client, url, shared)
}

fn assert_no_secrets(text: &str) {
    for secret in [PASSWORD, CODE, INVITE, "acc-", "ref-", "mfa-secret"] {
        assert!(
            !text.contains(secret),
            "secret {secret:?} leaked in {text:?}"
        );
    }
}

#[tokio::test]
async fn login_ok_sends_the_documented_body_and_stores_tokens() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    let body = shared.lock().unwrap().log[0].3.clone();
    assert_eq!(
        body,
        json!({"email": EMAIL, "password": PASSWORD, "geraete_id": DEVICE, "geraete_name": "Ampiera Device Simulator"})
    );
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.installation_id.as_deref(), Some(INST));
    let auth = shared
        .lock()
        .unwrap()
        .log
        .iter()
        .find(|l| l.1 == "/api/app/v1/dashboard")
        .unwrap()
        .2
        .clone();
    assert_eq!(auth.as_deref(), Some("Bearer acc-1"));
}

#[tokio::test]
async fn login_with_wrong_password_is_generic_and_leaks_nothing() {
    let (url, _shared) = start().await;
    let client = AppClient::new();
    let e = client
        .login(&url, EMAIL, "falsches-Passwort-999", DEVICE)
        .await
        .unwrap_err();
    assert_eq!(e, AppError::InvalidCredentials);
    assert_eq!(
        e.to_string(),
        "Anmeldung abgelehnt (Server meldet ungültige Zugangsdaten)."
    );
    assert!(!format!("{e} {e:?}").contains("falsches-Passwort"));
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);
}

#[tokio::test]
async fn login_maps_rate_limit_and_undeliverable_code() {
    let (url, shared) = start().await;
    let client = AppClient::new();
    shared.lock().unwrap().login_status_override = Some(429);
    assert_eq!(
        client
            .login(&url, EMAIL, PASSWORD, DEVICE)
            .await
            .unwrap_err(),
        AppError::TooManyRequests
    );
    shared.lock().unwrap().login_status_override = Some(503);
    assert_eq!(
        client
            .login(&url, EMAIL, PASSWORD, DEVICE)
            .await
            .unwrap_err(),
        AppError::CodeNotDeliverable
    );
    shared.lock().unwrap().login_status_override = Some(500);
    let e = client
        .login(&url, EMAIL, PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Der Server antwortete unerwartet (HTTP 500, Fehlercode x)."
    );
}

#[tokio::test]
async fn unknown_device_needs_a_code_and_the_flow_completes() {
    let (url, shared) = start().await;
    shared.lock().unwrap().require_code = true;
    let client = AppClient::new();
    assert_eq!(
        client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap(),
        AppLoginResult::DeviceCodeRequired
    );
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);

    // Bad format is rejected locally, without a request.
    for bad in ["12345", "1234567", "12345a", "١٢٣٤٥٦"] {
        assert!(
            matches!(
                client.verify_device(bad).await,
                Err(AppError::InvalidInput { .. })
            ),
            "{bad}"
        );
    }
    assert_eq!(count(&shared, "POST", "/api/app/v1/auth/verify-device"), 0);

    let wrong = client.verify_device("000000").await.unwrap_err();
    assert_eq!(wrong, AppError::CodeWrong);
    assert_no_secrets(&wrong.to_string());
    client.verify_device(CODE).await.unwrap();
    assert!(client.snapshot().await.is_ok());

    let sent = shared
        .lock()
        .unwrap()
        .log
        .iter()
        .rev()
        .find(|l| l.1.ends_with("verify-device"))
        .unwrap()
        .3
        .clone();
    assert_eq!(sent["mfa_token"], "mfa-secret-1");
    assert_eq!(sent["geraete_id"], DEVICE);
    assert_eq!(sent["geraete_name"], "Ampiera Device Simulator");

    // The device is known now: a second login needs no code.
    let again = AppClient::new();
    assert_eq!(
        again.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap(),
        AppLoginResult::Ok
    );
}

#[tokio::test]
async fn too_many_wrong_codes_end_the_pending_confirmation() {
    let (url, shared) = start().await;
    shared.lock().unwrap().require_code = true;
    let client = AppClient::new();
    client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap();
    assert_eq!(
        client.verify_device("000001").await.unwrap_err(),
        AppError::CodeWrong
    );
    assert_eq!(
        client.verify_device("000002").await.unwrap_err(),
        AppError::CodeWrong
    );
    assert_eq!(
        client.verify_device("000003").await.unwrap_err(),
        AppError::CodeTooManyAttempts
    );
    assert_eq!(
        client.verify_device(CODE).await.unwrap_err(),
        AppError::NoPendingDeviceCode
    );
}

#[tokio::test]
async fn verify_without_login_is_refused() {
    assert_eq!(
        AppClient::new().verify_device(CODE).await.unwrap_err(),
        AppError::NoPendingDeviceCode
    );
}

#[tokio::test]
async fn refresh_tokens_rotate_and_are_never_sent_twice() {
    let options = AppClientOptions {
        refresh_after: Duration::from_millis(300),
        snapshot_ttl: Duration::ZERO,
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    client.snapshot().await.unwrap();
    assert_eq!(count(&shared, "POST", "/api/app/v1/auth/refresh"), 0);
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        client.snapshot().await.unwrap();
    }
    let m = shared.lock().unwrap();
    assert!(m.refresh_tokens_received.len() >= 2);
    assert_eq!(m.refresh_tokens_received[0], "ref-1");
    assert_eq!(m.refresh_tokens_received[1], "ref-2");
    assert!(m.refresh_reuse.is_empty(), "reused: {:?}", m.refresh_reuse);
    let unique: HashSet<_> = m.refresh_tokens_received.iter().collect();
    assert_eq!(unique.len(), m.refresh_tokens_received.len());
}

#[tokio::test]
async fn a_401_triggers_exactly_one_refresh_and_one_retry() {
    let options = AppClientOptions {
        snapshot_ttl: Duration::ZERO,
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    shared.lock().unwrap().valid_access.clear();
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.summary.connection.as_deref(), Some("online"));
    assert_eq!(count(&shared, "POST", "/api/app/v1/auth/refresh"), 1);
    // dashboard: rejected once, then retried with the new token.
    assert_eq!(count(&shared, "GET", "/api/app/v1/dashboard"), 2);
    assert!(shared.lock().unwrap().refresh_reuse.is_empty());
}

#[tokio::test]
async fn parallel_requests_hitting_401_together_share_one_refresh() {
    let options = AppClientOptions {
        snapshot_ttl: Duration::ZERO,
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    shared.lock().unwrap().expire_access_after_dashboard = true;
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.summary.geo_state.as_deref(), Some("laedt"));
    assert!(snap.summary.last_quarter_kwh.is_some());
    assert_eq!(count(&shared, "POST", "/api/app/v1/auth/refresh"), 1);
    assert!(shared.lock().unwrap().refresh_reuse.is_empty());
}

#[tokio::test]
async fn rejected_refresh_ends_the_session_with_a_clear_message() {
    let options = AppClientOptions {
        snapshot_ttl: Duration::ZERO,
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    {
        let mut m = shared.lock().unwrap();
        m.valid_access.clear();
        m.valid_refresh.clear();
    }
    let e = client.snapshot().await.unwrap_err();
    assert_eq!(e, AppError::SessionExpired);
    assert_no_secrets(&e.to_string());
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);
    assert_eq!(count(&shared, "POST", "/api/app/v1/auth/refresh"), 1);
}

#[tokio::test]
async fn logout_revokes_the_current_refresh_token_and_clears_state() {
    let (client, _url, shared) = logged_in(AppClientOptions::default()).await;
    client.snapshot().await.unwrap();
    let gets_before = count(&shared, "GET", "");
    client.logout().await.unwrap();
    assert_eq!(
        shared.lock().unwrap().logout_tokens_received,
        vec!["ref-1".to_owned()]
    );
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);
    assert_eq!(
        count(&shared, "GET", ""),
        gets_before,
        "no request after logout"
    );
    // Idempotent and silent when nothing is stored.
    client.logout().await.unwrap();
    assert_eq!(shared.lock().unwrap().logout_tokens_received.len(), 1);
}

#[tokio::test]
async fn logout_clears_tokens_even_if_the_server_is_gone() {
    let (url, _shared, server) = start_abortable().await;
    let client = AppClient::new();
    client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap();
    server.abort();
    let _ = server.await;
    client.logout().await.unwrap();
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);
}

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
async fn redeem_invite_succeeds_and_maps_every_documented_error() {
    let (url, _shared) = start().await;
    let client = AppClient::new();
    let long = "x".repeat(129);
    client
        .redeem_invite(&url, EMAIL, INVITE, "ein-langes-passwort-123")
        .await
        .unwrap();
    let cases = [
        (INVITE, "k3-abc", AppError::PasswordTooShort),
        (INVITE, long.as_str(), AppError::PasswordTooLong),
        (INVITE, "passwortpasswort123", AppError::PasswordLeaked),
        (
            "falscher-code-zzz",
            "ein-langes-passwort-123",
            AppError::InviteInvalid,
        ),
    ];
    for (invite, password, expected) in cases {
        let e = client
            .redeem_invite(&url, EMAIL, invite, password)
            .await
            .unwrap_err();
        assert_eq!(e, expected);
        let text = format!("{e} {e:?}");
        assert!(!text.contains(invite) && !text.contains(password), "{text}");
    }
    assert_eq!(
        client.snapshot().await.unwrap_err(),
        AppError::NotLoggedIn,
        "redeem must not log in"
    );
}

#[tokio::test]
async fn bad_base_urls_are_refused_before_any_request() {
    let client = AppClient::new();
    for url in [
        "http://api.ampiera.de",
        "ftp://localhost",
        "kein url",
        "https://nutzer:geheim@api.ampiera.de",
    ] {
        let e = client
            .login(url, EMAIL, PASSWORD, DEVICE)
            .await
            .unwrap_err();
        assert!(matches!(e, AppError::InvalidBaseUrl { .. }), "{url}");
        assert!(!e.to_string().contains("geheim"));
    }
    // Private networks may use plain http, like the local backend.
    let quick = AppClient::with_options(AppClientOptions {
        request_timeout: Duration::from_millis(200),
        ..AppClientOptions::default()
    });
    let e = quick
        .login("http://192.168.10.5:1", EMAIL, PASSWORD, DEVICE)
        .await;
    assert!(!matches!(e, Err(AppError::InvalidBaseUrl { .. })));
}

#[tokio::test]
async fn device_id_length_is_checked_locally() {
    let client = AppClient::new();
    for id in ["1234567", &"x".repeat(201)] {
        let e = client
            .login("http://localhost:1", EMAIL, PASSWORD, id)
            .await
            .unwrap_err();
        assert!(matches!(e, AppError::InvalidInput { .. }));
    }
}

#[tokio::test]
async fn unreachable_server_and_timeout_have_clear_messages() {
    let dead = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let e = AppClient::new()
        .login(&dead, EMAIL, PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert_eq!(
        e,
        AppError::Network {
            cause: "Verbindung fehlgeschlagen"
        }
    );
    assert!(!e.to_string().contains("127.0.0.1"), "URL must not appear");

    let (url, shared) = start().await;
    shared.lock().unwrap().login_delay_ms = 1500;
    let client = AppClient::with_options(AppClientOptions {
        request_timeout: Duration::from_millis(200),
        ..AppClientOptions::default()
    });
    let e = client
        .login(&url, EMAIL, PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert_eq!(
        e,
        AppError::Network {
            cause: "Zeitüberschreitung"
        }
    );
    assert_no_secrets(&e.to_string());
}

#[tokio::test]
async fn login_result_serializes_as_tagged_snake_case() {
    assert_eq!(
        serde_json::to_value(AppLoginResult::Ok).unwrap(),
        json!({"result": "ok"})
    );
    assert_eq!(
        serde_json::to_value(AppLoginResult::DeviceCodeRequired).unwrap(),
        json!({"result": "device_code_required"})
    );
}
