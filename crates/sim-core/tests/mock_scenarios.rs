//! Integration tests: the scenarios S1..S11 against a mock of the Ampiera central system.

mod common;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use common::{
    fast_settings, local_config, wait_until, wait_until_async, Harness, Mock, RecordingSink,
    IDENTITY, PASSWORD,
};
use sim_core::model::{CheckOutcome, ReportOutcome, ScenarioId, ScenarioReport};
use sim_core::{SimError, Simulator};

const WAIT: Duration = Duration::from_secs(10);

fn outcome_of(report: &ScenarioReport, check: &str) -> CheckOutcome {
    report
        .checks
        .iter()
        .find(|c| c.name == check)
        .unwrap_or_else(|| panic!("check {check:?} missing in {:#?}", report.checks))
        .outcome
}

fn assert_passed(report: &ScenarioReport) {
    assert_eq!(
        report.outcome,
        ReportOutcome::Passed,
        "{:#?}",
        report.checks
    );
    assert!(report
        .checks
        .iter()
        .all(|c| c.outcome != CheckOutcome::Failed));
}

#[tokio::test]
async fn scenario_s1_boot_status_heartbeat() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S1).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "BootNotification beantwortet"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Aufrufe nach dem Start in richtiger Reihenfolge"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Heartbeat beantwortet"),
        CheckOutcome::Passed
    );
    let markdown = sim_core::report_to_markdown(&report);
    assert!(markdown.contains("Anmelden, Status, Heartbeat"));
}

#[tokio::test]
async fn scenario_s2_plug_in_and_charge() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S2).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "transactionId ab 1"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Leistung größer 0 gemeldet"),
        CheckOutcome::Passed
    );
}

#[tokio::test]
async fn scenario_s5_stop_transaction_zero() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S5).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        h.mock.calls("StopTransaction")[0].payload["transactionId"],
        0
    );
}

#[tokio::test]
async fn scenario_s6b_box_without_soc() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S6b).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Erste Konfiguration (mit SoC) abgelehnt"),
        CheckOutcome::Passed
    );
    assert!(
        h.snapshot(&id).await.config.supports_soc,
        "configuration is restored after the scenario"
    );
}

#[tokio::test]
async fn scenario_s7_second_connection_closes_the_first() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S7).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Erste Verbindung wird geschlossen"),
        CheckOutcome::Passed
    );
}

#[tokio::test]
async fn scenario_s8_wrong_clock_is_answered() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S8).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Zentrale verwirft die alten Werte"),
        CheckOutcome::Skipped
    );
    let shifted = h
        .mock
        .calls("MeterValues")
        .iter()
        .filter_map(|c| {
            c.payload["meterValue"][0]["timestamp"]
                .as_str()
                .map(str::to_string)
        })
        .filter_map(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
        .any(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds() > 800);
    assert!(
        shifted,
        "a MeterValues with a clock that is 15 minutes slow must have been sent"
    );
    assert_eq!(
        h.snapshot(&id).await.config.clock_offset_s,
        0,
        "offset is restored"
    );
}

#[tokio::test]
async fn scenario_s10_wrong_password_locally() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    h.connect_and_wait(&id).await;
    let before = h.mock.attempts();
    let report = h.sim.run_scenario(&id, ScenarioId::S10).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "HTTP 401 bei jedem Versuch"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Kein automatischer Wiederholversuch nach 401"),
        CheckOutcome::Passed
    );
    assert_eq!(
        h.mock.attempts() - before,
        4,
        "exactly three attempts with the wrong password (two direct, one by the wallbox) plus the reconnect \
         that restores the connection"
    );
    // The box had been connected before: after the pause that follows the 401 it is back.
    let sim = h.sim.clone();
    wait_until_async("Wallbox wieder verbunden", WAIT, || {
        let sim = sim.clone();
        let id = id.clone();
        async move {
            sim.list().await.iter().any(|s| {
                s.id == id
                    && matches!(
                        s.connection,
                        sim_core::model::ConnectionState::Connected { .. }
                    )
            })
        }
    })
    .await;
}

/// Sends a test limit once the box charges, like a person pressing "Testgrenze" in the intranet.
/// Returns the `validTo` that was used, filled in when the limit has been sent.
fn trigger_test_limit_later(
    h: &Harness,
    seconds_valid: i64,
) -> Arc<std::sync::Mutex<Option<chrono::DateTime<Utc>>>> {
    let sent = Arc::new(std::sync::Mutex::new(None));
    let slot = sent.clone();
    let mock = h.mock.clone();
    tokio::spawn(async move {
        wait_until("Ladevorgang", WAIT, || mock.count("MeterValues") >= 1).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let valid_to = Utc::now() + chrono::Duration::seconds(seconds_valid);
        *slot.lock().unwrap() = Some(valid_to);
        mock.send_test_limit(IDENTITY, 4000.0, valid_to, Some(1))
            .await;
    });
    sent
}

#[tokio::test]
async fn scenario_s3_test_limit_regulates_and_ends() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    trigger_test_limit_later(&h, 5);
    let report = h.sim.run_scenario(&id, ScenarioId::S3).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Wallbox regelt auf die Grenze"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Nach validTo lädt die Wallbox mit voller Leistung"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Gültigkeit höchstens 15 Minuten"),
        CheckOutcome::Passed
    );
}

#[tokio::test]
async fn scenario_s3_sees_a_profile_that_arrives_right_after_start_transaction() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let mock = h.mock.clone();
    tokio::spawn(async move {
        // No pause: the profile follows StartTransaction while the scenario is still waiting for "Charging".
        wait_until("StartTransaction", WAIT, || {
            mock.count("StartTransaction") >= 1
        })
        .await;
        let valid_to = Utc::now() + chrono::Duration::seconds(5);
        mock.send_test_limit(IDENTITY, 4000.0, valid_to, Some(1))
            .await;
    });
    let report = h.sim.run_scenario(&id, ScenarioId::S3).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Profil empfangen"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Wallbox regelt auf die Grenze"),
        CheckOutcome::Passed
    );
}

#[tokio::test]
async fn aborting_s4_while_the_box_is_blocked_brings_it_back_online() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    // A block far longer than the test: only the cleanup after the scenario can end it.
    settings.tuning.after_valid_to = Duration::from_secs(120);
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    let valid_to = Utc::now() + chrono::Duration::seconds(5);
    let sender = mock.clone();
    tokio::spawn(async move {
        wait_until("Ladevorgang", WAIT, || sender.count("MeterValues") >= 1).await;
        sender
            .send_test_limit(IDENTITY, 4000.0, valid_to, Some(1))
            .await;
    });
    let run_sim = sim.clone();
    let run_id = id.clone();
    let run = tokio::spawn(async move { run_sim.run_scenario(&run_id, ScenarioId::S4).await });
    let probe = sim.clone();
    let probe_id = id.clone();
    wait_until_async("Wallbox getrennt und gesperrt", WAIT, || {
        let sim = probe.clone();
        let id = probe_id.clone();
        async move {
            sim.list().await.iter().any(|s| {
                s.id == id
                    && matches!(
                        s.connection,
                        sim_core::model::ConnectionState::Reconnecting { .. }
                    )
            })
        }
    })
    .await;
    sim.abort_scenario(&id).await.unwrap();
    let report = run.await.unwrap().unwrap();
    assert_eq!(report.outcome, ReportOutcome::Aborted);
    let m = mock.clone();
    wait_until("Wallbox wieder verbunden", WAIT, || {
        m.is_connected(IDENTITY)
    })
    .await;
}

#[tokio::test]
async fn scenario_s4_server_gone_during_a_limit() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let valid_to_slot = trigger_test_limit_later(&h, 5);
    let report = h.sim.run_scenario(&id, ScenarioId::S4).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Grenze gilt ohne Verbindung"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Grenze endet zum Ablaufzeitpunkt ohne Zentrale"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Neuanmeldung nach der Sperre"),
        CheckOutcome::Passed
    );
    let mock = h.mock.clone();
    wait_until("second BootNotification", WAIT, || {
        mock.count("BootNotification") == 2
    })
    .await;
    let valid_to = valid_to_slot.lock().unwrap().expect("limit was sent");
    let second_boot = mock.calls("BootNotification")[1].at;
    let blocked_for = (second_boot - valid_to).num_milliseconds();
    assert!(
        blocked_for >= 1900,
        "reconnect must wait until validTo + after_valid_to (2 s), came {blocked_for} ms after validTo"
    );
}

#[tokio::test]
async fn scenario_s6_box_rejects_profiles() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let mock = h.mock.clone();
    tokio::spawn(async move {
        wait_until("Ladevorgang", WAIT, || mock.count("MeterValues") >= 1).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = mock
            .send_call(
                IDENTITY,
                "SetChargingProfile",
                serde_json::json!({"connectorId": 0, "csChargingProfiles": {
                    "chargingProfileId": 4711, "stackLevel": 1, "chargingProfilePurpose": "TxDefaultProfile",
                    "chargingProfileKind": "Absolute",
                    "validTo": (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339(),
                    "chargingSchedule": {"startSchedule": (Utc::now() - chrono::Duration::seconds(60)).to_rfc3339(),
                      "chargingRateUnit": "A", "chargingSchedulePeriod": [{"startPeriod": 0, "limit": 10.0}]}}}),
            )
            .await;
    });
    let report = h.sim.run_scenario(&id, ScenarioId::S6).await.unwrap();
    assert_passed(&report);
    assert_eq!(
        outcome_of(&report, "Wallbox antwortet mit Rejected"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Keine Grenze wirkt"),
        CheckOutcome::Passed
    );
    assert!(
        !h.snapshot(&id).await.config.reject_profiles,
        "configuration is restored"
    );
}

#[tokio::test]
async fn aborting_a_scenario_gives_an_aborted_report() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let sim = h.sim.clone();
    let run_id = id.clone();
    let run = tokio::spawn(async move { sim.run_scenario(&run_id, ScenarioId::S3).await });
    let sim = h.sim.clone();
    let abort_id = id.clone();
    wait_until_async("scenario running", WAIT, || {
        let sim = sim.clone();
        let abort_id = abort_id.clone();
        async move { sim.abort_scenario(&abort_id).await.is_ok() }
    })
    .await;
    let report = run.await.unwrap().unwrap();
    assert_eq!(report.outcome, ReportOutcome::Aborted);
    assert_eq!(
        h.sim.abort_scenario(&id).await,
        Err(SimError::NoScenarioRunning)
    );
}

#[tokio::test]
async fn human_scenario_keeps_waiting_for_the_human_step_until_aborted() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let sim = h.sim.clone();
    let run_id = id.clone();
    let run = tokio::spawn(async move { sim.run_scenario(&run_id, ScenarioId::S6).await });
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !run.is_finished(),
        "S6 must keep waiting for the test limit from the intranet"
    );
    h.sim.abort_scenario(&id).await.unwrap();
    let report = run.await.unwrap().unwrap();
    assert_eq!(report.outcome, ReportOutcome::Aborted);
}

struct FakeApp(sim_core::AppProbeData);

#[async_trait::async_trait]
impl sim_core::AppProbe for FakeApp {
    async fn snapshot(&self) -> Result<sim_core::AppProbeData, String> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn scenario_s11_without_app_probe_skips_all_checks() {
    let h = Harness::new().await;
    let id = h.add_box().await;
    let report = h.sim.run_scenario(&id, ScenarioId::S11).await.unwrap();
    assert_eq!(report.checks.len(), 4);
    assert!(report
        .checks
        .iter()
        .all(|c| c.outcome == CheckOutcome::Skipped));
    assert!(report.checks[0].detail.contains("angemeldet"));
}

#[tokio::test]
async fn scenario_s11_reports_the_known_app_gaps_as_failures() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    settings.tuning.app_poll_interval = Duration::from_millis(300);
    settings.tuning.app_max_wait = Some(Duration::from_secs(2));
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    // What the gap of 06.10.2026 looks like: connection online, but no live power, wallbox offline, no quarter value.
    sim.set_app_probe(Arc::new(FakeApp(sim_core::AppProbeData {
        connection: Some("online".into()),
        device_status: Some("offline".into()),
        live_power_w: None,
        ..Default::default()
    })));
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    let report = sim.run_scenario(&id, ScenarioId::S11).await.unwrap();
    assert_eq!(report.outcome, ReportOutcome::Failed);
    assert_eq!(
        outcome_of(&report, "Verbindung in der App"),
        CheckOutcome::Passed
    );
    assert_eq!(
        outcome_of(&report, "Gerätestatus in der App"),
        CheckOutcome::Failed
    );
    assert_eq!(
        outcome_of(&report, "Aktuelle Leistung in der App"),
        CheckOutcome::Failed
    );
    assert_eq!(
        outcome_of(&report, "Viertelstunden-kWh in der App"),
        CheckOutcome::Failed
    );
}

#[tokio::test]
async fn scenario_s11_passes_live_power_when_the_app_matches_the_box() {
    let mock = Mock::start().await;
    let mut settings = fast_settings();
    settings.tuning.app_poll_interval = Duration::from_millis(300);
    settings.tuning.app_max_wait = Some(Duration::from_secs(2));
    let sim = Simulator::with_settings(Arc::new(RecordingSink::default()), settings);
    sim.set_app_probe(Arc::new(FakeApp(sim_core::AppProbeData {
        connection: Some("online".into()),
        device_status: Some("laedt".into()),
        live_power_w: Some(10_800.0),
        ..Default::default()
    })));
    let id = sim
        .add(local_config(&mock), PASSWORD.into(), false)
        .await
        .unwrap();
    let report = sim.run_scenario(&id, ScenarioId::S11).await.unwrap();
    assert_eq!(
        outcome_of(&report, "Aktuelle Leistung in der App"),
        CheckOutcome::Passed,
        "within 10 % of 11000 W"
    );
    assert_eq!(
        outcome_of(&report, "Gerätestatus in der App"),
        CheckOutcome::Passed
    );
}
