//! Answers to calls from the central system. Each rule is a small pure function; `handle_call` only routes.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use super::profiles::{ChargingProfile, ClearFilter};
use super::state::{BoxState, EndedTransaction, StopReason};
use crate::ocpp::frames::ErrorCode;
use crate::ocpp::messages::{CONNECTOR_ID, MEASURAND_SOC};

/// Configuration key for the sampled measurands.
pub const KEY_SAMPLED_DATA: &str = "MeterValuesSampledData";
/// Configuration key for the MeterValues interval.
pub const KEY_SAMPLE_INTERVAL: &str = "MeterValueSampleInterval";

/// Answer to the central system.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// CALLRESULT payload.
    Result(Value),
    /// CALLERROR.
    Error {
        /// OCPP error code.
        code: ErrorCode,
        /// Description for the log.
        description: String,
    },
}

/// Message the box sends on request (TriggerMessage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    /// BootNotification.
    BootNotification,
    /// Heartbeat.
    Heartbeat,
    /// StatusNotification.
    StatusNotification,
    /// MeterValues.
    MeterValues,
}

/// Something the actor must do after the answer went out.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send the requested message.
    Trigger(TriggerKind),
    /// Close the socket and connect again with a new BootNotification.
    Reconnect,
    /// Send StopTransaction for this transaction.
    StopTransaction {
        /// The transaction that ended.
        ended: EndedTransaction,
        /// Why it ended.
        reason: StopReason,
    },
    /// Start a transaction with this idTag.
    StartTransaction {
        /// idTag from RemoteStartTransaction.
        id_tag: String,
    },
}

/// Answer plus follow-up actions.
#[derive(Debug, Clone, PartialEq)]
pub struct Handled {
    /// What goes back to the central system.
    pub reply: Reply,
    /// What happens afterwards.
    pub effects: Vec<Effect>,
}

impl Handled {
    fn result(payload: Value) -> Self {
        Self {
            reply: Reply::Result(payload),
            effects: Vec::new(),
        }
    }

    fn status(status: &str) -> Self {
        Self::result(json!({ "status": status }))
    }

    fn with(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    fn error(code: ErrorCode, description: impl Into<String>) -> Self {
        Self {
            reply: Reply::Error {
                code,
                description: description.into(),
            },
            effects: Vec::new(),
        }
    }
}

/// Result of ChangeConfiguration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigStatus {
    /// Applied.
    Accepted,
    /// Value not acceptable.
    Rejected,
    /// Key unknown.
    NotSupported,
}

impl ConfigStatus {
    /// OCPP spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "Accepted",
            Self::Rejected => "Rejected",
            Self::NotSupported => "NotSupported",
        }
    }
}

/// Splits a comma separated list, trimming blanks.
pub fn parse_csl(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// Rule for `MeterValuesSampledData`: a box without SoC measurement must reject a list containing `SoC`
/// (the backend then retries without it, scenario S6b). An empty list is rejected because the box could not
/// report anything.
pub fn check_sampled_data(value: &str, supports_soc: bool) -> ConfigStatus {
    let items = parse_csl(value);
    if items.is_empty() || (!supports_soc && items.iter().any(|m| m == MEASURAND_SOC)) {
        ConfigStatus::Rejected
    } else {
        ConfigStatus::Accepted
    }
}

/// Parses `MeterValueSampleInterval` (seconds, 0 = off).
pub fn parse_interval(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

/// Applies a ChangeConfiguration and reports the outcome.
pub fn change_configuration(state: &mut BoxState, key: &str, value: &str) -> ConfigStatus {
    if key.eq_ignore_ascii_case(KEY_SAMPLED_DATA) {
        let status = check_sampled_data(value, state.config.supports_soc);
        if status == ConfigStatus::Accepted {
            state.sampled_data = parse_csl(value);
        }
        status
    } else if key.eq_ignore_ascii_case(KEY_SAMPLE_INTERVAL) {
        match parse_interval(value) {
            Some(seconds) => {
                state.meter_interval_s = seconds;
                ConfigStatus::Accepted
            }
            None => ConfigStatus::Rejected,
        }
    } else {
        ConfigStatus::NotSupported
    }
}

/// Decision for TriggerMessage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerDecision {
    /// Accepted; send this message afterwards.
    Accepted(TriggerKind),
    /// Connector does not exist.
    Rejected,
    /// Message type not supported by the box.
    NotImplemented,
}

/// Rule for TriggerMessage: the four message types the box can send, on connector 0 or 1.
pub fn decide_trigger(requested: &str, connector_id: Option<u32>) -> TriggerDecision {
    if connector_id.is_some_and(|c| c > CONNECTOR_ID) {
        return TriggerDecision::Rejected;
    }
    match requested {
        "BootNotification" => TriggerDecision::Accepted(TriggerKind::BootNotification),
        "Heartbeat" => TriggerDecision::Accepted(TriggerKind::Heartbeat),
        "StatusNotification" => TriggerDecision::Accepted(TriggerKind::StatusNotification),
        "MeterValues" => TriggerDecision::Accepted(TriggerKind::MeterValues),
        _ => TriggerDecision::NotImplemented,
    }
}

/// Rule for RemoteStartTransaction: only with a vehicle plugged in.
pub fn remote_start_accepted(is_plugged: bool) -> bool {
    is_plugged
}

/// Rule for RemoteStopTransaction: only for the running transaction.
pub fn remote_stop_accepted(running: Option<i64>, requested: i64) -> bool {
    running == Some(requested)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeConfigurationRequest {
    key: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TriggerMessageRequest {
    requested_message: String,
    connector_id: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetChargingProfileRequest {
    connector_id: u32,
    cs_charging_profiles: ChargingProfile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteStartRequest {
    id_tag: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteStopRequest {
    transaction_id: i64,
}

#[derive(Deserialize)]
struct ResetRequest {
    #[serde(rename = "type")]
    reset_type: String,
}

#[derive(Deserialize)]
struct GetConfigurationRequest {
    key: Option<Vec<String>>,
}

fn parse<T: serde::de::DeserializeOwned>(action: &str, payload: &Value) -> Result<T, Handled> {
    serde_json::from_value(payload.clone()).map_err(|e| {
        Handled::error(
            ErrorCode::FormationViolation,
            format!("Ungültige Nutzdaten für {action}: {e}"),
        )
    })
}

/// Routes a call from the central system to its rule and builds the answer. Unknown actions get
/// CALLERROR `NotImplemented`, as OCPP-J requires.
pub fn handle_call(
    state: &mut BoxState,
    action: &str,
    payload: &Value,
    now: DateTime<Utc>,
) -> Handled {
    match action {
        "ChangeConfiguration" => on_change_configuration(state, payload),
        "GetConfiguration" => on_get_configuration(state, payload),
        "TriggerMessage" => on_trigger_message(payload),
        "SetChargingProfile" => on_set_charging_profile(state, payload, now),
        "ClearChargingProfile" => on_clear_charging_profile(state, payload, now),
        "RemoteStartTransaction" => on_remote_start(state, payload),
        "RemoteStopTransaction" => on_remote_stop(state, payload),
        "Reset" => on_reset(payload),
        other => Handled::error(
            ErrorCode::NotImplemented,
            format!("Die Aktion {other} wird von der simulierten Box nicht unterstützt."),
        ),
    }
}

fn on_change_configuration(state: &mut BoxState, payload: &Value) -> Handled {
    match parse::<ChangeConfigurationRequest>("ChangeConfiguration", payload) {
        Ok(req) => Handled::status(change_configuration(state, &req.key, &req.value).as_str()),
        Err(error) => error,
    }
}

fn on_get_configuration(state: &BoxState, payload: &Value) -> Handled {
    let req = match parse::<GetConfigurationRequest>("GetConfiguration", payload) {
        Ok(req) => req,
        Err(error) => return error,
    };
    let entries = [
        (KEY_SAMPLED_DATA, state.sampled_data.join(",")),
        (KEY_SAMPLE_INTERVAL, state.meter_interval_s.to_string()),
    ];
    let wanted = req.key.filter(|keys| !keys.is_empty());
    let selected = |key: &str| {
        wanted
            .as_ref()
            .is_none_or(|keys| keys.iter().any(|k| k.eq_ignore_ascii_case(key)))
    };
    let known: Vec<Value> = entries
        .iter()
        .filter(|(key, _)| selected(key))
        .map(|(key, value)| json!({ "key": key, "readonly": false, "value": value }))
        .collect();
    let unknown: Vec<&String> = wanted
        .iter()
        .flatten()
        .filter(|k| !entries.iter().any(|(key, _)| key.eq_ignore_ascii_case(k)))
        .collect();
    let mut result = json!({ "configurationKey": known });
    if !unknown.is_empty() {
        result["unknownKey"] = json!(unknown);
    }
    Handled::result(result)
}

fn on_trigger_message(payload: &Value) -> Handled {
    let req = match parse::<TriggerMessageRequest>("TriggerMessage", payload) {
        Ok(req) => req,
        Err(error) => return error,
    };
    match decide_trigger(&req.requested_message, req.connector_id) {
        TriggerDecision::Accepted(kind) => Handled::status("Accepted").with(Effect::Trigger(kind)),
        TriggerDecision::Rejected => Handled::status("Rejected"),
        TriggerDecision::NotImplemented => Handled::status("NotImplemented"),
    }
}

fn on_set_charging_profile(state: &mut BoxState, payload: &Value, now: DateTime<Utc>) -> Handled {
    let req = match parse::<SetChargingProfileRequest>("SetChargingProfile", payload) {
        Ok(req) => req,
        Err(error) => return error,
    };
    match state.set_profile(req.connector_id, req.cs_charging_profiles, now) {
        Ok(()) => Handled::status("Accepted"),
        Err(rejection) => {
            state.last_error = Some(format!("Ladeprofil abgelehnt: {}", rejection.describe()));
            Handled::status("Rejected")
        }
    }
}

fn on_clear_charging_profile(state: &mut BoxState, payload: &Value, now: DateTime<Utc>) -> Handled {
    let filter = match parse::<ClearFilter>("ClearChargingProfile", payload) {
        Ok(filter) => filter,
        Err(error) => return error,
    };
    let removed = state.clear_profiles(&filter, now);
    Handled::status(if removed > 0 { "Accepted" } else { "Unknown" })
}

fn on_remote_start(state: &mut BoxState, payload: &Value) -> Handled {
    let req = match parse::<RemoteStartRequest>("RemoteStartTransaction", payload) {
        Ok(req) => req,
        Err(error) => return error,
    };
    if !remote_start_accepted(state.is_plugged()) {
        return Handled::status("Rejected");
    }
    state.prepare_new_transaction();
    if state.wants_transaction() {
        Handled::status("Accepted").with(Effect::StartTransaction { id_tag: req.id_tag })
    } else {
        Handled::status("Accepted")
    }
}

fn on_remote_stop(state: &mut BoxState, payload: &Value) -> Handled {
    let req = match parse::<RemoteStopRequest>("RemoteStopTransaction", payload) {
        Ok(req) => req,
        Err(error) => return error,
    };
    if !remote_stop_accepted(state.transaction().map(|t| t.id), req.transaction_id) {
        return Handled::status("Rejected");
    }
    match state.end_transaction() {
        Some(ended) => Handled::status("Accepted").with(Effect::StopTransaction {
            ended,
            reason: StopReason::Remote,
        }),
        None => Handled::status("Rejected"),
    }
}

fn on_reset(payload: &Value) -> Handled {
    match parse::<ResetRequest>("Reset", payload) {
        Ok(req) if matches!(req.reset_type.as_str(), "Soft" | "Hard") => {
            Handled::status("Accepted").with(Effect::Reconnect)
        }
        Ok(_) => Handled::status("Rejected"),
        Err(error) => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChargePointConfig, RateUnit, VehicleConfig};
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }

    fn new_box() -> BoxState {
        BoxState::new("b".into(), ChargePointConfig::default_local("AP1"))
    }

    fn result_status(handled: &Handled) -> String {
        match &handled.reply {
            Reply::Result(v) => v["status"].as_str().unwrap_or_default().to_string(),
            Reply::Error { .. } => "CALLERROR".into(),
        }
    }

    fn profile_payload(
        connector: u32,
        purpose: &str,
        unit: &str,
        valid_to: DateTime<Utc>,
    ) -> Value {
        json!({
            "connectorId": connector,
            "csChargingProfiles": {
                "chargingProfileId": 4711, "stackLevel": 1,
                "chargingProfilePurpose": purpose, "chargingProfileKind": "Absolute",
                "validTo": crate::ocpp::messages::format_timestamp(valid_to),
                "chargingSchedule": {
                    "startSchedule": crate::ocpp::messages::format_timestamp(t0() - Duration::seconds(60)),
                    "chargingRateUnit": unit,
                    "chargingSchedulePeriod": [{"startPeriod": 0, "limit": 10.0, "numberPhases": 3}]
                }
            }
        })
    }

    #[test]
    fn sampled_data_with_soc_is_rejected_only_without_soc_support() {
        let with_soc = "Power.Active.Import,Energy.Active.Import.Register,SoC";
        assert_eq!(check_sampled_data(with_soc, false), ConfigStatus::Rejected);
        assert_eq!(check_sampled_data(with_soc, true), ConfigStatus::Accepted);
        assert_eq!(
            check_sampled_data("Power.Active.Import,Energy.Active.Import.Register", false),
            ConfigStatus::Accepted,
            "counter-check: the retry without SoC must be accepted"
        );
        assert_eq!(check_sampled_data("", true), ConfigStatus::Rejected);
    }

    #[test]
    fn change_configuration_applies_accepted_values_only() {
        let mut b = new_box();
        b.config.supports_soc = false;
        let rejected = change_configuration(&mut b, KEY_SAMPLED_DATA, "Power.Active.Import,SoC");
        assert_eq!(rejected, ConfigStatus::Rejected);
        assert!(
            !b.sampled_data.iter().any(|m| m == "SoC"),
            "rejected value must not be applied"
        );
        assert_eq!(
            change_configuration(&mut b, KEY_SAMPLED_DATA, "Power.Active.Import"),
            ConfigStatus::Accepted
        );
        assert_eq!(b.sampled_data, vec!["Power.Active.Import"]);
        assert_eq!(
            change_configuration(&mut b, "MeterValueSampleInterval", "15"),
            ConfigStatus::Accepted
        );
        assert_eq!(b.meter_interval_s, 15);
        assert_eq!(
            change_configuration(&mut b, KEY_SAMPLE_INTERVAL, "abc"),
            ConfigStatus::Rejected
        );
        assert_eq!(
            change_configuration(&mut b, KEY_SAMPLE_INTERVAL, "-5"),
            ConfigStatus::Rejected
        );
        assert_eq!(
            b.meter_interval_s, 15,
            "invalid value keeps the old interval"
        );
        assert_eq!(
            change_configuration(&mut b, "WebSocketPingInterval", "30"),
            ConfigStatus::NotSupported
        );
    }

    #[test]
    fn trigger_message_rule_covers_each_branch() {
        assert_eq!(
            decide_trigger("StatusNotification", Some(1)),
            TriggerDecision::Accepted(TriggerKind::StatusNotification)
        );
        assert_eq!(
            decide_trigger("MeterValues", None),
            TriggerDecision::Accepted(TriggerKind::MeterValues)
        );
        assert_eq!(
            decide_trigger("MeterValues", Some(0)),
            TriggerDecision::Accepted(TriggerKind::MeterValues)
        );
        assert_eq!(
            decide_trigger("MeterValues", Some(2)),
            TriggerDecision::Rejected
        );
        assert_eq!(
            decide_trigger("FirmwareStatusNotification", None),
            TriggerDecision::NotImplemented
        );
    }

    #[test]
    fn trigger_message_answers_accepted_and_asks_for_the_message() {
        let mut b = new_box();
        let h = handle_call(
            &mut b,
            "TriggerMessage",
            &json!({"requestedMessage": "StatusNotification"}),
            t0(),
        );
        assert_eq!(result_status(&h), "Accepted");
        assert_eq!(
            h.effects,
            vec![Effect::Trigger(TriggerKind::StatusNotification)]
        );
    }

    #[test]
    fn set_charging_profile_accepts_and_applies_a_valid_profile() {
        let mut b = new_box();
        let h = handle_call(
            &mut b,
            "SetChargingProfile",
            &profile_payload(0, "TxDefaultProfile", "A", t0() + Duration::seconds(900)),
            t0(),
        );
        assert_eq!(result_status(&h), "Accepted");
        assert_eq!(b.store.len(), 1);
        assert_eq!(b.active_limit().unwrap().limit_w, 10.0 * 230.0 * 3.0);
    }

    #[test]
    fn set_charging_profile_is_rejected_for_reject_flag_and_unit() {
        let payload = profile_payload(0, "TxDefaultProfile", "A", t0() + Duration::seconds(900));
        let mut rejecting = new_box();
        rejecting.config.reject_profiles = true;
        let h = handle_call(&mut rejecting, "SetChargingProfile", &payload, t0());
        assert_eq!(result_status(&h), "Rejected");
        assert_eq!(rejecting.store.len(), 0);
        assert!(
            rejecting.active_limit().is_none(),
            "no limit applied after a rejection"
        );
        assert!(rejecting
            .last_error
            .as_deref()
            .unwrap()
            .contains("abgelehnt"));

        let mut watts_only = new_box();
        watts_only.config.accepted_rate_units = vec![RateUnit::W];
        assert_eq!(
            result_status(&handle_call(
                &mut watts_only,
                "SetChargingProfile",
                &payload,
                t0()
            )),
            "Rejected"
        );
        let in_watts = profile_payload(0, "TxDefaultProfile", "W", t0() + Duration::seconds(900));
        assert_eq!(
            result_status(&handle_call(
                &mut watts_only,
                "SetChargingProfile",
                &in_watts,
                t0()
            )),
            "Accepted"
        );
    }

    #[test]
    fn tx_profile_needs_a_running_transaction() {
        let payload = profile_payload(1, "TxProfile", "A", t0() + Duration::seconds(900));
        let mut b = new_box();
        assert_eq!(
            result_status(&handle_call(&mut b, "SetChargingProfile", &payload, t0())),
            "Rejected"
        );
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        b.begin_transaction(9, "SIMULATOR", t0());
        assert_eq!(
            result_status(&handle_call(&mut b, "SetChargingProfile", &payload, t0())),
            "Accepted"
        );
    }

    #[test]
    fn malformed_profile_payload_is_a_formation_violation() {
        let mut b = new_box();
        let h = handle_call(
            &mut b,
            "SetChargingProfile",
            &json!({"connectorId": 0}),
            t0(),
        );
        assert!(matches!(
            h.reply,
            Reply::Error {
                code: ErrorCode::FormationViolation,
                ..
            }
        ));
    }

    #[test]
    fn clear_charging_profile_answers_accepted_or_unknown() {
        let mut b = new_box();
        let set = profile_payload(0, "TxDefaultProfile", "W", t0() + Duration::seconds(900));
        handle_call(&mut b, "SetChargingProfile", &set, t0());
        assert_eq!(
            result_status(&handle_call(
                &mut b,
                "ClearChargingProfile",
                &json!({"id": 9999}),
                t0()
            )),
            "Unknown"
        );
        assert_eq!(b.store.len(), 1, "unknown id must not clear anything");
        assert_eq!(
            result_status(&handle_call(
                &mut b,
                "ClearChargingProfile",
                &json!({"id": 4711}),
                t0()
            )),
            "Accepted"
        );
        assert_eq!(b.store.len(), 0);
        assert!(b.active_limit().is_none());
        assert_eq!(
            result_status(&handle_call(
                &mut b,
                "ClearChargingProfile",
                &json!({"id": 4711}),
                t0()
            )),
            "Unknown"
        );
    }

    #[test]
    fn remote_start_requires_a_plugged_vehicle() {
        let mut b = new_box();
        let payload = json!({"connectorId": 1, "idTag": "AMPIERA"});
        assert_eq!(
            result_status(&handle_call(
                &mut b,
                "RemoteStartTransaction",
                &payload,
                t0()
            )),
            "Rejected"
        );
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        let h = handle_call(&mut b, "RemoteStartTransaction", &payload, t0());
        assert_eq!(result_status(&h), "Accepted");
        assert_eq!(
            h.effects,
            vec![Effect::StartTransaction {
                id_tag: "AMPIERA".into()
            }]
        );
        b.begin_transaction(3, "AMPIERA", t0());
        let again = handle_call(&mut b, "RemoteStartTransaction", &payload, t0());
        assert_eq!(result_status(&again), "Accepted");
        assert!(
            again.effects.is_empty(),
            "a running transaction is not started twice"
        );
    }

    #[test]
    fn remote_stop_only_for_the_running_transaction() {
        let mut b = new_box();
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        b.begin_transaction(3, "SIMULATOR", t0());
        assert_eq!(
            result_status(&handle_call(
                &mut b,
                "RemoteStopTransaction",
                &json!({"transactionId": 4}),
                t0()
            )),
            "Rejected"
        );
        assert!(b.transaction().is_some());
        let h = handle_call(
            &mut b,
            "RemoteStopTransaction",
            &json!({"transactionId": 3}),
            t0(),
        );
        assert_eq!(result_status(&h), "Accepted");
        assert!(matches!(
            h.effects.as_slice(),
            [Effect::StopTransaction {
                reason: StopReason::Remote,
                ..
            }]
        ));
        assert!(b.transaction().is_none());
        assert!(remote_stop_accepted(Some(1), 1));
        assert!(
            !remote_stop_accepted(None, 0),
            "no transaction means nothing to stop, even for id 0"
        );
    }

    #[test]
    fn reset_is_accepted_and_reconnects() {
        let mut b = new_box();
        let h = handle_call(&mut b, "Reset", &json!({"type": "Soft"}), t0());
        assert_eq!(result_status(&h), "Accepted");
        assert_eq!(h.effects, vec![Effect::Reconnect]);
        assert_eq!(
            result_status(&handle_call(
                &mut b,
                "Reset",
                &json!({"type": "Warm"}),
                t0()
            )),
            "Rejected"
        );
    }

    #[test]
    fn get_configuration_returns_the_two_meter_keys() {
        let mut b = new_box();
        let h = handle_call(&mut b, "GetConfiguration", &json!({}), t0());
        let Reply::Result(v) = h.reply else {
            panic!("expected result")
        };
        let keys: Vec<&str> = v["configurationKey"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, vec![KEY_SAMPLED_DATA, KEY_SAMPLE_INTERVAL]);
        assert!(v.get("unknownKey").is_none());

        let h = handle_call(
            &mut b,
            "GetConfiguration",
            &json!({"key": ["MeterValueSampleInterval", "Nope"]}),
            t0(),
        );
        let Reply::Result(v) = h.reply else {
            panic!("expected result")
        };
        assert_eq!(v["configurationKey"].as_array().unwrap().len(), 1);
        assert_eq!(v["unknownKey"], json!(["Nope"]));
    }

    #[test]
    fn unknown_actions_get_a_not_implemented_callerror() {
        let mut b = new_box();
        for action in ["DataTransfer", "UnlockConnector", "UpdateFirmware"] {
            let h = handle_call(&mut b, action, &json!({}), t0());
            assert!(
                matches!(
                    h.reply,
                    Reply::Error {
                        code: ErrorCode::NotImplemented,
                        ..
                    }
                ),
                "{action}"
            );
        }
    }
}
