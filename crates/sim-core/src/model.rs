//! Serde types shared with the desktop shell and the UI (CONTRACT section 1).
//!
//! All structs serialize with `camelCase` field names; enums use the spelling given in the contract.
//! A missing measurement is `None` (JSON `null`), never `0`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default base URL of a local backend (CONTRACT section 1).
pub const DEFAULT_LOCAL_BASE_URL: &str = "ws://localhost:9000/ocpp";
/// Default base URL of the live backend (CONTRACT section 1).
pub const DEFAULT_LIVE_BASE_URL: &str = "wss://api.ampiera.de/ocpp";

/// Which backend a simulated box talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// A backend on the developer machine or in the office network.
    Local,
    /// The production backend; guarded by explicit confirmation and a connection limit.
    Live,
}

/// Unit of a charging schedule limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateUnit {
    /// Watts.
    W,
    /// Amperes per phase.
    A,
}

/// Static configuration of a simulated charge point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargePointConfig {
    /// Label shown in the UI, e.g. "Box 1".
    pub label: String,
    /// Backend the box connects to.
    pub target_kind: TargetKind,
    /// Base URL without trailing slash; the identity is appended.
    pub base_url: String,
    /// OCPP identity ("Kennung"), e.g. "AP7K2M9QX4RT".
    pub identity: String,
    /// BootNotification `chargePointVendor`, truncated to 20 characters when sent.
    pub vendor: String,
    /// BootNotification `chargePointModel`, truncated to 20 characters when sent.
    pub model: String,
    /// Number of phases of the box, 1 or 3.
    pub phases: u8,
    /// Hardware limit of the simulated box in W.
    pub max_power_w: f64,
    /// When false the box rejects `MeterValuesSampledData` containing SoC.
    pub supports_soc: bool,
    /// Profile units the box accepts; others are answered `Rejected`.
    pub accepted_rate_units: Vec<RateUnit>,
    /// Answer every SetChargingProfile with `Rejected`.
    pub reject_profiles: bool,
    /// Simulated clock error of the box in seconds (applied to outgoing timestamps).
    pub clock_offset_s: i64,
}

impl ChargePointConfig {
    /// A three-phase 11 kW box pointing at the local backend; a sensible starting point for the UI.
    pub fn default_local(identity: &str) -> Self {
        Self {
            label: "Box 1".to_string(),
            target_kind: TargetKind::Local,
            base_url: DEFAULT_LOCAL_BASE_URL.to_string(),
            identity: identity.to_string(),
            vendor: "Ampiera".to_string(),
            model: "Simulator".to_string(),
            phases: 3,
            max_power_w: 11_000.0,
            supports_soc: true,
            accepted_rate_units: vec![RateUnit::W, RateUnit::A],
            reject_profiles: false,
            clock_offset_s: 0,
        }
    }
}

/// Electric vehicle plugged into a box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VehicleConfig {
    /// Usable battery capacity in kWh.
    pub capacity_kwh: f64,
    /// State of charge when plugged in, 0..100.
    pub soc_pct: f64,
    /// On-board charger limit in W.
    pub max_power_w: f64,
    /// Number of phases the vehicle charges on, 1 or 3.
    pub phases: u8,
}

impl Default for VehicleConfig {
    /// A mid-size car at 20 % that can take the full power of an 11 kW box.
    fn default() -> Self {
        Self {
            capacity_kwh: 60.0,
            soc_pct: 20.0,
            max_power_w: 11_000.0,
            phases: 3,
        }
    }
}

/// State of the websocket connection to the central system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ConnectionState {
    /// No connection and none wanted.
    Disconnected,
    /// Handshake in progress.
    Connecting,
    /// Socket open.
    Connected {
        /// When the socket was opened.
        since: DateTime<Utc>,
    },
    /// Connection lost, waiting for the next attempt.
    Reconnecting {
        /// Number of the upcoming attempt, starting at 1.
        attempt: u32,
        /// When the next attempt starts.
        next_attempt_at: DateTime<Utc>,
    },
    /// Not retried automatically (e.g. HTTP 401).
    Failed {
        /// German explanation of the cause.
        reason: String,
    },
}

/// OCPP 1.6 connector status; serialized exactly as the OCPP strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OcppStatus {
    /// Nothing plugged in.
    Available,
    /// Vehicle plugged in, transaction not started yet.
    Preparing,
    /// Energy is flowing.
    Charging,
    /// Vehicle does not draw energy (battery full).
    SuspendedEV,
    /// Box does not offer energy (limit 0).
    SuspendedEVSE,
    /// Transaction ended, vehicle still plugged in or unplug pending.
    Finishing,
    /// Reserved for someone.
    Reserved,
    /// Out of operation.
    Unavailable,
    /// Error state.
    Faulted,
}

impl OcppStatus {
    /// The OCPP string of this status, as used in StatusNotification.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Preparing => "Preparing",
            Self::Charging => "Charging",
            Self::SuspendedEV => "SuspendedEV",
            Self::SuspendedEVSE => "SuspendedEVSE",
            Self::Finishing => "Finishing",
            Self::Reserved => "Reserved",
            Self::Unavailable => "Unavailable",
            Self::Faulted => "Faulted",
        }
    }
}

/// OCPP `chargingProfilePurpose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProfilePurpose {
    /// Caps the whole charge point.
    ChargePointMaxProfile,
    /// Default for every new transaction.
    TxDefaultProfile,
    /// Applies to one running transaction.
    TxProfile,
}

/// The limit currently in force for the box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveLimit {
    /// Effective limit in W (amperes converted with 230 V times the number of phases).
    pub limit_w: f64,
    /// Limit as received.
    pub raw_limit: f64,
    /// Unit of `raw_limit`.
    pub rate_unit: RateUnit,
    /// Id of the profile that sets the limit.
    pub profile_id: i64,
    /// Purpose of that profile.
    pub purpose: ProfilePurpose,
    /// End of the profile's validity, if any.
    pub valid_to: Option<DateTime<Utc>>,
}

/// Plugged-in vehicle as shown in a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VehicleSnapshot {
    /// Configuration the vehicle was plugged in with.
    pub config: VehicleConfig,
    /// Current state of charge, 0..100.
    pub soc_pct: f64,
    /// Always true while a vehicle is present; kept for the contract shape.
    pub plugged: bool,
}

/// Complete state of one charge point for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargePointSnapshot {
    /// Local id (uuid), not the OCPP identity.
    pub id: String,
    /// Configuration of the box.
    pub config: ChargePointConfig,
    /// Connection to the central system.
    pub connection: ConnectionState,
    /// Connector status.
    pub status: OcppStatus,
    /// Plugged-in vehicle, `None` when nothing is plugged in.
    pub vehicle: Option<VehicleSnapshot>,
    /// Current power in W; `None` when no vehicle is plugged in (nothing to measure).
    pub power_w: Option<f64>,
    /// Meter register `Energy.Active.Import.Register` in Wh.
    pub energy_wh: f64,
    /// Running transaction, if any.
    pub transaction_id: Option<i64>,
    /// Limit in force; `None` means the box charges at its own maximum.
    pub active_limit: Option<ActiveLimit>,
    /// Number of stored charging profiles.
    pub profile_count: usize,
    /// Heartbeat interval from the BootNotification response.
    pub heartbeat_interval_s: Option<u32>,
    /// Last error worth showing, German.
    pub last_error: Option<String>,
}

/// Direction of a logged frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameDirection {
    /// Box to central system.
    Out,
    /// Central system to box.
    In,
}

/// One OCPP-J frame in the log; never contains credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameLogEntry {
    /// Local id of the box.
    pub charge_point_id: String,
    /// When the frame was sent or received.
    pub at: DateTime<Utc>,
    /// Direction relative to the box.
    pub direction: FrameDirection,
    /// The frame text.
    pub raw: String,
}

/// Identifier of a scenario; serialized as "S1".."S11" and "S6b".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScenarioId {
    /// Anmelden, Status, Heartbeat.
    #[serde(rename = "S1")]
    S1,
    /// Auto anstecken und laden.
    #[serde(rename = "S2")]
    S2,
    /// Testgrenze aus dem Intranet.
    #[serde(rename = "S3")]
    S3,
    /// Server weg während einer Grenze.
    #[serde(rename = "S4")]
    S4,
    /// StopTransaction mit transactionId 0.
    #[serde(rename = "S5")]
    S5,
    /// Box lehnt Profil ab.
    #[serde(rename = "S6")]
    S6,
    /// Box ohne Ladestand (SoC).
    #[serde(rename = "S6b")]
    S6b,
    /// Zweite Verbindung derselben Kennung.
    #[serde(rename = "S7")]
    S7,
    /// Uhr der Box geht falsch.
    #[serde(rename = "S8")]
    S8,
    /// Fahrplan steuert die Box.
    #[serde(rename = "S9")]
    S9,
    /// Falsches Passwort.
    #[serde(rename = "S10")]
    S10,
    /// App-Sicht während des Ladens.
    #[serde(rename = "S11")]
    S11,
}

/// Static description of a scenario for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioInfo {
    /// Scenario id.
    pub id: ScenarioId,
    /// German title.
    pub title: String,
    /// German description including what a human must do.
    pub description: String,
    /// False for scenarios that must never run against live.
    pub live_allowed: bool,
    /// True when a person has to trigger something outside the simulator.
    pub needs_human: bool,
    /// Maximum run time in seconds.
    pub timeout_s: u64,
}

/// Result of one check inside a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    /// The expectation held.
    Passed,
    /// The expectation did not hold.
    Failed,
    /// Could not be decided; the detail says why.
    Skipped,
}

/// One named check with a German explanation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    /// German name of the check.
    pub name: String,
    /// Outcome.
    pub outcome: CheckOutcome,
    /// German detail text.
    pub detail: String,
}

/// Overall result of a scenario run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportOutcome {
    /// No check failed.
    Passed,
    /// At least one check failed.
    Failed,
    /// Stopped by the user.
    Aborted,
}

/// Report of one scenario run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioReport {
    /// Which scenario ran.
    pub scenario_id: ScenarioId,
    /// Local id of the box.
    pub charge_point_id: String,
    /// Backend the box was connected to.
    pub target_kind: TargetKind,
    /// Start of the run.
    pub started_at: DateTime<Utc>,
    /// End of the run.
    pub finished_at: DateTime<Utc>,
    /// Overall outcome.
    pub outcome: ReportOutcome,
    /// All checks in the order they were decided.
    pub checks: Vec<CheckResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn connection_state_is_internally_tagged_with_camel_case_fields() {
        let v = serde_json::to_value(ConnectionState::Reconnecting {
            attempt: 2,
            next_attempt_at: DateTime::from_timestamp(0, 0).unwrap(),
        })
        .unwrap();
        assert_eq!(v["state"], "reconnecting");
        assert_eq!(v["attempt"], 2);
        assert!(v.get("nextAttemptAt").is_some());
        let v = serde_json::to_value(ConnectionState::Failed { reason: "x".into() }).unwrap();
        assert_eq!(v, json!({"state": "failed", "reason": "x"}));
        let v = serde_json::to_value(ConnectionState::Disconnected).unwrap();
        assert_eq!(v, json!({"state": "disconnected"}));
    }

    #[test]
    fn ocpp_status_serializes_as_ocpp_strings() {
        for (status, text) in [
            (OcppStatus::SuspendedEV, "SuspendedEV"),
            (OcppStatus::SuspendedEVSE, "SuspendedEVSE"),
            (OcppStatus::Available, "Available"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(text));
            assert_eq!(status.as_str(), text);
        }
    }

    #[test]
    fn scenario_ids_use_contract_spelling() {
        assert_eq!(serde_json::to_value(ScenarioId::S6b).unwrap(), json!("S6b"));
        assert_eq!(serde_json::to_value(ScenarioId::S11).unwrap(), json!("S11"));
        let parsed: ScenarioId = serde_json::from_value(json!("S6b")).unwrap();
        assert_eq!(parsed, ScenarioId::S6b);
    }

    #[test]
    fn rate_units_and_target_kind_use_contract_spelling() {
        assert_eq!(serde_json::to_value(RateUnit::W).unwrap(), json!("W"));
        assert_eq!(serde_json::to_value(RateUnit::A).unwrap(), json!("A"));
        assert_eq!(
            serde_json::to_value(TargetKind::Live).unwrap(),
            json!("live")
        );
    }

    #[test]
    fn config_uses_camel_case_fields() {
        let v = serde_json::to_value(ChargePointConfig::default_local("AP1")).unwrap();
        for key in [
            "targetKind",
            "baseUrl",
            "maxPowerW",
            "supportsSoc",
            "acceptedRateUnits",
            "rejectProfiles",
            "clockOffsetS",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
    }
}
