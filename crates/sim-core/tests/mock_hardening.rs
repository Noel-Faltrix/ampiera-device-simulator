//! Integration tests for the protective rules: box limits, connect gate, cooldown, flap detection, backoff,
//! transaction retry and restarts. They run against the mock of the central system.

mod common;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use common::{
    fast_settings, local_config, wait_until, wait_until_async, Harness, Mock, RecordingSink,
    IDENTITY, PASSWORD,
};
use sim_core::model::{ConnectionState, OcppStatus, ScenarioId, TargetKind};
use sim_core::{SimError, Simulator};

const WAIT: Duration = Duration::from_secs(10);

async fn wait_connection(
    h: &Harness,
    id: &str,
    what: &str,
    pred: impl Fn(&ConnectionState) -> bool + Copy,
) {
    wait_until_async(what, WAIT, || async {
        pred(&h.snapshot(id).await.connection)
    })
    .await;
}

#[tokio::test]
async fn a_local_box_with_a_public_host_is_refused() {
    let h = Harness::new().await;
    let mut config = local_config(&h.mock);
    assert_eq!(config.target_kind, TargetKind::Local);
    config.base_url = "wss://api.ampiera.de/ocpp".into();
    let result = h.sim.add(config, PASSWORD.into(), true).await;
    assert!(
        matches!(result, Err(SimError::PublicHostForLocalTarget(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn duplicate_boxes_too_many_boxes_and_bad_passwords_are_refused() {
    let h = Harness::new().await;
    h.add_box().await;
    assert_eq!(
        h.sim
            .add(local_config(&h.mock), PASSWORD.into(), false)
            .await,
        Err(SimError::DuplicateBox)
    );
    for bad in ["", "päss", "line\nbreak"] {
        let mut config = local_config(&h.mock);
        config.identity = "APOTHER".into();
        assert_eq!(
            h.sim.add(config, bad.into(), false).await,
            Err(SimError::InvalidPassword),
            "password {bad:?}"
        );
    }
    for n in 1..20 {
        let mut config = local_config(&h.mock);
        config.identity = format!("APBOX{n:03}");
        h.sim
            .add(config, PASSWORD.into(), false)
            .await
            .expect("up to 20 boxes");
    }
    let mut config = local_config(&h.mock);
    config.identity = "APBOX999".into();
    assert_eq!(
        h.sim.add(config, PASSWORD.into(), false).await,
        Err(SimError::TooManyBoxes { max: 20 })
    );
    assert_eq!(
        h.sim.list().await.len(),
        20,
        "the refused box does not exist"
    );
}

#[tokio::test]
async fn parallel_connects_cannot_exceed_the_live_limit() {
    let h = Harness::new().await;
    let mut live = local_config(&h.mock);
    live.target_kind = TargetKind::Live;
    let mut ids = Vec::new();
    for n in 0..4 {
        let mut config = live.clone();
        config.identity = format!("APLIVE00000{n}");
        ids.push(h.sim.add(config, PASSWORD.into(), true).await.unwrap());
    }
    let attempts = ids.iter().map(|id| {
        let sim = h.sim.clone();
        let id = id.clone();
        tokio::spawn(async move { sim.connect(&id).await })
    });
    let results = futures_util::future::join_all(attempts).await;
    let refused = results
        .iter()
        .filter(|r| matches!(r, Ok(Err(SimError::TooManyLiveBoxes { .. }))))
        .count();
    let accepted = results.iter().filter(|r| matches!(r, Ok(Ok(())))).count();
    assert_eq!((accepted, refused), (3, 1), "{results:?}");
}

#[tokio::test]
async fn a_rejected_login_pauses_every_connect_for_a_while() {
    let h = Harness::new().await;
    let id = h
        .sim
        .add(local_config(&h.mock), "falsch".into(), false)
        .await
        .unwrap();
    h.sim.connect(&id).await.unwrap();
    wait_connection(&h, &id, "Zustand fehlgeschlagen", |c| {
        matches!(c, ConnectionState::Failed { .. })
    })
    .await;
    let attempts = h.mock.attempts();
    let paused = h.sim.connect(&id).await;
    assert!(
        matches!(paused, Err(SimError::ConnectCooldown { remaining_s }) if remaining_s >= 1),
        "{paused:?}"
    );
    assert!(paused.unwrap_err().to_string().contains("noch"));
    assert_eq!(h.mock.attempts(), attempts, "no handshake during the pause");
    tokio::time::sleep(Duration::from_millis(700)).await;
    h.sim.connect(&id).await.expect("the pause is over");
}

#[tokio::test]
async fn a_central_system_that_keeps_dropping_the_connection_ends_in_failed() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    settings.timings.flap_max_losses = 2;
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    sim.connect(&id).await.unwrap();
    for _ in 0..3 {
        let m = mock.clone();
        wait_until("Verbindung steht", WAIT, || m.is_connected(IDENTITY)).await;
        mock.close(IDENTITY);
        let m = mock.clone();
        wait_until("Verbindung weg", WAIT, || !m.is_connected(IDENTITY)).await;
    }
    let sim2 = sim.clone();
    let id2 = id.clone();
    wait_until_async("Zustand fehlgeschlagen", WAIT, || {
        let sim = sim2.clone();
        let id = id2.clone();
        async move {
            sim.list()
                .await
                .iter()
                .any(|s| s.id == id && matches!(s.connection, ConnectionState::Failed { .. }))
        }
    })
    .await;
    let snapshot = sim.list().await.remove(0);
    let ConnectionState::Failed { reason } = snapshot.connection else {
        panic!("expected failed");
    };
    assert!(reason.contains("abgebrochen"), "{reason}");
    let attempts = mock.attempts();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        mock.attempts(),
        attempts,
        "no automatic retry after flapping"
    );
}

#[tokio::test]
async fn the_backoff_counter_survives_a_connection_that_was_not_stable() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    settings.timings.stable_after = Duration::from_secs(60);
    settings.timings.flap_max_losses = 10;
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    sim.connect(&id).await.unwrap();
    for _ in 0..2 {
        let m = mock.clone();
        wait_until("Verbindung steht", WAIT, || m.is_connected(IDENTITY)).await;
        // Wait for the boot to be accepted: only then could the stability rule start counting.
        let m = mock.clone();
        wait_until("Boot", WAIT, || m.count("TriggerMessage.result") >= 1).await;
        mock.close(IDENTITY);
        let m = mock.clone();
        wait_until("Verbindung weg", WAIT, || !m.is_connected(IDENTITY)).await;
    }
    let sim2 = sim.clone();
    wait_until_async("Wiederverbindung Versuch 2", WAIT, || {
        let sim = sim2.clone();
        async move {
            sim.list().await.iter().any(|s| {
                matches!(s.connection, ConnectionState::Reconnecting { attempt, .. } if attempt >= 2)
            })
        }
    })
    .await;
}

#[tokio::test]
async fn a_failed_start_transaction_is_retried_later() {
    let h = Harness::new().await;
    h.mock.reject_next_starts(1);
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;
    assert_eq!(
        h.mock.count("StartTransaction"),
        2,
        "the rejected start is repeated after the retry delay"
    );
}

#[tokio::test]
async fn reset_ends_the_transaction_with_its_reason_and_charging_resumes_after_boot() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;
    let answer = h
        .mock
        .send_call(IDENTITY, "Reset", serde_json::json!({"type": "Soft"}))
        .await
        .expect("answer");
    assert_eq!(answer["status"], "Accepted");
    let mock = h.mock.clone();
    wait_until("zweites BootNotification", WAIT, || {
        mock.count("BootNotification") >= 2
    })
    .await;
    let stops = h.mock.calls("StopTransaction");
    assert_eq!(stops.len(), 1, "{stops:?}");
    assert_eq!(stops[0].payload["reason"], "SoftReset");
    assert!(
        stops[0].at <= h.mock.calls("BootNotification")[1].at,
        "StopTransaction goes out before the box reconnects"
    );
    let mock = h.mock.clone();
    wait_until("neue Transaktion", WAIT, || {
        mock.count("StartTransaction") >= 2
    })
    .await;
    h.wait_status(&id, OcppStatus::Charging).await;
}

#[tokio::test]
async fn a_remote_start_during_a_running_transaction_is_rejected() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    h.sim
        .plug_in(&id, sim_core::model::VehicleConfig::default())
        .await
        .unwrap();
    h.wait_status(&id, OcppStatus::Charging).await;
    let answer = h
        .mock
        .send_call(
            IDENTITY,
            "RemoteStartTransaction",
            serde_json::json!({"idTag": "AMPIERA", "connectorId": 1}),
        )
        .await
        .expect("answer");
    assert_eq!(answer["status"], "Rejected");
}

#[tokio::test]
async fn oversized_frames_from_the_central_system_end_the_connection_with_a_german_reason() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    // 70 KiB payload: above the 64 KiB message limit of the websocket.
    let big = "x".repeat(70 * 1024);
    let _ = h
        .mock
        .send_call(
            IDENTITY,
            "DataTransfer",
            serde_json::json!({"vendorId": "x", "data": big}),
        )
        .await;
    let sim = h.sim.clone();
    let id2 = id.clone();
    wait_until_async("Verbindung verloren", WAIT, || {
        let sim = sim.clone();
        let id = id2.clone();
        async move {
            sim.list().await.iter().any(|s| {
                s.id == id
                    && s.last_error
                        .as_deref()
                        .is_some_and(|e| e.contains("abgebrochen"))
            })
        }
    })
    .await;
}

#[tokio::test]
async fn snapshots_carry_an_update_time_that_moves_with_changes() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let first = h.snapshot(&id).await.updated_at;
    h.connect_and_wait(&id).await;
    let second = h.snapshot(&id).await.updated_at;
    assert!(second > first);
    assert!(second <= Utc::now());
    let json = serde_json::to_value(h.snapshot(&id).await).unwrap();
    assert!(json.get("updatedAt").is_some());
}

#[tokio::test]
async fn s10_is_refused_for_a_local_box_that_points_at_a_public_host_via_live_kind() {
    // A live box is live whatever its host looks like: the scenario guard follows the effective target.
    let h = Harness::new().await;
    let mut config = local_config(&h.mock);
    config.target_kind = TargetKind::Live;
    let id = h.sim.add(config, PASSWORD.into(), true).await.unwrap();
    assert_eq!(
        h.sim.run_scenario(&id, ScenarioId::S10).await.err(),
        Some(SimError::ScenarioNotAllowedOnLive(ScenarioId::S10))
    );
}

#[tokio::test]
async fn a_manual_connect_while_reconnecting_keeps_the_backoff_schedule() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    settings.timings.backoff_base = Duration::from_secs(2);
    settings.timings.stable_after = Duration::from_secs(60);
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    sim.connect(&id).await.unwrap();
    let m = mock.clone();
    wait_until("Boot", WAIT, || m.count("TriggerMessage.result") >= 1).await;
    mock.close(IDENTITY);
    let probe = sim.clone();
    wait_until_async("Reconnecting", WAIT, || {
        let sim = probe.clone();
        async move {
            sim.list()
                .await
                .iter()
                .any(|s| matches!(s.connection, ConnectionState::Reconnecting { .. }))
        }
    })
    .await;
    let attempts = mock.attempts();
    sim.connect(&id)
        .await
        .expect("accepted, but changes nothing");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(mock.attempts(), attempts, "no immediate handshake");
    assert!(matches!(
        sim.list().await[0].connection,
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
}

#[tokio::test]
async fn s7_is_refused_when_the_live_limit_is_used_up() {
    let h = Harness::new().await;
    let mut live = local_config(&h.mock);
    live.target_kind = TargetKind::Live;
    let mut ids = Vec::new();
    for n in 0..3 {
        let mut config = live.clone();
        config.identity = format!("APLIVE00000{n}");
        let id = h.sim.add(config, PASSWORD.into(), true).await.unwrap();
        h.sim.connect(&id).await.unwrap();
        ids.push(id);
    }
    for n in 0..3 {
        let m = h.mock.clone();
        wait_until("Verbindung", WAIT, || {
            m.is_connected(&format!("APLIVE00000{n}"))
        })
        .await;
    }
    let report = h.sim.run_scenario(&ids[0], ScenarioId::S7).await.unwrap();
    let check = report
        .checks
        .iter()
        .find(|c| c.name == "Zweite Verbindung öffnen")
        .expect("check exists");
    assert_eq!(check.outcome, sim_core::model::CheckOutcome::Failed);
    assert!(check.detail.contains("höchstens 3"), "{}", check.detail);
}

#[tokio::test]
async fn s10_restore_may_outlast_the_scenario_timeout_without_failing_the_scenario() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    settings.timings.cooldown = Duration::from_secs(4);
    settings.tuning.timeout_override = Some(Duration::from_secs(2));
    settings.tuning.no_retry_observation = Duration::from_millis(500);
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    sim.connect(&id).await.unwrap();
    let m = mock.clone();
    wait_until("Boot", WAIT, || m.count("TriggerMessage.result") >= 1).await;
    let report = sim.run_scenario(&id, ScenarioId::S10).await.unwrap();
    assert!(
        report
            .checks
            .iter()
            .all(|c| c.outcome != sim_core::model::CheckOutcome::Failed),
        "{:#?}",
        report.checks
    );
    assert_eq!(report.outcome, sim_core::model::ReportOutcome::Passed);
    assert!(
        matches!(
            sim.list().await[0].connection,
            ConnectionState::Connected { .. }
        ),
        "the wallbox is back after the pause"
    );
}
