//! Payload builders for the calls the box sends, and typed readers for the answers it gets.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::model::{ChargePointConfig, OcppStatus};

/// OCPP 1.6 limits `chargePointVendor` and `chargePointModel` to 20 characters (CiString20Type).
pub const MAX_VENDOR_MODEL_LEN: usize = 20;

/// The only connector of the simulated box.
pub const CONNECTOR_ID: u32 = 1;

/// Measurand names the box can report.
pub const MEASURAND_POWER: &str = "Power.Active.Import";
/// Energy register measurand.
pub const MEASURAND_ENERGY: &str = "Energy.Active.Import.Register";
/// State of charge measurand.
pub const MEASURAND_SOC: &str = "SoC";

/// OCPP timestamp format: ISO 8601 UTC with milliseconds.
pub fn format_timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// BootNotification request.
pub fn boot_notification(config: &ChargePointConfig) -> Value {
    json!({
        "chargePointVendor": truncate_chars(&config.vendor, MAX_VENDOR_MODEL_LEN),
        "chargePointModel": truncate_chars(&config.model, MAX_VENDOR_MODEL_LEN),
    })
}

/// StatusNotification request for the connector.
pub fn status_notification(status: OcppStatus, box_time: DateTime<Utc>) -> Value {
    json!({
        "connectorId": CONNECTOR_ID,
        "errorCode": "NoError",
        "status": status.as_str(),
        "timestamp": format_timestamp(box_time),
    })
}

/// StartTransaction request.
pub fn start_transaction(meter_start_wh: i64, id_tag: &str, box_time: DateTime<Utc>) -> Value {
    json!({
        "connectorId": CONNECTOR_ID,
        "idTag": id_tag,
        "meterStart": meter_start_wh,
        "timestamp": format_timestamp(box_time),
    })
}

/// StopTransaction request.
pub fn stop_transaction(
    transaction_id: i64,
    meter_stop_wh: i64,
    id_tag: &str,
    reason: &str,
    box_time: DateTime<Utc>,
) -> Value {
    json!({
        "transactionId": transaction_id,
        "meterStop": meter_stop_wh,
        "idTag": id_tag,
        "reason": reason,
        "timestamp": format_timestamp(box_time),
    })
}

/// Values a MeterValues message can carry; `None` means "not measurable", the sample is then left out.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterSample {
    /// Active power in W.
    pub power_w: Option<f64>,
    /// Energy register in Wh.
    pub energy_wh: i64,
    /// State of charge in percent.
    pub soc_pct: Option<f64>,
}

fn sampled_value(measurand: &str, value: String, unit: &str, location: &str) -> Value {
    json!({
        "value": value,
        "context": "Sample.Periodic",
        "measurand": measurand,
        "location": location,
        "unit": unit,
    })
}

/// MeterValues request. Only measurands listed in `sampled_data` are included, and only when a value exists.
pub fn meter_values(
    transaction_id: Option<i64>,
    box_time: DateTime<Utc>,
    sample: &MeterSample,
    sampled_data: &[String],
) -> Value {
    let wants = |name: &str| sampled_data.iter().any(|m| m == name);
    let mut values = Vec::new();
    if let (true, Some(power)) = (wants(MEASURAND_POWER), sample.power_w) {
        values.push(sampled_value(
            MEASURAND_POWER,
            format!("{power:.1}"),
            "W",
            "Outlet",
        ));
    }
    if wants(MEASURAND_ENERGY) {
        values.push(sampled_value(
            MEASURAND_ENERGY,
            sample.energy_wh.to_string(),
            "Wh",
            "Outlet",
        ));
    }
    if let (true, Some(soc)) = (wants(MEASURAND_SOC), sample.soc_pct) {
        values.push(sampled_value(
            MEASURAND_SOC,
            format!("{soc:.1}"),
            "Percent",
            "EV",
        ));
    }
    let mut payload = json!({
        "connectorId": CONNECTOR_ID,
        "meterValue": [{ "timestamp": format_timestamp(box_time), "sampledValue": values }],
    });
    if let Some(id) = transaction_id {
        payload["transactionId"] = json!(id);
    }
    payload
}

/// BootNotification response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootResponse {
    /// `Accepted`, `Pending` or `Rejected`.
    pub status: String,
    /// Heartbeat interval in seconds.
    pub interval: Option<u32>,
}

/// `idTagInfo` of StartTransaction and StopTransaction responses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IdTagInfo {
    /// `Accepted`, `Blocked`, ...
    pub status: String,
}

/// StartTransaction response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTransactionResponse {
    /// Transaction id; 0 means the central system stores nothing but the box keeps charging.
    pub transaction_id: i64,
    /// Authorization result.
    pub id_tag_info: IdTagInfo,
}

/// Reads a typed response; the error is a German text naming the action.
pub fn read_response<T: serde::de::DeserializeOwned>(
    action: &str,
    payload: &Value,
) -> Result<T, String> {
    serde_json::from_value(payload.clone())
        .map_err(|e| format!("Die Antwort auf {action} hat ein unerwartetes Format ({e})."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }

    #[test]
    fn boot_truncates_vendor_and_model_to_twenty_characters() {
        let mut config = ChargePointConfig::default_local("AP1");
        config.vendor = "V".repeat(30);
        config.model = "ä".repeat(25);
        let p = boot_notification(&config);
        assert_eq!(p["chargePointVendor"].as_str().unwrap().chars().count(), 20);
        assert_eq!(
            p["chargePointModel"].as_str().unwrap().chars().count(),
            20,
            "counted in characters, not bytes"
        );
        config.vendor = "Ampiera".into();
        assert_eq!(
            boot_notification(&config)["chargePointVendor"],
            "Ampiera",
            "short values stay untouched"
        );
    }

    #[test]
    fn timestamps_use_milliseconds_and_z() {
        assert_eq!(format_timestamp(at()), "2026-10-06T12:00:00.000Z");
    }

    #[test]
    fn meter_values_contain_only_configured_and_measurable_samples() {
        let sample = MeterSample {
            power_w: Some(3680.04),
            energy_wh: 1234,
            soc_pct: Some(55.55),
        };
        let all: Vec<String> = [MEASURAND_POWER, MEASURAND_ENERGY, MEASURAND_SOC]
            .map(String::from)
            .to_vec();
        let p = meter_values(Some(7), at(), &sample, &all);
        let values = p["meterValue"][0]["sampledValue"].as_array().unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0]["value"], "3680.0");
        assert_eq!(values[0]["unit"], "W");
        assert_eq!(values[1]["value"], "1234");
        assert_eq!(values[2]["unit"], "Percent");
        assert_eq!(p["transactionId"], 7);

        let no_soc = meter_values(None, at(), &sample, &all[..2]);
        assert_eq!(
            no_soc["meterValue"][0]["sampledValue"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(no_soc.get("transactionId").is_none());

        let unknown_power = MeterSample {
            power_w: None,
            energy_wh: 5,
            soc_pct: None,
        };
        let p = meter_values(Some(1), at(), &unknown_power, &all);
        let values = p["meterValue"][0]["sampledValue"].as_array().unwrap();
        assert_eq!(
            values.len(),
            1,
            "missing power is left out, never sent as 0"
        );
        assert_eq!(values[0]["measurand"], MEASURAND_ENERGY);
    }

    #[test]
    fn start_and_stop_payloads_have_required_fields() {
        let s = start_transaction(10, "SIMULATOR", at());
        assert_eq!(s["connectorId"], 1);
        assert_eq!(s["meterStart"], 10);
        let e = stop_transaction(0, 10, "SIMULATOR", "Local", at());
        assert_eq!(e["transactionId"], 0);
        assert_eq!(e["meterStop"], 10);
        assert_eq!(e["reason"], "Local");
    }

    #[test]
    fn responses_are_read_with_german_errors() {
        let boot: BootResponse = read_response(
            "BootNotification",
            &json!({"status": "Accepted", "interval": 300, "currentTime": "x"}),
        )
        .unwrap();
        assert_eq!(boot.interval, Some(300));
        let err =
            read_response::<StartTransactionResponse>("StartTransaction", &json!({})).unwrap_err();
        assert!(err.contains("StartTransaction"));
    }
}
