//! Pure decision rules the scenarios use to judge what they observed.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::charge_point::profiles::{effective_limit, ChargingProfile};
use crate::charge_point::vehicle::charging_power_w;
use crate::model::{CheckOutcome, CheckResult, ReportOutcome, VehicleConfig};

/// Tolerance for "the box regulates to the limit" (SCENARIOS S3).
pub const LIMIT_TOLERANCE_PCT: f64 = 1.0;

/// A power reading below this counts as "zero" when the expectation is zero. The meter resolution of real
/// wallboxes is 1 W, so a tighter bound would flag rounding.
const ZERO_POWER_TOLERANCE_W: f64 = 1.0;

/// A call received from the central system, as the scenarios record it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedCall {
    /// OCPP action.
    pub action: String,
    /// Payload.
    pub payload: Value,
}

/// True when `actual` is within `tolerance_pct` percent of `expected` (absolute floor of 1 W).
/// A missing reading never matches.
pub fn power_matches(actual: Option<f64>, expected: f64, tolerance_pct: f64) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    let allowed = (expected.abs() * tolerance_pct / 100.0).max(ZERO_POWER_TOLERANCE_W);
    (actual - expected).abs() <= allowed
}

/// Power the box should deliver under `limit_w` for this vehicle and state of charge.
pub fn expected_limited_power(
    limit_w: f64,
    box_max_w: f64,
    vehicle: &VehicleConfig,
    soc_pct: f64,
) -> f64 {
    charging_power_w(box_max_w, vehicle.max_power_w, Some(limit_w), soc_pct)
}

/// Power the box should deliver at `now` given the profiles it accepted: the limit in force, or its own maximum
/// when no profile applies (e.g. after `validTo`). Computed independently of the box's own evaluation.
pub fn expected_power_now(
    accepted: &[ChargingProfile],
    now: DateTime<Utc>,
    box_phases: u8,
    box_max_w: f64,
    vehicle: &VehicleConfig,
    soc_pct: f64,
) -> f64 {
    match effective_limit(accepted.iter(), now, box_phases) {
        Some(limit) => expected_limited_power(limit.limit_w, box_max_w, vehicle, soc_pct),
        None => charging_power_w(box_max_w, vehicle.max_power_w, None, soc_pct),
    }
}

/// Checks that `validTo` exists and lies at most `max` after `received_at` (the backend caps test limits at
/// 15 minutes). Returns the remaining validity on success.
pub fn valid_to_within(
    valid_to: Option<DateTime<Utc>>,
    received_at: DateTime<Utc>,
    max: Duration,
) -> Result<Duration, String> {
    let Some(valid_to) = valid_to else {
        return Err(
            "Das Profil hat kein validTo; die Box wüsste nie, wann ihre Pflicht endet.".to_string(),
        );
    };
    let remaining = valid_to - received_at;
    if remaining > max {
        return Err(format!(
            "validTo liegt {} Minuten in der Zukunft, erlaubt sind höchstens {}.",
            remaining.num_minutes(),
            max.num_minutes()
        ));
    }
    if remaining <= Duration::zero() {
        return Err("validTo liegt schon in der Vergangenheit.".to_string());
    }
    Ok(remaining)
}

/// Reads `SetChargingProfile` payloads: connector id and profile.
pub fn parse_set_profile(payload: &Value) -> Option<(u32, ChargingProfile)> {
    let connector = u32::try_from(payload.get("connectorId")?.as_u64()?).ok()?;
    let profile = serde_json::from_value(payload.get("csChargingProfiles")?.clone()).ok()?;
    Some((connector, profile))
}

/// The `status` field of a response payload.
pub fn response_status(payload: &Value) -> Option<&str> {
    payload.get("status").and_then(Value::as_str)
}

/// Value of one measurand in a MeterValues payload.
pub fn sampled_value(payload: &Value, measurand: &str) -> Option<f64> {
    payload
        .get("meterValue")?
        .as_array()?
        .iter()
        .flat_map(|mv| {
            mv.get("sampledValue")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find(|sv| sv.get("measurand").and_then(Value::as_str) == Some(measurand))
        .and_then(|sv| sv.get("value")?.as_str()?.parse().ok())
}

/// Timestamp of the first meter value in a MeterValues payload.
pub fn meter_timestamp(payload: &Value) -> Option<DateTime<Utc>> {
    let text = payload
        .get("meterValue")?
        .as_array()?
        .first()?
        .get("timestamp")?
        .as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// True when `box_time` deviates from `now` by `expected_offset_s` within `tolerance_s`.
pub fn clock_offset_matches(
    box_time: DateTime<Utc>,
    now: DateTime<Utc>,
    expected_offset_s: i64,
    tolerance_s: i64,
) -> bool {
    let actual = (box_time - now).num_seconds();
    (actual - expected_offset_s).abs() <= tolerance_s
}

/// Predicate on the `value` of a ChangeConfiguration call.
type ValueCheck = fn(&str) -> bool;

fn config_key_value(call: &ReceivedCall) -> Option<(&str, &str)> {
    if call.action != "ChangeConfiguration" {
        return None;
    }
    Some((
        call.payload.get("key")?.as_str()?,
        call.payload.get("value")?.as_str()?,
    ))
}

/// Checks the calls after BootNotification against CENTRAL_SYSTEM_BEHAVIOUR.md: ChangeConfiguration with
/// SoC, then (only if the box rejected it) the same without SoC, then the interval, then TriggerMessage for
/// StatusNotification. Returns a German description of what arrived, or what is wrong.
pub fn check_post_boot_sequence(
    calls: &[ReceivedCall],
    soc_rejected: bool,
) -> Result<String, String> {
    let mut expected: Vec<(&str, ValueCheck)> = vec![(
        "ChangeConfiguration MeterValuesSampledData (mit SoC)",
        |v| v.split(',').any(|m| m.trim() == "SoC"),
    )];
    if soc_rejected {
        expected.push((
            "ChangeConfiguration MeterValuesSampledData (ohne SoC)",
            |v| !v.split(',').any(|m| m.trim() == "SoC"),
        ));
    }
    let mut position = 0;
    for (label, value_ok) in &expected {
        let call = calls
            .get(position)
            .ok_or_else(|| format!("Es fehlt: {label}."))?;
        let good = config_key_value(call)
            .is_some_and(|(key, value)| key == "MeterValuesSampledData" && value_ok(value));
        if !good {
            return Err(format!(
                "An Stelle {} wurde {} erwartet, es kam {}.",
                position + 1,
                label,
                call.action
            ));
        }
        position += 1;
    }
    let interval = calls
        .get(position)
        .ok_or("Es fehlt: ChangeConfiguration MeterValueSampleInterval.")?;
    if !config_key_value(interval).is_some_and(|(key, _)| key == "MeterValueSampleInterval") {
        return Err(format!(
            "An Stelle {} wurde ChangeConfiguration MeterValueSampleInterval erwartet, es kam {}.",
            position + 1,
            interval.action
        ));
    }
    position += 1;
    let trigger = calls
        .get(position)
        .ok_or("Es fehlt: TriggerMessage StatusNotification.")?;
    let is_status_trigger = trigger.action == "TriggerMessage"
        && trigger
            .payload
            .get("requestedMessage")
            .and_then(Value::as_str)
            == Some("StatusNotification");
    if !is_status_trigger {
        return Err(format!(
            "An Stelle {} wurde TriggerMessage StatusNotification erwartet, es kam {}.",
            position + 1,
            trigger.action
        ));
    }
    Ok(format!(
        "{} Aufrufe in der erwarteten Reihenfolge angekommen.",
        position + 1
    ))
}

/// Overall outcome: aborted wins, then any failed check, else passed. Skipped checks do not fail a run.
pub fn outcome_of(checks: &[CheckResult], aborted: bool) -> ReportOutcome {
    if aborted {
        ReportOutcome::Aborted
    } else if checks.iter().any(|c| c.outcome == CheckOutcome::Failed) {
        ReportOutcome::Failed
    } else {
        ReportOutcome::Passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }

    fn change(key: &str, value: &str) -> ReceivedCall {
        ReceivedCall {
            action: "ChangeConfiguration".into(),
            payload: json!({"key": key, "value": value}),
        }
    }

    fn trigger() -> ReceivedCall {
        ReceivedCall {
            action: "TriggerMessage".into(),
            payload: json!({"requestedMessage": "StatusNotification"}),
        }
    }

    #[test]
    fn power_tolerance_is_one_percent_with_a_one_watt_floor() {
        assert!(power_matches(Some(3996.0), 4000.0, 1.0));
        assert!(power_matches(Some(4040.0), 4000.0, 1.0));
        assert!(!power_matches(Some(4041.0), 4000.0, 1.0));
        assert!(!power_matches(Some(3900.0), 4000.0, 1.0));
        assert!(power_matches(Some(0.0), 0.0, 1.0));
        assert!(!power_matches(Some(50.0), 0.0, 1.0));
        assert!(
            !power_matches(None, 0.0, 1.0),
            "unknown power never matches"
        );
    }

    #[test]
    fn expected_power_respects_vehicle_and_box() {
        let v = VehicleConfig::default();
        assert_eq!(expected_limited_power(4000.0, 11_000.0, &v, 20.0), 4000.0);
        assert_eq!(
            expected_limited_power(20_000.0, 11_000.0, &v, 20.0),
            11_000.0
        );
    }

    #[test]
    fn expected_power_follows_the_profile_until_valid_to_then_the_box_maximum() {
        let payload = json!({"connectorId": 0, "csChargingProfiles": {
            "chargingProfileId": 1, "stackLevel": 0, "chargingProfilePurpose": "TxDefaultProfile",
            "chargingProfileKind": "Absolute", "validTo": "2026-10-06T12:10:00Z",
            "chargingSchedule": {"startSchedule": "2026-10-06T11:59:00Z", "chargingRateUnit": "W",
              "chargingSchedulePeriod": [{"startPeriod": 0, "limit": 4000.0}]}}});
        let profile = parse_set_profile(&payload).unwrap().1;
        let v = VehicleConfig::default();
        let during =
            expected_power_now(std::slice::from_ref(&profile), t0(), 3, 11_000.0, &v, 20.0);
        let after = expected_power_now(
            std::slice::from_ref(&profile),
            t0() + Duration::minutes(11),
            3,
            11_000.0,
            &v,
            20.0,
        );
        assert_eq!(during, 4000.0);
        assert_eq!(
            after, 11_000.0,
            "counter-check: nothing limits the box after validTo"
        );
    }

    #[test]
    fn valid_to_must_exist_and_be_at_most_fifteen_minutes_ahead() {
        let max = Duration::minutes(15);
        assert_eq!(
            valid_to_within(Some(t0() + Duration::minutes(15)), t0(), max),
            Ok(Duration::minutes(15))
        );
        assert!(valid_to_within(Some(t0() + Duration::minutes(16)), t0(), max).is_err());
        assert!(valid_to_within(None, t0(), max).is_err());
        assert!(valid_to_within(Some(t0() - Duration::seconds(1)), t0(), max).is_err());
    }

    #[test]
    fn set_profile_payload_is_parsed() {
        let payload = json!({"connectorId": 0, "csChargingProfiles": {
            "chargingProfileId": 1, "stackLevel": 0, "chargingProfilePurpose": "TxDefaultProfile",
            "chargingProfileKind": "Absolute",
            "chargingSchedule": {"chargingRateUnit": "W", "chargingSchedulePeriod": [{"startPeriod": 0, "limit": 1.0}]}}});
        let (connector, profile) = parse_set_profile(&payload).unwrap();
        assert_eq!(connector, 0);
        assert_eq!(profile.charging_profile_id, 1);
        assert!(parse_set_profile(&json!({"connectorId": 0})).is_none());
    }

    #[test]
    fn meter_value_readers_find_measurands_and_timestamp() {
        let payload = json!({"connectorId": 1, "meterValue": [{"timestamp": "2026-10-06T11:45:00.000Z",
            "sampledValue": [{"measurand": "Power.Active.Import", "value": "3680.0"},
                             {"measurand": "Energy.Active.Import.Register", "value": "12"}]}]});
        assert_eq!(sampled_value(&payload, "Power.Active.Import"), Some(3680.0));
        assert_eq!(sampled_value(&payload, "SoC"), None);
        assert_eq!(
            meter_timestamp(&payload),
            Some(t0() - Duration::minutes(15))
        );
    }

    #[test]
    fn clock_offset_rule_has_a_counter_case() {
        assert!(clock_offset_matches(
            t0() - Duration::seconds(900),
            t0(),
            -900,
            30
        ));
        assert!(
            !clock_offset_matches(t0(), t0(), -900, 30),
            "an unshifted clock must not pass"
        );
    }

    #[test]
    fn post_boot_sequence_with_soc_support() {
        let calls = [
            change(
                "MeterValuesSampledData",
                "Power.Active.Import,Energy.Active.Import.Register,SoC",
            ),
            change("MeterValueSampleInterval", "60"),
            trigger(),
        ];
        assert!(check_post_boot_sequence(&calls, false).is_ok());
    }

    #[test]
    fn post_boot_sequence_without_soc_expects_the_retry() {
        let calls = [
            change(
                "MeterValuesSampledData",
                "Power.Active.Import,Energy.Active.Import.Register,SoC",
            ),
            change(
                "MeterValuesSampledData",
                "Power.Active.Import,Energy.Active.Import.Register",
            ),
            change("MeterValueSampleInterval", "60"),
            trigger(),
        ];
        assert!(check_post_boot_sequence(&calls, true).is_ok());
        let without_retry = [calls[0].clone(), calls[2].clone(), calls[3].clone()];
        assert!(
            check_post_boot_sequence(&without_retry, true).is_err(),
            "retry missing"
        );
        let retry_with_soc = [
            calls[0].clone(),
            calls[0].clone(),
            calls[2].clone(),
            calls[3].clone(),
        ];
        assert!(
            check_post_boot_sequence(&retry_with_soc, true).is_err(),
            "retry must not contain SoC"
        );
    }

    #[test]
    fn post_boot_sequence_detects_wrong_order_and_missing_calls() {
        let wrong = [
            change("MeterValueSampleInterval", "60"),
            change("MeterValuesSampledData", "Power.Active.Import,SoC"),
            trigger(),
        ];
        assert!(check_post_boot_sequence(&wrong, false).is_err());
        assert!(check_post_boot_sequence(&[], false).is_err());
        let partial = [
            change("MeterValuesSampledData", "SoC"),
            change("MeterValueSampleInterval", "60"),
        ];
        assert!(check_post_boot_sequence(&partial, false)
            .unwrap_err()
            .contains("TriggerMessage"));
    }

    #[test]
    fn outcome_priority_is_aborted_then_failed_then_passed() {
        let check = |outcome| CheckResult {
            name: "x".into(),
            outcome,
            detail: String::new(),
        };
        let passed = [check(CheckOutcome::Passed), check(CheckOutcome::Skipped)];
        let failed = [check(CheckOutcome::Passed), check(CheckOutcome::Failed)];
        assert_eq!(outcome_of(&passed, false), ReportOutcome::Passed);
        assert_eq!(outcome_of(&failed, false), ReportOutcome::Failed);
        assert_eq!(outcome_of(&failed, true), ReportOutcome::Aborted);
    }
}
