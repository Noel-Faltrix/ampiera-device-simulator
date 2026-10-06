//! Errors returned by the public API. Messages are German, name the cause and never contain secrets.

use crate::model::ScenarioId;

/// Everything that can go wrong when driving the simulator.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SimError {
    /// No box with this id.
    #[error("Die Wallbox mit der ID {0} gibt es nicht.")]
    UnknownBox(String),
    /// Live target without explicit confirmation.
    #[error("Für den Produktivserver musst du die Verbindung ausdrücklich bestätigen.")]
    LiveNotConfirmed,
    /// Too many live boxes connected at once.
    #[error(
        "Es dürfen höchstens {max} Wallboxen gleichzeitig mit dem Produktivserver verbunden sein."
    )]
    TooManyLiveBoxes {
        /// The configured maximum.
        max: usize,
    },
    /// Too many boxes in total.
    #[error("Es sind höchstens {max} Wallboxen gleichzeitig möglich.")]
    TooManyBoxes {
        /// The configured maximum.
        max: usize,
    },
    /// `ws://` towards a host outside the allowed ranges.
    #[error("Unverschlüsseltes ws:// ist nur für localhost, 127.0.0.1, ::1 und private IPv4-Adressen erlaubt, nicht für „{0}“. Verwende wss://.")]
    InsecureUrl(String),
    /// A box marked local points at a host that counts as Produktivserver.
    #[error("Das Ziel „{0}“ liegt nicht im lokalen Netz und zählt als Produktivserver. Stelle das Ziel auf „live“ und bestätige die Verbindung, wenn du dich damit verbinden willst.")]
    PublicHostForLocalTarget(String),
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
    /// The password has the wrong length or characters (the value itself is never shown).
    #[error("Das Passwort muss 1 bis 200 druckbare ASCII-Zeichen lang sein.")]
    InvalidPassword,
    /// A configuration value is out of range.
    #[error("Ungültige Einstellung: {0}")]
    InvalidConfig(String),
    /// Another box already uses the same address and identity.
    #[error("Es gibt schon eine Wallbox mit dieser Adresse und Kennung. Zwei davon würden sich gegenseitig aus der Zentrale werfen.")]
    DuplicateBox,
    /// Connecting is paused after a rejected login.
    #[error("Die Zentrale hat die Anmeldung abgelehnt (HTTP 401 oder 429). Die Wallbox wartet noch {remaining_s} s, bevor sie sich wieder verbindet, damit die Zentrale die Adresse nicht sperrt.")]
    ConnectCooldown {
        /// Seconds until the next attempt is allowed.
        remaining_s: u64,
    },
    /// A vehicle is already plugged in.
    #[error("Es ist schon ein Fahrzeug angesteckt.")]
    VehicleAlreadyPlugged,
    /// No vehicle plugged in.
    #[error("Es ist kein Fahrzeug angesteckt.")]
    NoVehicle,
    /// Operation needs an open connection.
    #[error("Die Wallbox ist nicht mit der Zentrale verbunden.")]
    NotConnected,
    /// Scenario is not allowed on a live box.
    #[error(
        "Dieses Szenario darf nicht gegen den Produktivserver laufen und wurde nicht ausgeführt."
    )]
    ScenarioNotAllowedOnLive(ScenarioId),
    /// Another scenario already runs on this box.
    #[error("Auf dieser Wallbox läuft schon ein Szenario.")]
    ScenarioAlreadyRunning,
    /// Abort requested without a running scenario.
    #[error("Auf dieser Wallbox läuft kein Szenario.")]
    NoScenarioRunning,
    /// The box's task has ended.
    #[error("Die Wallbox wurde beendet und nimmt keine Befehle mehr an.")]
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
            .contains("nimmt keine Befehle"));
        assert!(SimError::InsecureUrl("example.com".into())
            .to_string()
            .contains("„example.com“"));
    }

    #[test]
    fn user_text_says_wallbox_and_produktivserver() {
        let all = [
            SimError::NotConnected.to_string(),
            SimError::LiveNotConfirmed.to_string(),
            SimError::ScenarioNotAllowedOnLive(ScenarioId::S10).to_string(),
            SimError::ConnectCooldown { remaining_s: 42 }.to_string(),
        ];
        for text in &all {
            assert!(!text.contains("Live-System"), "{text}");
            assert!(!text.split_whitespace().any(|w| w == "Box"), "{text}");
        }
        assert!(all[3].contains("42 s"));
    }
}
