//! Integration tests: simulated boxes against a mock of the Ampiera central system.

mod common;

use std::time::Duration;

use chrono::Utc;
use common::{
    local_config, meter_power, status_of, wait_until, wait_until_async, Harness, IDENTITY, PASSWORD,
};
use sim_core::model::{ConnectionState, OcppStatus, ScenarioId, TargetKind};
use sim_core::SimError;

const WAIT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn boot_and_post_boot_configuration_are_answered() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;

    let boot = &h.mock.calls("BootNotification")[0].payload;
    assert_eq!(boot["chargePointVendor"], "Ampiera");
    let configs = h.mock.calls("ChangeConfiguration.result");
    assert!(
        configs.iter().all(|c| status_of(&c.payload) == "Accepted"),
        "{configs:?}"
    );
    assert_eq!(configs.len(), 2, "sampled data and interval");
    assert_eq!(h.mock.count("TriggerMessage.result"), 1);
    assert_eq!(
        status_of(&h.mock.calls("TriggerMessage.result")[0].payload),
        "Accepted"
    );
    let first_status = &h.mock.calls("StatusNotification")[0].payload;
    assert_eq!(first_status["status"], "Available");
    assert_eq!(first_status["connectorId"], 1);

    let snapshot = h.snapshot(&id).await;
    assert!(matches!(
        snapshot.connection,
        ConnectionState::Connected { .. }
    ));
    assert_eq!(snapshot.heartbeat_interval_s, Some(1));
    let mock = h.mock.clone();
    wait_until("Heartbeat", WAIT, || mock.count("Heartbeat") >= 1).await;
}

#[tokio::test]
async fn plug_in_starts_a_transaction_and_reports_power() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;

    let mock = h.mock.clone();
    wait_until("MeterValues", WAIT, || mock.count("MeterValues") >= 2).await;
    let statuses: Vec<String> = h
        .mock
        .calls("StatusNotification")
        .iter()
        .map(|c| status_of(&c.payload).to_string())
        .collect();
    assert_eq!(
        statuses,
        vec!["Available", "Available", "Preparing", "Charging"],
        "boot status, answer to the triggered StatusNotification, then the plug-in sequence"
    );
    let start = &h.mock.calls("StartTransaction")[0].payload;
    assert_eq!(start["idTag"], "SIMULATOR");
    assert_eq!(start["connectorId"], 1);
    let meter = &h.mock.calls("MeterValues")[0].payload;
    assert_eq!(meter_power(meter), Some(11_000.0));
    assert_eq!(meter["transactionId"], 1);
    let measurands: Vec<&str> = meter["meterValue"][0]["sampledValue"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["measurand"].as_str())
        .collect();
    assert_eq!(
        measurands,
        vec![
            "Power.Active.Import",
            "Energy.Active.Import.Register",
            "SoC"
        ]
    );

    let snapshot = h.snapshot(&id).await;
    assert_eq!(snapshot.transaction_id, Some(1));
    assert!(snapshot.energy_wh > 0.0);

    h.sim.unplug(&id).await.unwrap();
    h.wait_status(&id, OcppStatus::Available).await;
    let mock = h.mock.clone();
    wait_until("StopTransaction", WAIT, || {
        mock.count("StopTransaction") == 1
    })
    .await;
    let stop = &h.mock.calls("StopTransaction")[0].payload;
    assert_eq!(stop["transactionId"], 1);
    assert_eq!(stop["reason"], "EVDisconnected");
}

#[tokio::test]
async fn a_profile_limits_power_and_stops_limiting_after_valid_to() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;

    let valid_to = Utc::now() + chrono::Duration::seconds(4);
    h.mock
        .send_test_limit(IDENTITY, 4000.0, valid_to, Some(1))
        .await;

    let snapshot = h.snapshot(&id).await;
    let limit = snapshot.active_limit.expect("limit active");
    assert_eq!(limit.limit_w, 4000.0);
    assert_eq!(
        limit.valid_to.map(|t| t.timestamp()),
        Some(valid_to.timestamp())
    );
    assert_eq!(snapshot.profile_count, 2);
    wait_until_async("power at 4000 W", WAIT, || async {
        h.snapshot(&id).await.power_w == Some(4000.0)
    })
    .await;
    let mock = h.mock.clone();
    wait_until("limited MeterValues", WAIT, || {
        mock.calls("MeterValues")
            .iter()
            .any(|c| meter_power(&c.payload) == Some(4000.0))
    })
    .await;

    tokio::time::sleep(
        (valid_to - Utc::now()).to_std().unwrap_or_default() + Duration::from_millis(600),
    )
    .await;
    let after = h.snapshot(&id).await;
    assert!(
        after.active_limit.is_none(),
        "the limit must end at validTo: {:?}",
        after.active_limit
    );
    assert_eq!(after.power_w, Some(11_000.0));
    assert_eq!(after.profile_count, 0, "expired profiles are dropped");
    assert_eq!(after.status, OcppStatus::Charging);
}

#[tokio::test]
async fn zero_limit_suspends_the_charge_point() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;
    h.mock
        .send_test_limit(
            IDENTITY,
            0.0,
            Utc::now() + chrono::Duration::seconds(30),
            Some(1),
        )
        .await;
    h.wait_status(&id, OcppStatus::SuspendedEVSE).await;
    let mock = h.mock.clone();
    wait_until("SuspendedEVSE notification", WAIT, || {
        mock.calls("StatusNotification")
            .iter()
            .any(|c| status_of(&c.payload) == "SuspendedEVSE")
    })
    .await;
    let cleared = h
        .mock
        .send_call(
            IDENTITY,
            "ClearChargingProfile",
            serde_json::json!({"id": 4711}),
        )
        .await
        .unwrap();
    assert_eq!(status_of(&cleared), "Accepted");
    let unknown = h
        .mock
        .send_call(
            IDENTITY,
            "ClearChargingProfile",
            serde_json::json!({"id": 4711}),
        )
        .await
        .unwrap();
    assert_eq!(status_of(&unknown), "Unknown");
}

#[tokio::test]
async fn unauthorized_leads_to_failed_without_retry() {
    let h = Harness::new().await;
    let id = h
        .sim
        .add(
            local_config(&h.mock),
            "falsches-passwort".to_string(),
            false,
        )
        .await
        .unwrap();
    h.sim.connect(&id).await.unwrap();
    wait_until_async("failed state", WAIT, || async {
        matches!(
            h.snapshot(&id).await.connection,
            ConnectionState::Failed { .. }
        )
    })
    .await;
    let snapshot = h.snapshot(&id).await;
    let ConnectionState::Failed { reason } = &snapshot.connection else {
        unreachable!()
    };
    assert!(reason.contains("401"), "{reason}");
    assert!(!reason.contains("falsches-passwort"));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(h.mock.attempts(), 1, "no automatic retry after 401");
    assert!(matches!(
        h.snapshot(&id).await.connection,
        ConnectionState::Failed { .. }
    ));
}

#[tokio::test]
async fn lost_connection_reconnects_with_a_fresh_boot() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    assert_eq!(h.mock.count("BootNotification"), 1);
    h.mock.close(IDENTITY);
    let mock = h.mock.clone();
    wait_until("second BootNotification", WAIT, || {
        mock.count("BootNotification") == 2
    })
    .await;
    wait_until_async("connected again", WAIT, || async {
        matches!(
            h.snapshot(&id).await.connection,
            ConnectionState::Connected { .. }
        )
    })
    .await;
}

#[tokio::test]
async fn exported_log_contains_no_credentials() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;
    let log = h.sim.export_log(&id).await.unwrap();
    let entries: Vec<sim_core::model::FrameLogEntry> =
        serde_json::from_str(&log).expect("JSON array of entries");
    assert!(entries.len() > 5);
    let basic = base64_of(&format!("{IDENTITY}:{PASSWORD}"));
    assert!(!log.contains(PASSWORD));
    assert!(!log.contains(&basic));
    assert!(!log.to_lowercase().contains("authorization"));
    assert!(log.contains("BootNotification"));
    let sink_frames = h.sink.frames.lock().unwrap().clone();
    assert!(sink_frames.iter().all(|f| !f.raw.contains(PASSWORD)));
    assert!(sink_frames.len() >= entries.len().saturating_sub(2));
}

fn base64_of(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[tokio::test]
async fn updates_are_throttled_to_four_per_second() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let updates = h.sink.updates.lock().unwrap().clone();
    let times: Vec<std::time::Instant> = updates
        .iter()
        .filter(|(_, s)| s.id == id)
        .map(|(t, _)| *t)
        .collect();
    let last = *times.last().expect("updates");
    let in_last_second = times
        .iter()
        .filter(|t| last.duration_since(**t) < Duration::from_secs(1))
        .count();
    assert!(
        in_last_second <= 4 + 1,
        "{in_last_second} updates in one second"
    );
    assert!(
        in_last_second >= 2,
        "power changes every tick, so updates must keep coming"
    );
}

#[tokio::test]
async fn contract_rules_are_enforced_by_the_core() {
    let h = Harness::new().await;
    let mut live = local_config(&h.mock);
    live.target_kind = TargetKind::Live;
    assert_eq!(
        h.sim.add(live.clone(), PASSWORD.into(), false).await,
        Err(SimError::LiveNotConfirmed)
    );
    let mut public = live.clone();
    public.base_url = "ws://api.ampiera.de/ocpp".into();
    assert!(matches!(
        h.sim.add(public, PASSWORD.into(), true).await,
        Err(SimError::InsecureUrl(_))
    ));

    let mut ids = Vec::new();
    for n in 0..4 {
        let mut config = live.clone();
        config.identity = format!("APLIVE00000{n}");
        ids.push(
            h.sim
                .add(config, PASSWORD.into(), true)
                .await
                .expect("add live box"),
        );
    }
    // Wrong identity for the mock's password check does not matter: the box only has to be "active".
    for id in &ids[..3] {
        h.sim.connect(id).await.expect("first three live boxes");
    }
    assert_eq!(
        h.sim.connect(&ids[3]).await,
        Err(SimError::TooManyLiveBoxes { max: 3 }),
        "fourth live box must be refused"
    );
    h.sim.disconnect(&ids[0]).await.unwrap();
    h.sim.connect(&ids[3]).await.expect("a slot is free again");

    let refused = h.sim.run_scenario(&ids[3], ScenarioId::S10).await;
    assert_eq!(
        refused.err(),
        Some(SimError::ScenarioNotAllowedOnLive(ScenarioId::S10))
    );
    let refused = h.sim.run_scenario(&ids[3], ScenarioId::S9).await;
    assert_eq!(
        refused.err(),
        Some(SimError::ScenarioNotAllowedOnLive(ScenarioId::S9))
    );
}

#[tokio::test]
async fn removing_a_box_stops_it_and_reports_the_removal() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim.remove(&id).await.unwrap();
    assert!(h.sim.list().await.is_empty());
    assert_eq!(
        h.sink.removed.lock().unwrap().as_slice(),
        std::slice::from_ref(&id)
    );
    assert_eq!(h.sim.connect(&id).await, Err(SimError::UnknownBox(id)));
}
