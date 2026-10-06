//! Websocket client for OCPP 1.6J: URL, Basic auth, TLS, error classification and reconnect backoff.
//!
//! Certificate validation is always on: the TLS configuration uses the bundled webpki roots and there is no
//! switch to turn verification off.

use std::fmt;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};
use url::Url;

/// An open websocket to the central system.
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// OCPP subprotocol offered in the handshake; the backend rejects offers that do not contain it.
pub const OCPP_SUBPROTOCOL: &str = "ocpp1.6";

/// Longest part of an HTTP error body kept for diagnostics.
const MAX_BODY_CHARS: usize = 200;

/// A password that never shows up in `Debug` output or logs.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value; call only where the secret is actually needed (building the auth header).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Why a connection attempt failed and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectFailure {
    /// True when retrying with backoff makes sense; false for 401/404/429 and invalid URLs.
    pub retryable: bool,
    /// German explanation, safe to show (never contains the password).
    pub reason: String,
    /// HTTP status of the handshake response, if there was one.
    pub http_status: Option<u16>,
    /// Start of the response body, if there was one.
    pub body: Option<String>,
}

/// Builds `<baseUrl>/<identity>`.
pub fn endpoint_url(base_url: &str, identity: &str) -> Result<Url, String> {
    let joined = format!("{}/{}", base_url.trim_end_matches('/'), identity);
    Url::parse(&joined).map_err(|e| format!("Die Adresse „{joined}“ ist ungültig ({e})."))
}

/// `Authorization` header value for HTTP Basic auth. Built in memory, never logged or stored.
pub fn basic_auth_value(identity: &str, password: &str) -> String {
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("{identity}:{password}"));
    format!("Basic {encoded}")
}

/// Delay before reconnect attempt number `attempt` (1-based): `base`, 2x, 4x ... capped at `max`.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let exponent = attempt.saturating_sub(1).min(20);
    base.saturating_mul(1u32 << exponent).min(max)
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring supports the default protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

fn failure(retryable: bool, reason: impl Into<String>) -> ConnectFailure {
    ConnectFailure {
        retryable,
        reason: reason.into(),
        http_status: None,
        body: None,
    }
}

/// Maps a handshake error to a decision. 401, 404 and 429 are final: retrying a wrong password would fill the
/// backend's failure counter (20 per IP in 15 minutes) and lock out everyone behind the same address.
pub fn classify_error(error: &WsError) -> ConnectFailure {
    match error {
        WsError::Http(response) => {
            let status = response.status().as_u16();
            let body = response.body().as_ref().map(|b| {
                String::from_utf8_lossy(b)
                    .chars()
                    .take(MAX_BODY_CHARS)
                    .collect::<String>()
            });
            let (retryable, reason) = match status {
                401 => (
                    false,
                    "Anmeldung abgelehnt (HTTP 401): Kennung oder Passwort stimmt nicht. Es wird nicht automatisch \
                     erneut versucht, damit die Zentrale die Adresse nicht sperrt."
                        .to_string(),
                ),
                404 => (
                    false,
                    "Die Zentrale kennt diesen Pfad nicht (HTTP 404): Basisadresse und Kennung prüfen.".to_string(),
                ),
                429 => (
                    false,
                    "Die Zentrale hat zu viele Fehlversuche von dieser Adresse gezählt (HTTP 429). Einige Minuten \
                     warten, dann erneut verbinden."
                        .to_string(),
                ),
                other => (true, format!("Die Zentrale hat den Verbindungsaufbau mit HTTP {other} abgelehnt.")),
            };
            ConnectFailure {
                retryable,
                reason,
                http_status: Some(status),
                body,
            }
        }
        WsError::Url(e) => failure(
            false,
            format!("Die Adresse der Zentrale ist ungültig ({e})."),
        ),
        WsError::Tls(e) => failure(true, format!("TLS-Fehler beim Verbindungsaufbau: {e}")),
        WsError::Io(e) => {
            let text = e.to_string();
            if text.to_lowercase().contains("certificate") || text.contains("invalid peer") {
                failure(
                    true,
                    format!("Das Zertifikat der Zentrale wurde nicht akzeptiert ({text}). Die Prüfung ist immer aktiv."),
                )
            } else {
                failure(true, format!("Verbindung nicht möglich: {text}"))
            }
        }
        WsError::Protocol(e) => failure(
            true,
            format!(
                "Die Zentrale hat das Unterprotokoll {OCPP_SUBPROTOCOL} nicht bestätigt ({e})."
            ),
        ),
        other => failure(true, format!("Verbindung fehlgeschlagen: {other}")),
    }
}

/// Opens the websocket with Basic auth and the `ocpp1.6` subprotocol.
pub async fn connect(
    base_url: &str,
    identity: &str,
    password: &Secret,
    timeout: Duration,
) -> Result<WsStream, ConnectFailure> {
    let url = endpoint_url(base_url, identity).map_err(|reason| failure(false, reason))?;
    let mut request = url.as_str().into_client_request().map_err(|e| {
        failure(
            false,
            format!("Die Adresse der Zentrale ist ungültig ({e})."),
        )
    })?;
    let mut auth =
        HeaderValue::from_str(&basic_auth_value(identity, password.expose())).map_err(|_| {
            failure(
                false,
                "Kennung oder Passwort enthalten Zeichen, die nicht übertragen werden können.",
            )
        })?;
    auth.set_sensitive(true);
    request.headers_mut().insert(AUTHORIZATION, auth);
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(OCPP_SUBPROTOCOL),
    );
    let connector = Connector::Rustls(tls_config());
    let attempt = connect_async_tls_with_config(request, None, false, Some(connector));
    match tokio::time::timeout(timeout, attempt).await {
        Ok(Ok((stream, _response))) => Ok(stream),
        Ok(Err(error)) => Err(classify_error(&error)),
        Err(_) => Err(failure(
            true,
            format!(
                "Zeitüberschreitung: die Zentrale hat nicht innerhalb von {} s geantwortet.",
                timeout.as_secs()
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::http::Response;

    fn http_error(status: u16, body: &str) -> WsError {
        let response = Response::builder()
            .status(status)
            .body(Some(body.as_bytes().to_vec()))
            .unwrap();
        WsError::Http(Box::new(response))
    }

    #[test]
    fn url_is_base_plus_identity_without_double_slash() {
        assert_eq!(
            endpoint_url("ws://localhost:9000/ocpp", "AP1")
                .unwrap()
                .as_str(),
            "ws://localhost:9000/ocpp/AP1"
        );
        assert_eq!(
            endpoint_url("wss://api.ampiera.de/ocpp/", "AP1")
                .unwrap()
                .as_str(),
            "wss://api.ampiera.de/ocpp/AP1"
        );
        assert!(endpoint_url("::bad", "AP1").is_err());
    }

    #[test]
    fn basic_auth_matches_the_rfc_example_shape() {
        assert_eq!(basic_auth_value("user", "pass"), "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn secret_never_prints() {
        let secret = Secret::new("hunter2");
        assert!(!format!("{secret:?}").contains("hunter2"));
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn backoff_doubles_and_caps_at_sixty_seconds() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(60);
        let seconds: Vec<u64> = (1..=9)
            .map(|n| backoff_delay(n, base, max).as_secs())
            .collect();
        assert_eq!(seconds, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(
            backoff_delay(500, base, max),
            max,
            "no overflow for huge attempt numbers"
        );
    }

    #[test]
    fn auth_and_path_errors_are_final_and_other_errors_retry() {
        for status in [401u16, 404, 429] {
            let f = classify_error(&http_error(status, "x"));
            assert!(!f.retryable, "{status} must not be retried");
            assert_eq!(f.http_status, Some(status));
            assert!(f.reason.contains(&status.to_string()));
        }
        for status in [400u16, 500, 502, 503] {
            let f = classify_error(&http_error(status, ""));
            assert!(f.retryable, "{status} should be retried with backoff");
        }
    }

    #[test]
    fn io_errors_retry_and_certificate_errors_say_so() {
        let io = WsError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        let f = classify_error(&io);
        assert!(f.retryable);
        assert!(f.reason.contains("refused"));
        let cert = WsError::Io(std::io::Error::other(
            "invalid peer certificate: UnknownIssuer",
        ));
        let f = classify_error(&cert);
        assert!(f.reason.contains("Zertifikat"));
    }

    #[tokio::test]
    async fn tls_connection_to_a_peer_that_does_not_speak_tls_fails_cleanly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            // Accept and drop: the TLS handshake can never succeed, and certificate checking stays on.
            while let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        let result = connect(
            &format!("wss://127.0.0.1:{port}/ocpp"),
            "AP1",
            &Secret::new("pw"),
            Duration::from_secs(5),
        )
        .await;
        let failure = result.expect_err("handshake must fail");
        assert!(failure.retryable);
        assert!(!failure.reason.contains("pw\""));
    }

    #[test]
    fn failure_text_never_contains_the_password() {
        let f = classify_error(&http_error(401, "no hint"));
        assert!(!f.reason.to_lowercase().contains("hunter2"));
        assert_eq!(f.body.as_deref(), Some("no hint"));
    }
}
