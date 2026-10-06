//! Shared machinery of the scenarios: waiting for frames and state, recording checks, abort handling.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use super::rules::ReceivedCall;
use super::AppProbe;
use crate::handle::{BoxEvent, BoxHandle, CallOutcome};
use crate::model::{
    ChargePointConfig, ChargePointSnapshot, CheckOutcome, CheckResult, ConnectionState,
    FrameDirection, OcppStatus, ScenarioInfo, VehicleConfig,
};
use crate::ocpp::frames::{parse_frame, Frame};

/// Waits and thresholds of the scenarios. The defaults are the values from SCENARIOS.md; tests shorten them.
#[derive(Debug, Clone)]
pub struct ScenarioTuning {
    /// Time after `validTo` the box must have given up the limit and the backend's reconnect block ends
    /// (SCENARIOS S3/S4: "validTo + 30 s").
    pub after_valid_to: Duration,
    /// Time the box needs to apply a profile before power is measured; a few simulation ticks.
    pub limit_settle: Duration,
    /// How long to wait for a second SetChargingProfile (the backend sends the TxProfile right after the default).
    pub second_profile_wait: Duration,
    /// How long to watch for unwanted reconnect attempts after a 401 (S10).
    pub no_retry_observation: Duration,
    /// Time allowed for the connection to come up or to fail.
    pub connect_wait: Duration,
    /// Time to wait for the server to close a replaced socket (S7).
    pub close_wait: Duration,
    /// Time to wait for the post-boot calls of the central system after the boot answer (S1).
    pub post_boot_wait: Duration,
    /// Poll interval of the app view in S11 (CONTRACT: at most every 30 s).
    pub app_poll_interval: Duration,
    /// Minimum charging time in S11 before the quarter-hour comparison is attempted.
    pub min_charge_duration: Duration,
    /// Optional earlier end of S11 than the scenario timeout (used by tests; `None` waits for the
    /// quarter-hour value up to the timeout).
    pub app_max_wait: Option<Duration>,
}

impl Default for ScenarioTuning {
    fn default() -> Self {
        Self {
            after_valid_to: Duration::from_secs(30),
            limit_settle: Duration::from_secs(3),
            second_profile_wait: Duration::from_secs(5),
            no_retry_observation: Duration::from_secs(5),
            connect_wait: Duration::from_secs(30),
            close_wait: Duration::from_secs(10),
            post_boot_wait: Duration::from_secs(20),
            app_poll_interval: Duration::from_secs(30),
            min_charge_duration: Duration::from_secs(15 * 60),
            app_max_wait: None,
        }
    }
}

/// Result of waiting for something.
#[derive(Debug)]
pub enum Wait<T> {
    /// It happened.
    Ready(T),
    /// The time ran out (own limit or scenario deadline).
    TimedOut,
    /// The user aborted.
    Aborted,
}

/// A frame together with when it was seen.
#[derive(Debug, Clone)]
pub struct SeenFrame {
    /// When the box logged it.
    pub at: DateTime<Utc>,
    /// The parsed frame.
    pub frame: Frame,
}

/// State of one scenario run.
pub struct ScenarioCtx {
    /// The box under test.
    pub handle: BoxHandle,
    /// Static info of the scenario being run.
    pub info: ScenarioInfo,
    /// Thresholds.
    pub tuning: ScenarioTuning,
    /// App view, if the shell registered one.
    pub probe: Option<Arc<dyn AppProbe>>,
    abort: watch::Receiver<bool>,
    deadline: Instant,
    checks: Vec<CheckResult>,
    aborted: bool,
    /// Configuration before the scenario changed anything; restored at the end.
    pub original_config: ChargePointConfig,
}

impl ScenarioCtx {
    /// Creates the context; the deadline is the scenario's own timeout.
    pub fn new(
        handle: BoxHandle,
        info: ScenarioInfo,
        tuning: ScenarioTuning,
        probe: Option<Arc<dyn AppProbe>>,
        abort: watch::Receiver<bool>,
    ) -> Self {
        let original_config = handle.snapshot().config;
        let deadline = Instant::now() + Duration::from_secs(info.timeout_s);
        Self {
            handle,
            info,
            tuning,
            probe,
            abort,
            deadline,
            checks: Vec::new(),
            aborted: false,
            original_config,
        }
    }

    // ----- recording -----

    /// Adds a check.
    pub fn push(&mut self, name: &str, outcome: CheckOutcome, detail: impl Into<String>) {
        self.checks.push(CheckResult {
            name: name.to_string(),
            outcome,
            detail: detail.into(),
        });
    }

    /// Adds a passed check.
    pub fn pass(&mut self, name: &str, detail: impl Into<String>) {
        self.push(name, CheckOutcome::Passed, detail);
    }

    /// Adds a failed check.
    pub fn fail(&mut self, name: &str, detail: impl Into<String>) {
        self.push(name, CheckOutcome::Failed, detail);
    }

    /// Adds a skipped check.
    pub fn skip(&mut self, name: &str, detail: impl Into<String>) {
        self.push(name, CheckOutcome::Skipped, detail);
    }

    /// Adds a passed or failed check depending on `ok`.
    pub fn check(
        &mut self,
        ok: bool,
        name: &str,
        pass_detail: impl Into<String>,
        fail_detail: impl Into<String>,
    ) {
        if ok {
            self.pass(name, pass_detail);
        } else {
            self.fail(name, fail_detail);
        }
    }

    /// Adds an already built check.
    pub fn push_result(&mut self, result: CheckResult) {
        self.checks.push(result);
    }

    /// Takes the recorded checks and whether the run was aborted.
    pub fn finish(self) -> (Vec<CheckResult>, bool) {
        (self.checks, self.aborted)
    }

    /// True once the user aborted.
    pub fn is_aborted(&self) -> bool {
        self.aborted || *self.abort.borrow()
    }

    /// Time left until the scenario's own timeout.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    // ----- waiting -----

    /// Waits until `limit` has passed. Returns false when aborted.
    pub async fn sleep(&mut self, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        self.sleep_until(until).await
    }

    async fn sleep_until(&mut self, until: Instant) -> bool {
        let mut abort = self.abort.clone();
        loop {
            if *abort.borrow() {
                self.aborted = true;
                return false;
            }
            tokio::select! {
                () = tokio::time::sleep_until(until) => return true,
                changed = abort.changed() => {
                    if changed.is_err() {
                        tokio::time::sleep_until(until).await;
                        return true;
                    }
                }
            }
        }
    }

    /// Waits until the wall-clock time `at`, capped by the scenario deadline.
    /// Returns `TimedOut` when the deadline comes first.
    pub async fn sleep_until_utc(&mut self, at: DateTime<Utc>) -> Wait<()> {
        let wanted = (at - Utc::now()).to_std().unwrap_or_default();
        if wanted > self.remaining() {
            let remaining = self.remaining();
            return if self.sleep(remaining).await {
                Wait::TimedOut
            } else {
                Wait::Aborted
            };
        }
        if self.sleep(wanted).await {
            Wait::Ready(())
        } else {
            Wait::Aborted
        }
    }

    /// Waits for a box event accepted by `pick`, for at most `within` (and never past the deadline).
    pub async fn wait_event<T>(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        within: Duration,
        mut pick: impl FnMut(&BoxEvent) -> Option<T>,
    ) -> Wait<T> {
        let until = (Instant::now() + within).min(self.deadline);
        let mut abort = self.abort.clone();
        loop {
            if *abort.borrow() {
                self.aborted = true;
                return Wait::Aborted;
            }
            tokio::select! {
                received = rx.recv() => match received {
                    Ok(event) => {
                        if let Some(found) = pick(&event) {
                            return Wait::Ready(found);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Wait::TimedOut,
                },
                () = tokio::time::sleep_until(until) => return Wait::TimedOut,
                changed = abort.changed() => {
                    if changed.is_err() {
                        // The sender is gone, which only happens when the box was removed; treat as time out.
                        return Wait::TimedOut;
                    }
                }
            }
        }
    }

    /// Waits for a frame in the given direction accepted by `pred`.
    pub async fn wait_frame(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        direction: FrameDirection,
        within: Duration,
        mut pred: impl FnMut(&Frame) -> bool,
    ) -> Wait<SeenFrame> {
        self.wait_event(rx, within, |event| {
            let BoxEvent::Frame(entry) = event else {
                return None;
            };
            if entry.direction != direction {
                return None;
            }
            let frame = parse_frame(&entry.raw).ok()?;
            pred(&frame).then_some(SeenFrame {
                at: entry.at,
                frame,
            })
        })
        .await
    }

    /// Like `wait_frame`, but records a failed check with `missing_detail` on time-out.
    /// Returns `None` on time-out or abort.
    pub async fn expect_frame(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        direction: FrameDirection,
        within: Duration,
        pred: impl FnMut(&Frame) -> bool,
        check_name: &str,
        missing_detail: &str,
    ) -> Option<SeenFrame> {
        match self.wait_frame(rx, direction, within, pred).await {
            Wait::Ready(seen) => Some(seen),
            Wait::TimedOut => {
                self.fail(check_name, missing_detail);
                None
            }
            Wait::Aborted => None,
        }
    }

    /// Waits for the CALLRESULT or CALLERROR with this message id.
    pub async fn wait_answer(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        direction: FrameDirection,
        id: &str,
        within: Duration,
    ) -> Wait<SeenFrame> {
        self.wait_frame(rx, direction, within, |frame| {
            matches!(frame, Frame::CallResult { id: i, .. } | Frame::CallError { id: i, .. } if i == id)
        })
        .await
    }

    /// Waits for a snapshot accepted by `pred`.
    pub async fn wait_snapshot(
        &mut self,
        within: Duration,
        mut pred: impl FnMut(&ChargePointSnapshot) -> bool,
    ) -> Wait<ChargePointSnapshot> {
        let until = (Instant::now() + within).min(self.deadline);
        let mut snapshots = self.handle.watch_snapshot();
        let mut abort = self.abort.clone();
        loop {
            let current = snapshots.borrow_and_update().clone();
            if pred(&current) {
                return Wait::Ready(current);
            }
            if *abort.borrow() {
                self.aborted = true;
                return Wait::Aborted;
            }
            tokio::select! {
                changed = snapshots.changed() => {
                    if changed.is_err() {
                        return Wait::TimedOut;
                    }
                }
                () = tokio::time::sleep_until(until) => return Wait::TimedOut,
                changed = abort.changed() => {
                    if changed.is_err() {
                        return Wait::TimedOut;
                    }
                }
            }
        }
    }

    // ----- preparation steps shared by several scenarios -----

    /// Makes sure the box is connected. Records a failed check (and returns false) when it cannot be.
    pub async fn ensure_connected(&mut self) -> bool {
        let snapshot = self.handle.snapshot();
        if matches!(snapshot.connection, ConnectionState::Connected { .. }) {
            return true;
        }
        if let Err(error) = self.handle.connect().await {
            self.fail("Verbindung zur Zentrale", error.to_string());
            return false;
        }
        let within = self.tuning.connect_wait;
        let result = self
            .wait_snapshot(within, |s| {
                matches!(
                    s.connection,
                    ConnectionState::Connected { .. } | ConnectionState::Failed { .. }
                )
            })
            .await;
        match result {
            Wait::Ready(s) => match s.connection {
                ConnectionState::Connected { .. } => true,
                ConnectionState::Failed { reason } => {
                    self.fail("Verbindung zur Zentrale", reason);
                    false
                }
                _ => false,
            },
            Wait::TimedOut => {
                let last = self
                    .handle
                    .snapshot()
                    .last_error
                    .unwrap_or_else(|| "keine Fehlermeldung".to_string());
                self.fail(
                    "Verbindung zur Zentrale",
                    format!(
                        "Die Box wurde innerhalb von {} s nicht verbunden. Letzter Fehler: {last}",
                        within.as_secs()
                    ),
                );
                false
            }
            Wait::Aborted => false,
        }
    }

    /// Makes sure a vehicle is plugged in and the box is charging (transaction running).
    pub async fn ensure_charging(&mut self) -> bool {
        let snapshot = self.handle.snapshot();
        if snapshot.vehicle.is_none() {
            let vehicle = default_vehicle_for(&snapshot.config);
            if let Err(error) = self.handle.plug_in(vehicle).await {
                self.fail("Fahrzeug anstecken", error.to_string());
                return false;
            }
        }
        let within = self.tuning.connect_wait;
        match self
            .wait_snapshot(within, |s| {
                s.status == OcppStatus::Charging && s.transaction_id.is_some()
            })
            .await
        {
            Wait::Ready(_) => true,
            Wait::TimedOut => {
                self.fail(
                    "Ladevorgang gestartet",
                    format!(
                        "Die Box ist nach {} s nicht im Status Charging.",
                        within.as_secs()
                    ),
                );
                false
            }
            Wait::Aborted => false,
        }
    }

    /// Sends a CALL through the box, retrying briefly while the boot handshake is still running.
    pub async fn call(&mut self, action: &str, payload: Value) -> Option<CallOutcome> {
        for _ in 0..30 {
            match self.handle.send_call(action, payload.clone()).await {
                Ok(CallOutcome::NotConnected) => {
                    if !self.sleep(Duration::from_millis(500)).await {
                        return None;
                    }
                }
                Ok(outcome) => return Some(outcome),
                Err(error) => {
                    self.fail(&format!("{action} senden"), error.to_string());
                    return None;
                }
            }
        }
        self.fail(
            &format!("{action} senden"),
            "Die Box war nicht angemeldet, der Aufruf konnte nicht gesendet werden.",
        );
        None
    }

    /// Collects every SetChargingProfile that arrives: waits up to `first` for the first one and up to
    /// `extra` for each further one. Each is paired with the answer the box gave.
    pub async fn collect_profiles(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        first: Duration,
        extra: Duration,
    ) -> Vec<ReceivedProfile> {
        let mut found = Vec::new();
        let mut within = first;
        loop {
            let waited = self
                .wait_frame(
                    rx,
                    FrameDirection::In,
                    within,
                    |f| matches!(f, Frame::Call { action, .. } if action == "SetChargingProfile"),
                )
                .await;
            let Wait::Ready(seen) = waited else {
                return found;
            };
            let Frame::Call { id, payload, .. } = seen.frame else {
                return found;
            };
            let answer = self
                .wait_answer(rx, FrameDirection::Out, &id, Duration::from_secs(5))
                .await;
            let status = match answer {
                Wait::Ready(SeenFrame {
                    frame: Frame::CallResult { payload, .. },
                    ..
                }) => super::rules::response_status(&payload).map(str::to_string),
                _ => None,
            };
            found.push(ReceivedProfile {
                received_at: seen.at,
                payload,
                answer: status,
            });
            within = extra;
        }
    }
}

/// A SetChargingProfile the box received and what it answered.
#[derive(Debug, Clone)]
pub struct ReceivedProfile {
    /// When it arrived.
    pub received_at: DateTime<Utc>,
    /// The request payload.
    pub payload: Value,
    /// The box's answer status (`Accepted`, `Rejected`), `None` if no answer was seen.
    pub answer: Option<String>,
}

/// The vehicle scenarios plug in: the default car with the phases of the box.
pub fn default_vehicle_for(config: &ChargePointConfig) -> VehicleConfig {
    VehicleConfig {
        phases: config.phases,
        ..VehicleConfig::default()
    }
}

/// Records a call as the scenarios compare them.
pub fn received_call(frame: &Frame) -> Option<ReceivedCall> {
    match frame {
        Frame::Call {
            action, payload, ..
        } => Some(ReceivedCall {
            action: action.clone(),
            payload: payload.clone(),
        }),
        _ => None,
    }
}

/// An empty payload.
pub fn empty_payload() -> Value {
    json!({})
}
