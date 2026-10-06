//! Mock of the customer API shared by the integration tests. It enforces token rotation, so a
//! reused refresh token shows up as a recorded violation.
#![allow(dead_code)]

use app_view::{AppClient, AppClientOptions, AppLoginResult};
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

pub const EMAIL: &str = "tester@example.de";
pub const PASSWORD: &str = "Geheimes-Testpasswort-42";
pub const DEVICE: &str = "sim-device-0123456789";
pub const CODE: &str = "123456";
pub const INVITE: &str = "invite-geheim-abc";
pub const INST: &str = "inst-1";

#[derive(Default)]
pub struct Mock {
    pub counter: u32,
    pub known_devices: HashSet<String>,
    pub require_code: bool,
    pub code_attempts: u32,
    pub login_status_override: Option<u16>,
    pub login_delay_ms: u64,
    pub expire_access_after_dashboard: bool,
    pub valid_access: HashSet<String>,
    pub valid_refresh: HashSet<String>,
    pub refresh_tokens_received: Vec<String>,
    pub refresh_reuse: Vec<String>,
    pub logout_tokens_received: Vec<String>,
    pub path_status: HashMap<String, u16>,
    pub log: Vec<(String, String, Option<String>, Value)>,
    pub energy_wallbox: bool,
    pub refresh_status: Option<u16>,
    pub pad_bytes: HashMap<String, usize>,
}

pub type Shared = Arc<Mutex<Mock>>;

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
            if let Some(status) = m.refresh_status {
                return err(status, "kaputt");
            }
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
            let mut body = body;
            if let Some(n) = m.pad_bytes.get(p) {
                body["pad"] = Value::String("x".repeat(*n));
            }
            reply(200, body)
        }
        _ => err(404, "nicht_gefunden"),
    }
}

pub async fn start() -> (String, Shared) {
    let (url, shared, _handle) = start_abortable().await;
    (url, shared)
}

pub async fn start_abortable() -> (String, Shared, tokio::task::JoinHandle<()>) {
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

pub fn count(shared: &Shared, method: &str, path_prefix: &str) -> usize {
    shared
        .lock()
        .unwrap()
        .log
        .iter()
        .filter(|(m, p, _, _)| m == method && p.starts_with(path_prefix))
        .count()
}

pub async fn logged_in(options: AppClientOptions) -> (AppClient, String, Shared) {
    let (url, shared) = start().await;
    let client = AppClient::with_options(options).unwrap();
    let result = client.login(&url, EMAIL, PASSWORD, DEVICE).await.unwrap();
    assert_eq!(result, AppLoginResult::Ok);
    (client, url, shared)
}

pub fn assert_no_secrets(text: &str) {
    for secret in [PASSWORD, CODE, INVITE, "acc-", "ref-", "mfa-secret"] {
        assert!(
            !text.contains(secret),
            "secret {secret:?} leaked in {text:?}"
        );
    }
}
