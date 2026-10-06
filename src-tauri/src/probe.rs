//! Connects the app view to the simulator core (scenario S11): the core asks for the numbers the
//! customer app shows and compares them with the simulated box.

use app_view::{extract::parse_time, AppClient, AppViewSnapshot};
use async_trait::async_trait;
use sim_core::{AppProbe, AppProbeData};
use std::sync::Arc;

/// Answers core probes from `AppClient::snapshot`, which is cached for 30 seconds.
pub struct AppProbeAdapter {
    client: Arc<AppClient>,
}

impl AppProbeAdapter {
    /// Creates an adapter over a shared client.
    pub fn new(client: Arc<AppClient>) -> Self {
        Self { client }
    }
}

/// Maps the summary one to one; unknown values stay `None`.
pub fn to_probe_data(snapshot: &AppViewSnapshot) -> AppProbeData {
    let s = &snapshot.summary;
    AppProbeData {
        live_power_w: s.live_power_w,
        device_status: s.device_status.clone(),
        connection: s.connection.clone(),
        last_quarter_kwh: s.last_quarter_kwh,
        last_quarter_at: s.last_quarter_at.as_deref().and_then(parse_time),
    }
}

#[async_trait]
impl AppProbe for AppProbeAdapter {
    async fn snapshot(&self) -> Result<AppProbeData, String> {
        let snapshot = self.client.snapshot().await.map_err(|e| e.to_string())?;
        Ok(to_probe_data(&snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_view::AppSummary;
    use chrono::Utc;
    use serde_json::Map;

    fn snapshot(summary: AppSummary) -> AppViewSnapshot {
        AppViewSnapshot {
            fetched_at: Utc::now(),
            installation_id: None,
            summary,
            raw: Map::new(),
        }
    }

    #[test]
    fn empty_summary_maps_to_all_none() {
        let data = to_probe_data(&snapshot(AppSummary::default()));
        assert_eq!(data.live_power_w, None);
        assert_eq!(data.device_status, None);
        assert_eq!(data.connection, None);
        assert_eq!(data.last_quarter_kwh, None);
        assert_eq!(data.last_quarter_at, None);
    }

    #[test]
    fn values_are_passed_on_and_time_is_parsed() {
        let data = to_probe_data(&snapshot(AppSummary {
            connection: Some("online".into()),
            device_status: Some("offline".into()),
            live_power_w: Some(7360.0),
            last_quarter_kwh: Some(1.75),
            last_quarter_at: Some("2026-10-06T10:15:00Z".into()),
            ..AppSummary::default()
        }));
        assert_eq!(data.connection.as_deref(), Some("online"));
        assert_eq!(data.device_status.as_deref(), Some("offline"));
        assert_eq!(data.live_power_w, Some(7360.0));
        assert_eq!(data.last_quarter_kwh, Some(1.75));
        assert_eq!(
            data.last_quarter_at.map(|t| t.to_rfc3339()),
            Some("2026-10-06T10:15:00+00:00".into())
        );
    }
}
