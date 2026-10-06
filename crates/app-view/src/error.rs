//! Error type of the app view client. Every message is German, names the cause and never contains
//! passwords, tokens, invite tokens or device codes.

use thiserror::Error;

/// Failure of an app view operation. `Display` is the text shown to the user.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AppError {
    /// No tokens in memory (never logged in, logged out or session ended).
    #[error("Du bist nicht angemeldet. Melde dich zuerst in der App-Sicht an.")]
    NotLoggedIn,
    /// The base URL cannot be used; `reason` is a fixed text, never the URL itself.
    #[error("Die Server-Adresse ist ungültig: {reason}")]
    InvalidBaseUrl {
        /// Fixed explanation without the URL.
        reason: &'static str,
    },
    /// A locally checked input is unusable; `reason` is a fixed text, never the value itself.
    #[error("Eingabe ungültig: {reason}")]
    InvalidInput {
        /// Fixed explanation without the value.
        reason: &'static str,
    },
    /// The server could not be reached or did not answer in time.
    #[error("Server nicht erreichbar ({cause}). Prüfe Verbindung und Adresse.")]
    Network {
        /// Fixed classification such as "Zeitüberschreitung".
        cause: &'static str,
    },
    /// Login rejected; the server answers every failure the same way.
    #[error("Anmeldung abgelehnt (Server meldet ungültige Zugangsdaten).")]
    InvalidCredentials,
    /// HTTP 429 from the server.
    #[error("Zu viele Versuche. Warte einige Minuten und versuche es dann erneut.")]
    TooManyRequests,
    /// HTTP 503 `code_nicht_zustellbar`.
    #[error("Der Bestätigungscode für dieses Gerät konnte nicht per E-Mail zugestellt werden.")]
    CodeNotDeliverable,
    /// `verify_device` called without a preceding login that asked for a code.
    #[error("Es wartet kein Gerätecode. Melde dich zuerst an.")]
    NoPendingDeviceCode,
    /// The device confirmation session is unknown or expired on the server.
    #[error("Die Geräte-Bestätigung ist ungültig oder abgelaufen. Melde dich erneut an.")]
    DeviceTokenInvalid,
    /// Wrong six-digit code; the user may try again.
    #[error("Der Code ist falsch. Gib den Code aus der E-Mail erneut ein.")]
    CodeWrong,
    /// The code is no longer valid.
    #[error("Der Code ist abgelaufen. Melde dich erneut an, um einen neuen Code zu bekommen.")]
    CodeExpired,
    /// Too many wrong codes for this confirmation.
    #[error("Zu viele falsche Codes. Melde dich erneut an, um einen neuen Code zu bekommen.")]
    CodeTooManyAttempts,
    /// The invite token is unknown, used or expired.
    #[error("Einladung abgelehnt (Server meldet ungültige oder abgelaufene Einladung).")]
    InviteInvalid,
    /// Password shorter than the server minimum.
    #[error("Das Passwort ist zu kurz (mindestens 15 Zeichen).")]
    PasswordTooShort,
    /// Password longer than the server maximum.
    #[error("Das Passwort ist zu lang (höchstens 128 Zeichen).")]
    PasswordTooLong,
    /// Password found in a leak database.
    #[error("Das Passwort ist in bekannten Datenlecks aufgetaucht. Wähle ein anderes Passwort.")]
    PasswordLeaked,
    /// Refresh or access token rejected; tokens were discarded.
    #[error("Die Sitzung ist abgelaufen oder wurde beendet. Melde dich erneut an.")]
    SessionExpired,
    /// The refresh request may have used up the refresh token without a usable answer.
    #[error("Die Sitzung konnte nicht erneuert werden und wurde zur Sicherheit beendet. Melde dich erneut an.")]
    RefreshUncertain,
    /// Unexpected HTTP status; `code` is the server's short error code when it looks harmless.
    #[error("Der Server antwortete unerwartet (HTTP {status}{code}).")]
    Server {
        /// HTTP status.
        status: u16,
        /// Empty, or `", Fehlercode xyz"`.
        code: String,
    },
    /// The answer exceeds the size limit for a single response.
    #[error("Antwort des Servers zu groß.")]
    ResponseTooLarge,
    /// The HTTP client could not be built (TLS initialisation failed).
    #[error("Der HTTP-Client konnte nicht aufgebaut werden (TLS-Initialisierung fehlgeschlagen).")]
    ClientInit,
    /// The answer was not the expected JSON.
    #[error("Die Antwort des Servers ist unlesbar oder unvollständig ({what}).")]
    InvalidResponse {
        /// Fixed description of what was missing.
        what: &'static str,
    },
}

impl AppError {
    /// True when the login is gone and the user has to sign in again.
    pub fn is_session_loss(&self) -> bool {
        matches!(
            self,
            AppError::NotLoggedIn | AppError::SessionExpired | AppError::RefreshUncertain
        )
    }
}
