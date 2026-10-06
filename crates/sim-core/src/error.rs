//! Errors returned by the public API. Messages are German, name the cause and never contain secrets.

use crate::model::ScenarioId;

/// Everything that can go wrong when driving the simulator.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SimError {
    /// No box with this id.
    #[error("Die Box mit der ID {0} gibt es nicht.")]
    UnknownBox(String),
    /// Live target without explicit confirmation.
    #[error("Für das Live-System muss die Verbindung ausdrücklich bestätigt werden.")]
    LiveNotConfirmed,
    /// Too many live boxes connected at once.
    #[error("Es dürfen höchstens {max} Live-Boxen gleichzeitig verbunden sein.")]
    TooManyLiveBoxes {
        /// The configured maximum.
        max: usize,
    },
    /// `ws://` towards a host outside the allowed ranges.
    #[error("Unverschlüsseltes ws:// ist nur für localhost, 127.0.0.1, ::1 und private IPv4-Adressen erlaubt, nicht für „{0}“. Bitte wss:// verwenden.")]
    InsecureUrl(String),
    /// The base URL cannot be used.
    #[error("Die Adresse „{url}“ ist ungültig: {reason}")]
    InvalidUrl {
        /// The offending URL.
        url: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The OCPP identity does not match the backend's rule.
    #[error("Die Kennung „{0}“ ist ungültig: erlaubt sind 1 bis 48 Zeichen aus Buchstaben, Ziffern, Punkt, Unterstrich und Bindestrich.")]
    InvalidIdentity(String),
    /// A configuration value is out of range.
    #[error("Ungültige Einstellung: {0}")]
    InvalidConfig(String),
    /// A vehicle is already plugged in.
    #[error("Es ist bereits ein Fahrzeug angesteckt.")]
    VehicleAlreadyPlugged,
    /// No vehicle plugged in.
    #[error("Es ist kein Fahrzeug angesteckt.")]
    NoVehicle,
    /// Operation needs an open connection.
    #[error("Die Box ist nicht mit der Zentrale verbunden.")]
    NotConnected,
    /// Scenario is not allowed on a live box.
    #[error(
        "Das Szenario {0:?} darf nicht gegen das Live-System laufen und wurde nicht gestartet."
    )]
    ScenarioNotAllowedOnLive(ScenarioId),
    /// Another scenario already runs on this box.
    #[error("Auf dieser Box läuft bereits ein Szenario.")]
    ScenarioAlreadyRunning,
    /// Abort requested without a running scenario.
    #[error("Auf dieser Box läuft kein Szenario.")]
    NoScenarioRunning,
    /// The box's task has ended.
    #[error("Die Box wurde beendet und kann keine Befehle mehr annehmen.")]
    BoxStopped,
    /// The frame log could not be serialized.
    #[error("Das Protokoll konnte nicht exportiert werden: {0}")]
    LogExport(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_german_with_umlauts() {
        let text = SimError::TooManyLiveBoxes { max: 3 }.to_string();
        assert!(text.contains("höchstens 3"));
        assert!(SimError::BoxStopped
            .to_string()
            .contains("kann keine Befehle"));
        assert!(SimError::InsecureUrl("example.com".into())
            .to_string()
            .contains("„example.com“"));
    }
}
