//! Shared machinery of the scenarios: waiting for frames and state, recording checks, abort handling.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use super::rules::ReceivedCall;
use super::AppProbe;
use crate::error::SimError;
use crate::handle::{BoxEvent, BoxHandle, CallOutcome};
use crate::model::{
    ChargePointConfig, ChargePointSnapshot, CheckOutcome, CheckResult, ConnectionState,
    FrameDirection, OcppStatus, ScenarioInfo, VehicleConfig,
};
use crate::ocpp::frames::{parse_frame, Frame};

/// How long the answer to a call that was already seen is awaited.
pub const ANSWER_WAIT: Duration = Duration::from_secs(5);

/// Poll step while waiting for an answer that is expected in the frame log.
const LOG_POLL: Duration = Duration::from_millis(100);

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
    /// Time the backend's 15-minute job needs after a quarter ended (3 min after the quarter plus margin);
    /// S11 waits this long before it reads the quarter value.
    pub backend_job_wait: Duration,
    /// Optional earlier end of S11 than the scenario timeout (used by tests; `None` waits for the
    /// quarter-hour value up to the timeout).
    pub app_max_wait: Option<Duration>,
    /// Added to a connect cooldown before the scenario tries again.
    pub cooldown_margin: Duration,
    /// Replaces the scenario's own timeout (tests); `None` uses the catalog value.
    pub timeout_override: Option<Duration>,
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
            backend_job_wait: Duration::from_secs(4 * 60),
            app_max_wait: None,
            cooldown_margin: Duration::from_secs(1),
            timeout_override: None,
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
    /// When the run started; frames logged before are not part of it.
    pub started_at: DateTime<Utc>,
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
        let timeout = tuning
            .timeout_override
            .unwrap_or_else(|| Duration::from_secs(info.timeout_s));
        let deadline = Instant::now() + timeout;
        Self {
            handle,
            info,
            tuning,
            probe,
            abort,
            deadline,
            started_at: Utc::now(),
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

    /// Waits until `limit` has passed, but never past the scenario deadline. Returns false when aborted.
    pub async fn sleep(&mut self, limit: Duration) -> bool {
        let until = (Instant::now() + limit).min(self.deadline);
        self.sleep_until(until).await
    }

    /// Gives the scenario time for the work after its checks are decided (restoring the connection): the
    /// deadline moves out to cover a connect pause plus the time to connect.
    pub fn extend_deadline_for_restore(&mut self) {
        let extra = self.handle.timings().cooldown
            + self.tuning.cooldown_margin
            + self.tuning.connect_wait
            + self.handle.timings().connect_timeout;
        self.deadline = self.deadline.max(Instant::now() + extra);
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
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        // Events were lost, so a "missing frame" verdict could be wrong; say so in the report.
                        self.fail(
                            "Ereignisse verpasst",
                            format!(
                                "Das Szenario hat {missed} Ereignisse der Wallbox verpasst; das Ergebnis kann unvollständig sein."
                            ),
                        );
                    }
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

    /// Like `wait_answer`, but records a failed check with `missing_detail` on time-out.
    /// Returns `None` on time-out or abort.
    pub async fn expect_answer(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        direction: FrameDirection,
        id: &str,
        within: Duration,
        check_name: &str,
        missing_detail: &str,
    ) -> Option<SeenFrame> {
        match self.wait_answer(rx, direction, id, within).await {
            Wait::Ready(seen) => Some(seen),
            Wait::TimedOut => {
                self.fail(check_name, missing_detail);
                None
            }
            Wait::Aborted => None,
        }
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

    /// True while the box has an open connection.
    pub fn is_connected(&self) -> bool {
        matches!(
            self.handle.snapshot().connection,
            ConnectionState::Connected { .. }
        )
    }

    /// Opens the connection. After a rejected login (HTTP 401/429) the box refuses to connect for a while;
    /// this waits out that pause once and tries again, because a scenario that needs the connection has no
    /// better choice.
    pub async fn connect_box(&mut self) -> Result<(), SimError> {
        match self.handle.connect().await {
            Err(SimError::ConnectCooldown { remaining_s }) => {
                let pause = Duration::from_secs(remaining_s) + self.tuning.cooldown_margin;
                if !self.sleep(pause).await {
                    return Err(SimError::ConnectCooldown { remaining_s });
                }
                self.handle.connect().await
            }
            other => other,
        }
    }

    /// Starts a connection, or restarts it when the box is already connected (so the BootNotification and
    /// the calls after it happen after the caller subscribed).
    pub async fn connect_or_reboot(&mut self) -> Result<(), SimError> {
        if self.is_connected() {
            self.handle.reboot().await
        } else {
            self.connect_box().await
        }
    }

    /// Makes sure the box is connected. Records a failed check (and returns false) when it cannot be.
    pub async fn ensure_connected(&mut self) -> bool {
        match self.connect_and_wait().await {
            Ok(connected) => connected,
            Err(text) => {
                self.fail("Verbindung zur Zentrale", text);
                false
            }
        }
    }

    /// Connects and waits for the connection. `Ok(false)` means aborted, `Err` carries the German reason.
    pub async fn connect_and_wait(&mut self) -> Result<bool, String> {
        if self.is_connected() {
            return Ok(true);
        }
        if let Err(error) = self.connect_box().await {
            return Err(error.to_string());
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
                ConnectionState::Connected { .. } => Ok(true),
                ConnectionState::Failed { reason } => Err(reason),
                _ => Ok(false),
            },
            Wait::TimedOut => {
                let last = self
                    .handle
                    .snapshot()
                    .last_error
                    .unwrap_or_else(|| "keine Fehlermeldung".to_string());
                Err(format!(
                    "Die Wallbox wurde innerhalb von {} s nicht verbunden. Letzter Fehler: {last}",
                    within.as_secs()
                ))
            }
            Wait::Aborted => Ok(false),
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
                        "Die Wallbox ist nach {} s nicht im Status Charging.",
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
            "Die Wallbox war nicht angemeldet, der Aufruf konnte nicht gesendet werden.",
        );
        None
    }

    /// Collects every SetChargingProfile of this run: those already in the frame log (they can arrive right after
    /// StartTransaction, before the scenario starts to wait) and those that arrive later. Waits up to `first` for
    /// the first one and up to `extra` for each further one. Each is paired with the answer the box gave.
    pub async fn collect_profiles(
        &mut self,
        rx: &mut broadcast::Receiver<BoxEvent>,
        first: Duration,
        extra: Duration,
    ) -> Vec<ReceivedProfile> {
        let mut found = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        for (id, received_at, payload) in self.logged_profile_calls() {
            let answer = self.logged_answer(&id).await;
            seen_ids.insert(id);
            found.push(ReceivedProfile {
                received_at,
                payload,
                answer,
            });
        }
        let mut within = if found.is_empty() { first } else { extra };
        loop {
            let waited = self
                .wait_frame(rx, FrameDirection::In, within, |f| {
                    matches!(f, Frame::Call { id, action, .. }
                        if action == "SetChargingProfile" && !seen_ids.contains(id))
                })
                .await;
            let Wait::Ready(seen) = waited else {
                return found;
            };
            let Frame::Call { id, payload, .. } = seen.frame else {
                return found;
            };
            let answer = self
                .wait_answer(rx, FrameDirection::Out, &id, ANSWER_WAIT)
                .await;
            let status = match answer {
                Wait::Ready(answer) => answer_status(&answer.frame),
                _ => None,
            };
            seen_ids.insert(id);
            found.push(ReceivedProfile {
                received_at: seen.at,
                payload,
                answer: status,
            });
            within = extra;
        }
    }

    /// SetChargingProfile calls the box received since this run started: (message id, time, payload).
    fn logged_profile_calls(&self) -> Vec<(String, DateTime<Utc>, Value)> {
        self.handle
            .log_entries()
            .into_iter()
            .filter(|e| e.direction == FrameDirection::In && e.at >= self.started_at)
            .filter_map(|e| match parse_frame(&e.raw) {
                Ok(Frame::Call {
                    id,
                    action,
                    payload,
                }) if action == "SetChargingProfile" => Some((id, e.at, payload)),
                _ => None,
            })
            .collect()
    }

    /// The status the box answered to call `id`, read from the frame log (the answer may still be on its way).
    async fn logged_answer(&mut self, id: &str) -> Option<String> {
        let until = Instant::now() + ANSWER_WAIT;
        loop {
            let found = self.handle.log_entries().into_iter().find_map(|e| {
                if e.direction != FrameDirection::Out {
                    return None;
                }
                match parse_frame(&e.raw) {
                    Ok(Frame::CallResult { id: i, payload }) if i == id => {
                        Some(super::rules::response_status(&payload).map(str::to_string))
                    }
                    _ => None,
                }
            });
            if let Some(status) = found {
                return status;
            }
            if Instant::now() >= until || !self.sleep(LOG_POLL).await {
                return None;
            }
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

/// The `status` of a CALLRESULT (`Accepted`, `Rejected`, ...); `None` for anything else.
pub fn answer_status(frame: &Frame) -> Option<String> {
    match frame {
        Frame::CallResult { payload, .. } => {
            super::rules::response_status(payload).map(str::to_string)
        }
        _ => None,
    }
}
