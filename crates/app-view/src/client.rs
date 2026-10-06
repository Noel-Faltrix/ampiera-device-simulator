//! HTTP client for the customer API. Tokens live in memory only. Refresh tokens rotate, so the
//! session lock is held across every refresh request and a refresh token is never sent twice.

use crate::error::AppError;
use crate::extract;
use crate::types::{AppLoginResult, AppSummary, AppViewSnapshot};
use chrono::Utc;
use reqwest::{redirect, Url};
use serde_json::{json, Map, Value};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

const DEVICE_NAME: &str = "Ampiera Device Simulator";
const API: [&str; 3] = ["api", "app", "v1"];
const DEVICE_ID_CHARS: std::ops::RangeInclusive<usize> = 8..=200;
/// No answer of the customer API is larger than this; anything bigger is a fault or an attack.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Largest body that is copied into `raw` for display.
const MAX_RAW_BYTES: usize = 256 * 1024;

/// Timing parameters. The defaults are the production values; tests shorten them.
#[derive(Debug, Clone, Copy)]
pub struct AppClientOptions {
    /// Per-request timeout. The backend hashes with Argon2 even for unknown users, so logins are slow.
    pub request_timeout: Duration,
    /// Age of the access token after which it is refreshed before use. The server issues 15 minutes.
    pub refresh_after: Duration,
    /// How long `snapshot` serves the cached result.
    pub snapshot_ttl: Duration,
}

impl Default for AppClientOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(20),
            refresh_after: Duration::from_secs(12 * 60),
            snapshot_ttl: Duration::from_secs(30),
        }
    }
}

struct Tokens {
    access: String,
    refresh: String,
    refresh_at: Instant,
}

/// A login that waits for the e-mailed device code. Kept apart from the active session so a failed
/// re-login never destroys a working one.
struct Pending {
    base_url: Url,
    device_id: String,
    mfa_token: String,
}

#[derive(Default)]
struct Session {
    base_url: Option<Url>,
    tokens: Option<Tokens>,
    pending: Option<Pending>,
}

struct CachedSnapshot {
    generation: u64,
    fetched: Instant,
    result: Result<AppViewSnapshot, AppError>,
}

struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Why no reply could be read.
enum Failure {
    Transport(reqwest::Error),
    TooLarge,
}

impl Failure {
    fn app(&self) -> AppError {
        match self {
            Failure::Transport(e) => network_error(e),
            Failure::TooLarge => AppError::ResponseTooLarge,
        }
    }
}

/// Read-only client for the customer API of one backend. Safe to share behind an `Arc`.
pub struct AppClient {
    http: reqwest::Client,
    options: AppClientOptions,
    session: Mutex<Session>,
    cache: Mutex<Option<CachedSnapshot>>,
    // Bumped whenever the login state changes so a snapshot fetched for an old session is dropped.
    generation: AtomicU64,
}

impl Default for AppClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AppClient {
    /// Client with production timing.
    ///
    /// # Panics
    /// If the TLS stack cannot be initialised. A client without certificate validation is never
    /// substituted. The shell uses [`AppClient::try_new`] to report this instead.
    pub fn new() -> Self {
        Self::try_new().expect("TLS-Initialisierung des HTTP-Clients fehlgeschlagen")
    }

    /// Client with production timing; fails if the HTTP client cannot be built.
    pub fn try_new() -> Result<Self, AppError> {
        Self::with_options(AppClientOptions::default())
    }

    /// Client with custom timing. Certificate validation stays on in every case.
    pub fn with_options(options: AppClientOptions) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(options.request_timeout)
            // A redirect would carry the bearer token to whatever host the server names.
            .redirect(redirect::Policy::none())
            .user_agent(concat!(
                "Ampiera-Device-Simulator/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|_| AppError::ClientInit)?;
        Ok(Self {
            http,
            options,
            session: Mutex::new(Session::default()),
            cache: Mutex::new(None),
            generation: AtomicU64::new(0),
        })
    }

    /// Redeems an invite and sets the first password. Does not log in.
    pub async fn redeem_invite(
        &self,
        base_url: &str,
        email: &str,
        invite_token: &str,
        password: &str,
    ) -> Result<(), AppError> {
        let base = parse_base(base_url)?;
        require_filled(email, "Die E-Mail-Adresse fehlt.")?;
        require_filled(invite_token, "Der Einladungscode fehlt.")?;
        require_filled(password, "Das Passwort fehlt.")?;
        let body = json!({"email": email.trim(), "invite_token": invite_token.trim(), "password": password});
        let reply = self
            .post(&endpoint(&base, &["auth", "redeem-invite"]), &body)
            .await
            .map_err(|e| e.app())?;
        if reply.ok() {
            return Ok(());
        }
        Err(match (reply.status, error_code(&reply.body)) {
            (400, Some("invalid_invite")) => AppError::InviteInvalid,
            (400, Some("password_zu_kurz")) => AppError::PasswordTooShort,
            (400, Some("password_zu_lang")) => AppError::PasswordTooLong,
            (400, Some("password_geleakt")) => AppError::PasswordLeaked,
            (429, _) => AppError::TooManyRequests,
            _ => server_error(&reply),
        })
    }

    /// Signs in. A new device gets a code by e-mail first; the result then is
    /// `DeviceCodeRequired` and `verify_device` finishes the login.
    pub async fn login(
        &self,
        base_url: &str,
        email: &str,
        password: &str,
        device_id: &str,
    ) -> Result<AppLoginResult, AppError> {
        let base = parse_base(base_url)?;
        require_filled(email, "Die E-Mail-Adresse fehlt.")?;
        require_filled(password, "Das Passwort fehlt.")?;
        if !DEVICE_ID_CHARS.contains(&device_id.chars().count()) {
            return Err(AppError::InvalidInput {
                reason: "Die Geräte-Kennung muss 8 bis 200 Zeichen lang sein.",
            });
        }
        let mut session = self.session.lock().await;
        // A new attempt replaces an unfinished one, but the active session stays untouched.
        session.pending = None;

        let body = json!({
            "email": email.trim(),
            "password": password,
            "geraete_id": device_id,
            "geraete_name": DEVICE_NAME,
        });
        let reply = self
            .post(&endpoint(&base, &["auth", "login"]), &body)
            .await
            .map_err(|e| e.app())?;
        match (reply.status, error_code(&reply.body)) {
            (200, _) => {}
            (401, _) => return Err(AppError::InvalidCredentials),
            (429, _) => return Err(AppError::TooManyRequests),
            (503, Some("code_nicht_zustellbar")) => return Err(AppError::CodeNotDeliverable),
            _ => return Err(server_error(&reply)),
        }
        if let Some(tokens) = self.parse_tokens(&reply.body) {
            let old = self.replace_session(&mut session, base, tokens);
            drop(session);
            self.revoke(old).await;
            return Ok(AppLoginResult::Ok);
        }
        let mfa = reply.body.get("mfa_required").and_then(Value::as_bool) == Some(true);
        match reply.body.get("mfa_token").and_then(Value::as_str) {
            Some(token) if mfa && !token.is_empty() => {
                session.pending = Some(Pending {
                    base_url: base,
                    device_id: device_id.to_owned(),
                    mfa_token: token.to_owned(),
                });
                Ok(AppLoginResult::DeviceCodeRequired)
            }
            _ => Err(AppError::InvalidResponse {
                what: "weder Token noch Gerätecode-Anforderung",
            }),
        }
    }

    /// Completes a login that asked for a device code.
    pub async fn verify_device(&self, code: &str) -> Result<(), AppError> {
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AppError::InvalidInput {
                reason: "Der Code besteht aus genau 6 Ziffern.",
            });
        }
        let mut session = self.session.lock().await;
        let Some(pending) = session.pending.as_ref() else {
            return Err(AppError::NoPendingDeviceCode);
        };
        let base = pending.base_url.clone();
        let body = json!({
            "mfa_token": pending.mfa_token,
            "code": code,
            "geraete_id": pending.device_id,
            "geraete_name": DEVICE_NAME,
        });
        let reply = self
            .post(&endpoint(&base, &["auth", "verify-device"]), &body)
            .await
            .map_err(|e| e.app())?;
        if reply.ok() {
            let tokens = self
                .parse_tokens(&reply.body)
                .ok_or(AppError::InvalidResponse {
                    what: "Token fehlen",
                })?;
            session.pending = None;
            let old = self.replace_session(&mut session, base, tokens);
            drop(session);
            self.revoke(old).await;
            return Ok(());
        }
        let error = match (reply.status, error_code(&reply.body)) {
            (401, Some("mfa_token_ungueltig")) => AppError::DeviceTokenInvalid,
            (401, Some("code_falsch")) => AppError::CodeWrong,
            (401, Some("code_abgelaufen")) => AppError::CodeExpired,
            (401, Some("code_zu_viele_versuche")) => AppError::CodeTooManyAttempts,
            (429, _) => AppError::TooManyRequests,
            _ => server_error(&reply),
        };
        // A wrong code can be retyped; every other failure needs a fresh login and a fresh code.
        if !matches!(error, AppError::CodeWrong | AppError::TooManyRequests) {
            session.pending = None;
        }
        Err(error)
    }

    /// Forgets all tokens and revokes the refresh token on the server. The local state is cleared
    /// even when the server cannot be reached; the refresh token then expires on its own.
    pub async fn logout(&self) -> Result<(), AppError> {
        let old = {
            let mut session = self.session.lock().await;
            session.pending = None;
            self.generation.fetch_add(1, Ordering::SeqCst);
            (session.base_url.clone(), session.tokens.take())
        };
        self.revoke(old).await;
        Ok(())
    }

    // Installs a new session and hands back the previous one for revocation.
    fn replace_session(
        &self,
        session: &mut Session,
        base: Url,
        tokens: Tokens,
    ) -> (Option<Url>, Option<Tokens>) {
        let old = (
            session.base_url.replace(base),
            session.tokens.replace(tokens),
        );
        self.generation.fetch_add(1, Ordering::SeqCst);
        old
    }

    // Best effort: the server answers 204 for every input, and an old refresh token that cannot be
    // revoked expires on its own.
    async fn revoke(&self, (base, tokens): (Option<Url>, Option<Tokens>)) {
        if let (Some(base), Some(tokens)) = (base, tokens) {
            let body = json!({"refresh_token": tokens.refresh});
            let _ = self
                .post(&endpoint(&base, &["auth", "logout"]), &body)
                .await;
        }
    }

    /// What the customer app shows right now. The result is cached for the configured TTL (30 s).
    /// A failing endpoint becomes `{"error": ...}` in `raw` and `None` in the summary; only a lost
    /// session fails the whole call.
    pub async fn snapshot(&self) -> Result<AppViewSnapshot, AppError> {
        let mut cache = self.cache.lock().await;
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some(cached) = cache.as_ref() {
            // Failures count as attempts too, so a broken server is not polled faster than 30 s.
            // A lost session is the exception: logging in again must work at once.
            let reusable = match &cached.result {
                Ok(_) => true,
                Err(e) => !e.is_session_loss(),
            };
            if reusable
                && cached.generation == generation
                && cached.fetched.elapsed() < self.options.snapshot_ttl
            {
                return cached.result.clone();
            }
        }
        let result = self.fetch_snapshot().await;
        *cache = Some(CachedSnapshot {
            generation,
            fetched: Instant::now(),
            result: result.clone(),
        });
        result
    }

    async fn fetch_snapshot(&self) -> Result<AppViewSnapshot, AppError> {
        let base = {
            let session = self.session.lock().await;
            if session.tokens.is_none() {
                return Err(AppError::NotLoggedIn);
            }
            session.base_url.clone().ok_or(AppError::NotLoggedIn)?
        };
        let now = Utc::now();
        let mut raw = Map::new();
        let mut summary = AppSummary::default();

        let dashboard_path = path_of(&["dashboard"]);
        let dashboard = self.authed_get(endpoint(&base, &["dashboard"])).await;
        let mut skipped = "Übersprungen, weil das Dashboard nicht geladen werden konnte.";
        let mut installation_id = None;
        if let Some(body) = fold(&mut raw, &dashboard_path, dashboard)? {
            let d = extract::extract_dashboard(&body);
            summary.connection = d.connection;
            summary.last_contact = d.last_contact;
            summary.device_status = d.device_status;
            summary.live_power_w = d.live_power_w;
            installation_id = d.installation_id;
            skipped = "Übersprungen, weil das Dashboard keine Anlage enthält.";
        }

        let Some(id) = installation_id.clone() else {
            for segments in [
                &["evaluation", "{id}", "geo-position"][..],
                &["schedule", "{id}"][..],
                &["device-energy"][..],
            ] {
                raw.insert(path_of(segments), error_json(skipped));
            }
            return Ok(AppViewSnapshot {
                fetched_at: now,
                installation_id: None,
                summary,
                raw,
            });
        };

        let (von, bis) = extract::berlin_date_range(now);
        let mut energy_url = endpoint(&base, &["device-energy"]);
        energy_url
            .query_pairs_mut()
            .append_pair("von", &von.to_string())
            .append_pair("bis", &bis.to_string())
            .append_pair("aufloesung", "viertelstunde")
            .append_pair("anlageId", &id);
        let geo_segments = ["evaluation", id.as_str(), "geo-position"];
        let schedule_segments = ["schedule", id.as_str()];
        let (geo, schedule, energy) = tokio::join!(
            self.authed_get(endpoint(&base, &geo_segments)),
            self.authed_get(endpoint(&base, &schedule_segments)),
            self.authed_get(energy_url),
        );

        if let Some(body) = fold(&mut raw, &path_of(&geo_segments), geo)? {
            let g = extract::extract_geo(&body);
            summary.geo_state = g.state;
            summary.geo_headline = g.headline;
        }
        if let Some(body) = fold(&mut raw, &path_of(&schedule_segments), schedule)? {
            summary.next_schedule_steps = extract::extract_schedule(&body, now);
        }
        if let Some(body) = fold(&mut raw, &path_of(&["device-energy"]), energy)? {
            let e = extract::extract_energy(&body);
            summary.last_quarter_kwh = e.kwh;
            summary.last_quarter_at = e.at;
        }
        Ok(AppViewSnapshot {
            fetched_at: now,
            installation_id,
            summary,
            raw,
        })
    }

    /// GET with bearer token: refreshes a stale token first and retries once after a 401.
    async fn authed_get(&self, url: Url) -> Result<Value, AppError> {
        let token = self.fresh_access_token().await?;
        let mut reply = self.get(&url, &token).await?;
        if reply.status == 401 {
            let token = self.access_after_401(&token).await?;
            reply = self.get(&url, &token).await?;
            if reply.status == 401 {
                return Err(AppError::SessionExpired);
            }
        }
        if !reply.ok() {
            return Err(match reply.status {
                429 => AppError::TooManyRequests,
                _ => server_error(&reply),
            });
        }
        if reply.body.is_null() {
            return Err(AppError::InvalidResponse {
                what: "leere Antwort",
            });
        }
        Ok(reply.body)
    }

    async fn fresh_access_token(&self) -> Result<String, AppError> {
        let mut session = self.session.lock().await;
        let stale = match session.tokens.as_ref() {
            Some(t) => Instant::now() >= t.refresh_at,
            None => return Err(AppError::NotLoggedIn),
        };
        if stale {
            self.refresh_locked(&mut session).await?;
        }
        session
            .tokens
            .as_ref()
            .map(|t| t.access.clone())
            .ok_or(AppError::NotLoggedIn)
    }

    /// Several requests of one snapshot can hit a 401 together. Only the first one refreshes; the
    /// others find a changed access token and reuse it.
    async fn access_after_401(&self, rejected: &str) -> Result<String, AppError> {
        let mut session = self.session.lock().await;
        match session.tokens.as_ref() {
            None => return Err(AppError::NotLoggedIn),
            Some(t) if t.access != rejected => return Ok(t.access.clone()),
            Some(_) => {}
        }
        self.refresh_locked(&mut session).await?;
        session
            .tokens
            .as_ref()
            .map(|t| t.access.clone())
            .ok_or(AppError::NotLoggedIn)
    }

    /// Exchanges the refresh token. The caller holds the session lock, so no second refresh can
    /// start. The old token is dropped in every outcome except "never reached the server".
    async fn refresh_locked(&self, session: &mut Session) -> Result<(), AppError> {
        let base = session.base_url.clone().ok_or(AppError::NotLoggedIn)?;
        let old = session
            .tokens
            .as_ref()
            .map(|t| t.refresh.clone())
            .ok_or(AppError::NotLoggedIn)?;
        let body = json!({"refresh_token": old});
        let sent = self
            .post(&endpoint(&base, &["auth", "refresh"]), &body)
            .await;
        match sent {
            Ok(reply) if reply.ok() => match self.parse_tokens(&reply.body) {
                Some(tokens) => {
                    session.tokens = Some(tokens);
                    Ok(())
                }
                None => {
                    self.end_session(session);
                    Err(AppError::RefreshUncertain)
                }
            },
            Ok(reply) if reply.status == 401 => {
                self.end_session(session);
                Err(AppError::SessionExpired)
            }
            // The server refused before looking at the token.
            Ok(reply) if reply.status == 429 => Err(AppError::TooManyRequests),
            // Not connected means the token was not used; keep it for the next attempt.
            Err(Failure::Transport(e)) if e.is_connect() => Err(network_error(&e)),
            // Any other answer (5xx included), a timeout or a broken body may come after the server
            // rotated the token, and the new one is lost. The old one must not be sent again.
            Ok(_) | Err(_) => {
                self.end_session(session);
                Err(AppError::RefreshUncertain)
            }
        }
    }

    fn end_session(&self, session: &mut Session) {
        session.tokens = None;
        session.pending = None;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn parse_tokens(&self, body: &Value) -> Option<Tokens> {
        let access = body
            .get("access_token")?
            .as_str()
            .filter(|s| !s.is_empty())?;
        let refresh = body
            .get("refresh_token")?
            .as_str()
            .filter(|s| !s.is_empty())?;
        // Refresh before the server-side expiry even if it is shorter than the usual 15 minutes.
        let mut after = self.options.refresh_after;
        if let Some(expires) = body.get("expires_in").and_then(Value::as_u64) {
            after = after.min(Duration::from_secs(expires.saturating_mul(4) / 5));
        }
        Some(Tokens {
            access: access.to_owned(),
            refresh: refresh.to_owned(),
            refresh_at: Instant::now() + after,
        })
    }

    async fn post(&self, url: &Url, body: &Value) -> Result<Reply, Failure> {
        let response = self
            .http
            .post(url.clone())
            .json(body)
            .send()
            .await
            .map_err(Failure::Transport)?;
        read_reply(response).await
    }

    async fn get(&self, url: &Url, token: &str) -> Result<Reply, AppError> {
        let response = self
            .http
            .get(url.clone())
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Failure::Transport(e).app())?;
        read_reply(response).await.map_err(|f| f.app())
    }
}

/// Reads the body in chunks and stops as soon as it exceeds the limit, so a hostile or broken
/// server cannot make the app allocate unbounded memory.
async fn read_reply(mut response: reqwest::Response) -> Result<Reply, Failure> {
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY_BYTES as u64)
    {
        return Err(Failure::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(Failure::Transport)? {
        if bytes.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(Failure::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    // Empty and non-JSON bodies (204, proxy error pages) are normal; they become `null`.
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply { status, body })
}

/// Stores a successful body in `raw`, or its German error text. Only session loss aborts.
fn fold(
    raw: &mut Map<String, Value>,
    path: &str,
    result: Result<Value, AppError>,
) -> Result<Option<Value>, AppError> {
    match result {
        Ok(body) => {
            // The summary is read from the full body; only the copy for display is capped.
            let size = serde_json::to_vec(&body).map_or(usize::MAX, |v| v.len());
            let shown = if size > MAX_RAW_BYTES {
                error_json("Antwort zu groß für die Anzeige")
            } else {
                body.clone()
            };
            raw.insert(path.to_owned(), shown);
            Ok(Some(body))
        }
        Err(
            e @ (AppError::NotLoggedIn | AppError::SessionExpired | AppError::RefreshUncertain),
        ) => Err(e),
        Err(e) => {
            raw.insert(path.to_owned(), error_json(&e.to_string()));
            Ok(None)
        }
    }
}

fn error_json(text: &str) -> Value {
    json!({ "error": text })
}

fn path_of(segments: &[&str]) -> String {
    let mut path = format!("/{}", API.join("/"));
    for s in segments {
        path.push('/');
        path.push_str(s);
    }
    path
}

fn require_filled(value: &str, reason: &'static str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::InvalidInput { reason });
    }
    Ok(())
}

/// Builds `<base>/api/app/v1/<segments>`; each segment is percent-encoded by `Url`.
fn endpoint(base: &Url, segments: &[&str]) -> Url {
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    if let Ok(mut path) = url.path_segments_mut() {
        path.pop_if_empty().extend(API).extend(segments);
    }
    url
}

/// Accepts `https://` for any host and `http://` only where the traffic cannot leave the machine or
/// the local network. Credentials in the URL are refused so they cannot end up in logs.
fn parse_base(text: &str) -> Result<Url, AppError> {
    let invalid = |reason| AppError::InvalidBaseUrl { reason };
    let url = Url::parse(text.trim())
        .map_err(|_| invalid("Das Format stimmt nicht (erwartet z. B. https://api.ampiera.de)."))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("Zugangsdaten gehören nicht in die Adresse."));
    }
    let host = url
        .host_str()
        .ok_or_else(|| invalid("Der Rechnername fehlt."))?;
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_local_host(host) => Ok(url),
        "http" => Err(invalid(
            "Unverschlüsseltes http ist nur für localhost und das lokale Netz erlaubt.",
        )),
        _ => Err(invalid("Nur http und https werden unterstützt.")),
    }
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// Short machine code of an error body, only if it cannot carry anything but a code.
fn error_code(body: &Value) -> Option<&str> {
    body.get("error")?.as_str().filter(|c| {
        !c.is_empty() && c.len() <= 40 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    })
}

fn server_error(reply: &Reply) -> AppError {
    AppError::Server {
        status: reply.status,
        code: error_code(&reply.body)
            .map(|c| format!(", Fehlercode {c}"))
            .unwrap_or_default(),
    }
}

/// Classifies a transport failure. The reqwest text is not passed on: it contains the URL.
fn network_error(error: &reqwest::Error) -> AppError {
    let mut chain = String::new();
    let mut source: Option<&dyn std::error::Error> = Some(error);
    while let Some(e) = source {
        chain.push_str(&e.to_string().to_lowercase());
        source = e.source();
    }
    let cause = if error.is_timeout() {
        "Zeitüberschreitung"
    } else if chain.contains("certificate") || chain.contains("unknownissuer") {
        "Zertifikat des Servers nicht vertrauenswürdig"
    } else if error.is_connect() {
        "Verbindung fehlgeschlagen"
    } else if error.is_redirect() {
        "unerwartete Weiterleitung"
    } else {
        "Übertragungsfehler"
    };
    AppError::Network { cause }
}
