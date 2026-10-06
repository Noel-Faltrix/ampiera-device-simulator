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

/// At most this many boxes can exist at once; keeps a typo or a restored config file from spawning hundreds of tasks.
pub const MAX_BOXES: usize = 20;

/// Longest label shown in the UI.
const MAX_LABEL_CHARS: usize = 60;

/// OCPP CiString20Type limit for vendor and model.
const MAX_VENDOR_MODEL_CHARS: usize = 20;

/// Longest accepted password; far above any generated one, small enough to bound the Basic header.
const MAX_PASSWORD_CHARS: usize = 200;

/// Upper bound for box and vehicle power in W; the largest DC chargers are below this.
const MAX_POWER_W: f64 = 350_000.0;

/// Largest battery in kWh the model accepts (heavy trucks and buses).
const MAX_CAPACITY_KWH: f64 = 300.0;

/// Largest simulated clock error: one day.
pub const MAX_CLOCK_OFFSET_S: i64 = 86_400;

/// True for 10/8, 172.16/12 and 192.168/16.
fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}

/// True when the host is a loopback name or address or a private IPv4 address.
fn is_local_host(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback() || is_private_ipv4(*ip),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Checks scheme and host of a base URL: only `ws`/`wss`, and `ws` only towards local or private hosts.
/// Userinfo, query and fragment are refused: credentials in a URL would end up in logs, and the identity is
/// appended to the path.
pub fn validate_base_url(base_url: &str) -> Result<(), SimError> {
    let invalid = |reason: &str| SimError::InvalidUrl {
        url: base_url.to_string(),
        reason: reason.to_string(),
    };
    let url = Url::parse(base_url).map_err(|e| invalid(&e.to_string()))?;
    let host = url
        .host()
        .ok_or_else(|| invalid("es fehlt der Rechnername"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("Zugangsdaten gehören nicht in die Adresse"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("die Adresse darf weder ? noch # enthalten"));
    }
    match url.scheme() {
        "wss" => Ok(()),
        "ws" if is_local_host(&host) => Ok(()),
        "ws" => Err(SimError::InsecureUrl(host.to_string())),
        other => Err(invalid(&format!(
            "das Schema „{other}“ wird nicht unterstützt (nur ws und wss)"
        ))),
    }
}

/// The target a box really talks to, derived from its URL host and the configured kind. A host outside
/// localhost, loopback and private IPv4 is always `Live`; a `Live` configuration stays `Live` even towards a
/// local host (stricter, never looser). Unparsable URLs count as `Live`.
///
/// This is the single place that decides: live confirmation, the live connection limit and `liveAllowed`
/// of scenarios all use it.
pub fn effective_kind(config: &ChargePointConfig) -> TargetKind {
    let host_is_local = Url::parse(&config.base_url)
        .ok()
        .and_then(|url| url.host().map(|h| is_local_host(&h)))
        .unwrap_or(false);
    if config.target_kind == TargetKind::Live || !host_is_local {
        TargetKind::Live
    } else {
        TargetKind::Local
    }
}

/// Key under which two boxes count as "the same": normalized base URL (scheme and host lower case, no default
/// port, no trailing slash) plus identity. Two such boxes would push each other out of the central system.
pub fn endpoint_key(config: &ChargePointConfig) -> String {
    let base = Url::parse(&config.base_url)
        .map(|u| u.as_str().trim_end_matches('/').to_string())
        .unwrap_or_else(|_| config.base_url.trim_end_matches('/').to_lowercase());
    format!("{base}/{}", config.identity)
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

/// Password rule: 1 to 200 printable ASCII characters.
pub fn validate_password(password: &str) -> Result<(), SimError> {
    let printable = password.chars().all(|c| (' '..='~').contains(&c));
    if (1..=MAX_PASSWORD_CHARS).contains(&password.chars().count()) && printable {
        Ok(())
    } else {
        Err(SimError::InvalidPassword)
    }
}

/// Validates a configuration for `Simulator::add`: URL, identity, target consistency, live confirmation and
/// value ranges.
pub fn validate_new_box(config: &ChargePointConfig, live_confirmed: bool) -> Result<(), SimError> {
    validate_base_url(&config.base_url)?;
    validate_identity(&config.identity)?;
    validate_box_values(config)?;
    let effective = effective_kind(config);
    if config.target_kind == TargetKind::Local && effective == TargetKind::Live {
        let host = Url::parse(&config.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        return Err(SimError::PublicHostForLocalTarget(host));
    }
    if effective == TargetKind::Live && !live_confirmed {
        return Err(SimError::LiveNotConfirmed);
    }
    Ok(())
}

fn text_ok(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.chars().count()) && !text.chars().any(char::is_control)
}

fn invalid(text: &str) -> SimError {
    SimError::InvalidConfig(text.to_string())
}

/// Value ranges of a box configuration (also used when a scenario swaps the configuration).
pub fn validate_box_values(config: &ChargePointConfig) -> Result<(), SimError> {
    if !text_ok(&config.label, 1, MAX_LABEL_CHARS) {
        return Err(invalid("Die Bezeichnung muss 1 bis 60 Zeichen lang sein und darf keine Steuerzeichen enthalten."));
    }
    for (name, value) in [("Hersteller", &config.vendor), ("Modell", &config.model)] {
        if !text_ok(value, 1, MAX_VENDOR_MODEL_CHARS) {
            return Err(SimError::InvalidConfig(format!(
                "{name} muss 1 bis 20 Zeichen lang sein und darf keine Steuerzeichen enthalten."
            )));
        }
    }
    if !matches!(config.phases, 1 | 3) {
        return Err(invalid("Die Phasenzahl der Wallbox muss 1 oder 3 sein."));
    }
    if !(config.max_power_w.is_finite() && (1.0..=MAX_POWER_W).contains(&config.max_power_w)) {
        return Err(invalid(
            "Die maximale Leistung der Wallbox muss zwischen 1 W und 350000 W liegen.",
        ));
    }
    // `abs()` would overflow for i64::MIN; a range check cannot.
    if !(-MAX_CLOCK_OFFSET_S..=MAX_CLOCK_OFFSET_S).contains(&config.clock_offset_s) {
        return Err(invalid(
            "Die Uhrabweichung darf höchstens einen Tag (86400 s) betragen.",
        ));
    }
    if config.accepted_rate_units.is_empty() && !config.reject_profiles {
        return Err(invalid(
            "Die Wallbox muss mindestens eine Einheit (W oder A) für Ladeprofile akzeptieren.",
        ));
    }
    Ok(())
}

/// Validates a vehicle before plugging it in.
pub fn validate_vehicle(vehicle: &crate::model::VehicleConfig) -> Result<(), SimError> {
    if !matches!(vehicle.phases, 1 | 3) {
        return Err(invalid("Die Phasenzahl des Fahrzeugs muss 1 oder 3 sein."));
    }
    if !(vehicle.capacity_kwh.is_finite()
        && (1.0..=MAX_CAPACITY_KWH).contains(&vehicle.capacity_kwh))
    {
        return Err(invalid(
            "Die Batteriekapazität muss zwischen 1 und 300 kWh liegen.",
        ));
    }
    if !(vehicle.max_power_w.is_finite() && (1.0..=MAX_POWER_W).contains(&vehicle.max_power_w)) {
        return Err(invalid(
            "Die Ladeleistung des Fahrzeugs muss zwischen 1 W und 350000 W liegen.",
        ));
    }
    if !(vehicle.soc_pct.is_finite() && (0.0..=100.0).contains(&vehicle.soc_pct)) {
        return Err(invalid("Der Ladestand muss zwischen 0 und 100 % liegen."));
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

    #[test]
    fn effective_kind_comes_from_the_host() {
        let mut config = ChargePointConfig::default_local("AP1");
        assert_eq!(effective_kind(&config), TargetKind::Local);
        config.base_url = "wss://api.ampiera.de/ocpp".into();
        assert_eq!(
            effective_kind(&config),
            TargetKind::Live,
            "public host is live whatever the UI says"
        );
        config.base_url = "ws://127.0.0.1:9000/ocpp".into();
        config.target_kind = TargetKind::Live;
        assert_eq!(
            effective_kind(&config),
            TargetKind::Live,
            "a live configuration is never loosened"
        );
        config.base_url = "not a url".into();
        config.target_kind = TargetKind::Local;
        assert_eq!(
            effective_kind(&config),
            TargetKind::Live,
            "unparsable counts as live"
        );
    }

    #[test]
    fn local_target_with_a_public_host_is_rejected() {
        let mut config = ChargePointConfig::default_local("AP1");
        config.base_url = "wss://api.ampiera.de/ocpp".into();
        assert!(matches!(
            validate_new_box(&config, true),
            Err(SimError::PublicHostForLocalTarget(host)) if host == "api.ampiera.de"
        ));
        config.target_kind = TargetKind::Live;
        assert_eq!(
            validate_new_box(&config, false),
            Err(SimError::LiveNotConfirmed)
        );
        assert!(validate_new_box(&config, true).is_ok());
    }

    #[test]
    fn urls_with_userinfo_query_or_fragment_are_refused() {
        for url in [
            "ws://user:pw@localhost:9000/ocpp",
            "ws://user@localhost:9000/ocpp",
            "ws://localhost:9000/ocpp?x=1",
            "ws://localhost:9000/ocpp#frag",
        ] {
            assert!(
                matches!(validate_base_url(url), Err(SimError::InvalidUrl { .. })),
                "{url}"
            );
        }
    }

    #[test]
    fn endpoint_key_normalizes_case_slash_and_default_port() {
        let mut a = ChargePointConfig::default_local("AP1");
        let mut b = a.clone();
        a.base_url = "WSS://Api.Ampiera.de:443/ocpp/".into();
        b.base_url = "wss://api.ampiera.de/ocpp".into();
        assert_eq!(endpoint_key(&a), endpoint_key(&b));
        b.identity = "AP2".into();
        assert_ne!(
            endpoint_key(&a),
            endpoint_key(&b),
            "counter-check: another identity is another box"
        );
    }

    #[test]
    fn password_must_be_printable_ascii_of_bounded_length() {
        assert!(validate_password("a").is_ok());
        assert!(validate_password(&"x".repeat(200)).is_ok());
        assert!(validate_password("").is_err());
        assert!(validate_password(&"x".repeat(201)).is_err());
        assert!(validate_password("tab\there").is_err());
        assert!(validate_password("Passwört").is_err());
        assert!(validate_password("mit Leerzeichen ~!").is_ok());
    }

    #[test]
    fn text_and_number_bounds_are_enforced() {
        let base = ChargePointConfig::default_local("AP1");
        let mut c = base.clone();
        c.label = "x".repeat(61);
        assert!(validate_box_values(&c).is_err());
        c.label = "ok\u{7}".into();
        assert!(validate_box_values(&c).is_err(), "control character");
        let mut c = base.clone();
        c.vendor = "v".repeat(21);
        assert!(validate_box_values(&c).is_err());
        let mut c = base.clone();
        c.model = String::new();
        assert!(validate_box_values(&c).is_err());
        let mut c = base.clone();
        c.max_power_w = 350_001.0;
        assert!(validate_box_values(&c).is_err());
        c.max_power_w = 350_000.0;
        assert!(validate_box_values(&c).is_ok());
        let mut c = base.clone();
        c.clock_offset_s = 86_401;
        assert!(validate_box_values(&c).is_err());
        c.clock_offset_s = -86_400;
        assert!(validate_box_values(&c).is_ok());
        for extreme in [i64::MIN, i64::MAX] {
            c.clock_offset_s = extreme;
            assert!(validate_box_values(&c).is_err(), "{extreme}");
        }
        let mut vehicle = crate::model::VehicleConfig {
            capacity_kwh: 301.0,
            ..Default::default()
        };
        assert!(validate_vehicle(&vehicle).is_err());
        vehicle.capacity_kwh = 0.5;
        assert!(validate_vehicle(&vehicle).is_err());
        vehicle.capacity_kwh = 60.0;
        vehicle.max_power_w = 350_001.0;
        assert!(validate_vehicle(&vehicle).is_err());
    }
}
