//! Wire types of the app view (CONTRACT section 1). JSON uses camelCase.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Result of `login`: either signed in, or the server mailed a code for an unknown device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AppLoginResult {
    /// Tokens are held in memory.
    Ok,
    /// Call `verify_device` with the six-digit code from the e-mail.
    DeviceCodeRequired,
}

/// One upcoming wallbox step of the charging schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextScheduleStep {
    /// Step start, ISO 8601 UTC.
    pub start: String,
    /// Action name as sent by the backend.
    pub action: String,
    /// Target power in watts.
    pub target_power_w: f64,
}

/// Values the S11 scenario compares with the simulated box. Unknown values are `None`, never 0.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    /// Dashboard `verbindung`.
    pub connection: Option<String>,
    /// Dashboard `letzter_kontakt`.
    pub last_contact: Option<String>,
    /// Status of the wallbox device in the dashboard.
    pub device_status: Option<String>,
    /// Dashboard `live.wallbox_leistung_w`.
    pub live_power_w: Option<f64>,
    /// `zustand` of the wallbox in the geo-position view.
    pub geo_state: Option<String>,
    /// Headline of the geo-position view.
    pub geo_headline: Option<String>,
    /// `kwhPositiv` of the latest quarter-hour value of the wallbox.
    pub last_quarter_kwh: Option<f64>,
    /// `zeit` of that value, ISO 8601 UTC.
    pub last_quarter_at: Option<String>,
    /// Up to four wallbox steps starting now or later.
    pub next_schedule_steps: Vec<NextScheduleStep>,
}

/// What the customer app currently shows for the selected installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppViewSnapshot {
    /// When the data was fetched.
    pub fetched_at: DateTime<Utc>,
    /// Installation the data belongs to.
    pub installation_id: Option<String>,
    /// Extracted comparison values.
    pub summary: AppSummary,
    /// Endpoint path to parsed JSON body, or `{"error": "<German text>"}` for a failed endpoint.
    pub raw: Map<String, Value>,
}
