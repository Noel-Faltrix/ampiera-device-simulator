//! Rules from CONTRACT section 4 that the core enforces regardless of the UI.

use std::net::Ipv4Addr;

use url::{Host, Url};

use crate::error::SimError;
use crate::model::{ChargePointConfig, TargetKind};

/// At most this many live boxes may be connected at the same time. Each connected box counts towards the
/// backend's per-station load and the office IP's failure counter, so a typo cannot flood production.
pub const MAX_LIVE_BOXES_CONNECTED: usize = 3;

/// Longest allowed identity; same bound as the backend (`ocppRegeln.ts`).
const MAX_IDENTITY_LEN: usize = 48;

/// True for 10/8, 172.16/12 and 192.168/16.
fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}

/// True when an unencrypted `ws://` connection to this host is acceptable: loopback names and private IPv4.
fn is_plain_ws_allowed(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback() || is_private_ipv4(*ip),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Checks scheme and host of a base URL: only `ws`/`wss`, and `ws` only towards local or private hosts.
pub fn validate_base_url(base_url: &str) -> Result<(), SimError> {
    let invalid = |reason: &str| SimError::InvalidUrl {
        url: base_url.to_string(),
        reason: reason.to_string(),
    };
    let url = Url::parse(base_url).map_err(|e| invalid(&e.to_string()))?;
    let host = url
        .host()
        .ok_or_else(|| invalid("es fehlt der Rechnername"))?;
    match url.scheme() {
        "wss" => Ok(()),
        "ws" if is_plain_ws_allowed(&host) => Ok(()),
        "ws" => Err(SimError::InsecureUrl(host.to_string())),
        other => Err(invalid(&format!(
            "das Schema „{other}“ wird nicht unterstützt (nur ws und wss)"
        ))),
    }
}

/// Identity rule of the backend: `^[A-Za-z0-9._-]{1,48}$`.
pub fn validate_identity(identity: &str) -> Result<(), SimError> {
    let valid = !identity.is_empty()
        && identity.len() <= MAX_IDENTITY_LEN
        && identity
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(SimError::InvalidIdentity(identity.to_string()))
    }
}

/// Validates a configuration for `Simulator::add`: URL, identity, live confirmation and value ranges.
pub fn validate_new_box(config: &ChargePointConfig, live_confirmed: bool) -> Result<(), SimError> {
    if config.target_kind == TargetKind::Live && !live_confirmed {
        return Err(SimError::LiveNotConfirmed);
    }
    validate_base_url(&config.base_url)?;
    validate_identity(&config.identity)?;
    validate_box_values(config)
}

/// Value ranges of a box configuration (also used when a scenario swaps the configuration).
pub fn validate_box_values(config: &ChargePointConfig) -> Result<(), SimError> {
    if !matches!(config.phases, 1 | 3) {
        return Err(SimError::InvalidConfig(
            "Die Phasenzahl der Box muss 1 oder 3 sein.".into(),
        ));
    }
    if !(config.max_power_w.is_finite() && config.max_power_w > 0.0) {
        return Err(SimError::InvalidConfig(
            "Die maximale Leistung der Box muss größer als 0 W sein.".into(),
        ));
    }
    if config.accepted_rate_units.is_empty() && !config.reject_profiles {
        return Err(SimError::InvalidConfig(
            "Die Box muss mindestens eine Einheit (W oder A) für Ladeprofile akzeptieren.".into(),
        ));
    }
    Ok(())
}

/// Validates a vehicle before plugging it in.
pub fn validate_vehicle(vehicle: &crate::model::VehicleConfig) -> Result<(), SimError> {
    if !matches!(vehicle.phases, 1 | 3) {
        return Err(SimError::InvalidConfig(
            "Die Phasenzahl des Fahrzeugs muss 1 oder 3 sein.".into(),
        ));
    }
    if !(vehicle.capacity_kwh.is_finite() && vehicle.capacity_kwh > 0.0) {
        return Err(SimError::InvalidConfig(
            "Die Batteriekapazität muss größer als 0 kWh sein.".into(),
        ));
    }
    if !(vehicle.max_power_w.is_finite() && vehicle.max_power_w > 0.0) {
        return Err(SimError::InvalidConfig(
            "Die Ladeleistung des Fahrzeugs muss größer als 0 W sein.".into(),
        ));
    }
    if !(vehicle.soc_pct.is_finite() && (0.0..=100.0).contains(&vehicle.soc_pct)) {
        return Err(SimError::InvalidConfig(
            "Der Ladestand muss zwischen 0 und 100 % liegen.".into(),
        ));
    }
    Ok(())
}

/// True when connecting one more live box would exceed [`MAX_LIVE_BOXES_CONNECTED`].
pub fn live_limit_reached(other_live_boxes_connected: usize) -> bool {
    other_live_boxes_connected >= MAX_LIVE_BOXES_CONNECTED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ws_allowed_for_loopback_and_private_ranges() {
        for url in [
            "ws://localhost:9000/ocpp",
            "ws://127.0.0.1:9000/ocpp",
            "ws://[::1]:9000/ocpp",
            "ws://10.1.2.3/ocpp",
            "ws://172.16.0.1/ocpp",
            "ws://172.31.255.255/ocpp",
            "ws://192.168.178.20:9000/ocpp",
        ] {
            assert!(validate_base_url(url).is_ok(), "{url} should be allowed");
        }
    }

    #[test]
    fn plain_ws_rejected_outside_private_ranges() {
        for url in [
            "ws://api.ampiera.de/ocpp",
            "ws://8.8.8.8/ocpp",
            "ws://172.32.0.1/ocpp",
            "ws://172.15.0.1/ocpp",
            "ws://192.169.0.1/ocpp",
            "ws://11.0.0.1/ocpp",
        ] {
            assert!(
                matches!(validate_base_url(url), Err(SimError::InsecureUrl(_))),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn wss_is_allowed_everywhere_and_other_schemes_are_not() {
        assert!(validate_base_url("wss://api.ampiera.de/ocpp").is_ok());
        assert!(matches!(
            validate_base_url("http://localhost/ocpp"),
            Err(SimError::InvalidUrl { .. })
        ));
        assert!(matches!(
            validate_base_url("not a url"),
            Err(SimError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn identity_follows_backend_rule() {
        assert!(validate_identity("AP7K2M9QX4RT").is_ok());
        assert!(validate_identity("a.b_c-d").is_ok());
        assert!(validate_identity(&"x".repeat(48)).is_ok());
        assert!(validate_identity(&"x".repeat(49)).is_err());
        assert!(validate_identity("").is_err());
        assert!(validate_identity("with space").is_err());
        assert!(validate_identity("a/b").is_err());
        assert!(validate_identity("Käse").is_err());
    }

    #[test]
    fn live_requires_confirmation_but_local_does_not() {
        let mut config = ChargePointConfig::default_local("AP1");
        assert!(validate_new_box(&config, false).is_ok());
        config.target_kind = TargetKind::Live;
        config.base_url = "wss://api.ampiera.de/ocpp".into();
        assert_eq!(
            validate_new_box(&config, false),
            Err(SimError::LiveNotConfirmed)
        );
        assert!(validate_new_box(&config, true).is_ok());
    }

    #[test]
    fn live_limit_is_three() {
        assert!(!live_limit_reached(2));
        assert!(live_limit_reached(3));
    }

    #[test]
    fn box_values_are_range_checked() {
        let mut config = ChargePointConfig::default_local("AP1");
        config.phases = 2;
        assert!(validate_box_values(&config).is_err());
        config.phases = 3;
        config.max_power_w = 0.0;
        assert!(validate_box_values(&config).is_err());
    }

    #[test]
    fn vehicle_values_are_range_checked() {
        let mut vehicle = crate::model::VehicleConfig::default();
        assert!(validate_vehicle(&vehicle).is_ok());
        vehicle.soc_pct = 101.0;
        assert!(validate_vehicle(&vehicle).is_err());
        vehicle.soc_pct = 50.0;
        vehicle.capacity_kwh = 0.0;
        assert!(validate_vehicle(&vehicle).is_err());
    }
}
