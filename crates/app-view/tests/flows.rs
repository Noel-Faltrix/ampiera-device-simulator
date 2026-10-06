//! Login, device code, refresh, logout and error flows against the mock.

use app_view::{AppClient, AppClientOptions, AppError, AppLoginResult};
use common::*;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

mod common;

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
    })
    .unwrap();
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
    })
    .unwrap();
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

#[tokio::test]
async fn texts_use_informal_du_and_no_please() {
    let all = [
        AppError::NotLoggedIn,
        AppError::TooManyRequests,
        AppError::CodeNotDeliverable,
        AppError::NoPendingDeviceCode,
        AppError::DeviceTokenInvalid,
        AppError::CodeWrong,
        AppError::CodeExpired,
        AppError::CodeTooManyAttempts,
        AppError::PasswordLeaked,
        AppError::SessionExpired,
        AppError::RefreshUncertain,
        AppError::Network {
            cause: "Zeitüberschreitung",
        },
    ];
    for e in all {
        let text = e.to_string();
        assert!(
            !text.contains("Bitte") && !text.contains("code_nicht"),
            "{text}"
        );
    }
    assert_eq!(
        AppError::NotLoggedIn.to_string(),
        "Du bist nicht angemeldet. Melde dich zuerst in der App-Sicht an."
    );
}

#[tokio::test]
async fn refresh_answered_with_5xx_ends_the_session() {
    let options = AppClientOptions {
        snapshot_ttl: Duration::ZERO,
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    {
        let mut m = shared.lock().unwrap();
        m.valid_access.clear();
        m.refresh_status = Some(503);
    }
    assert_eq!(
        client.snapshot().await.unwrap_err(),
        AppError::RefreshUncertain
    );
    assert_eq!(client.snapshot().await.unwrap_err(), AppError::NotLoggedIn);
    assert_eq!(count(&shared, "POST", "/api/app/v1/auth/refresh"), 1);
}

#[tokio::test]
async fn refresh_answered_with_429_keeps_the_session() {
    let options = AppClientOptions {
        snapshot_ttl: Duration::ZERO,
        ..AppClientOptions::default()
    };
    let (client, _url, shared) = logged_in(options).await;
    {
        let mut m = shared.lock().unwrap();
        m.valid_access.clear();
        m.refresh_status = Some(429);
    }
    let snap = client.snapshot().await.unwrap();
    let text = snap.raw["/api/app/v1/dashboard"]["error"].as_str().unwrap();
    assert!(text.contains("Zu viele Versuche"), "{text}");
    shared.lock().unwrap().refresh_status = None;
    assert!(
        client.snapshot().await.is_ok(),
        "same refresh token is used again after 429"
    );
}

#[tokio::test]
async fn failed_relogin_keeps_the_working_session() {
    let (client, url, shared) = logged_in(AppClientOptions::default()).await;
    let e = client
        .login(&url, EMAIL, "falsches-Passwort-999", DEVICE)
        .await
        .unwrap_err();
    assert_eq!(e, AppError::InvalidCredentials);
    assert!(client.snapshot().await.is_ok());
    assert!(
        shared.lock().unwrap().logout_tokens_received.is_empty(),
        "old token must not be revoked"
    );
}

#[tokio::test]
async fn successful_relogin_revokes_the_old_refresh_token() {
    let (client, url, shared) = logged_in(AppClientOptions::default()).await;
    assert_eq!(
        client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap(),
        AppLoginResult::Ok
    );
    assert_eq!(
        shared.lock().unwrap().logout_tokens_received,
        vec!["ref-1".to_owned()]
    );
    client.snapshot().await.unwrap();
    let auth = shared
        .lock()
        .unwrap()
        .log
        .iter()
        .find(|l| l.1 == "/api/app/v1/dashboard")
        .unwrap()
        .2
        .clone();
    assert_eq!(auth.as_deref(), Some("Bearer acc-2"));
}

#[tokio::test]
async fn pending_device_code_does_not_destroy_the_active_session() {
    let (client, url, shared) = logged_in(AppClientOptions::default()).await;
    shared.lock().unwrap().require_code = true;
    assert_eq!(
        client
            .login(&url, EMAIL, PASSWORD, "andere-geraete-id-1")
            .await
            .unwrap(),
        AppLoginResult::DeviceCodeRequired
    );
    assert!(
        client.snapshot().await.is_ok(),
        "old session still works while the code is pending"
    );
    client.verify_device(CODE).await.unwrap();
    assert_eq!(shared.lock().unwrap().logout_tokens_received.len(), 1);
}
