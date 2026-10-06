//! The per-box actor: owns the box state and the websocket, speaks OCPP-J with the central system.
//!
//! One task per box. It reacts to commands, incoming frames, a connect attempt in flight, and timers
//! (simulation tick, heartbeat, MeterValues, call timeout, reconnect). Only one CALL per direction is
//! outstanding at any time.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::client::{self, backoff_delay, ConnectFailure, Secret, WsStream};
use crate::charge_point::state::{BoxState, StopReason};
use crate::charge_point::status::Phase;
use crate::error::SimError;
use crate::handle::{
    BoxEvent, BoxHandle, CallOutcome, Command, CommandReply, Timings, EVENT_CHANNEL_CAPACITY,
};
use crate::model::{
    ChargePointConfig, ChargePointSnapshot, ConnectionState, FrameLogEntry, OcppStatus,
};
use crate::policy;
use crate::throttle::Throttle;
use crate::EventSink;

/// Heartbeat interval used when the central system sends none (or 0). The backend always answers 300 s.
const FALLBACK_HEARTBEAT_S: u64 = 300;

/// Time the actor waits for a graceful websocket close before dropping the socket.
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

mod traffic;

type ConnectFuture = Pin<Box<dyn Future<Output = Result<WsStream, ConnectFailure>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Boot,
    Heartbeat,
    Status,
    StartTransaction,
    StopTransaction,
    Meter,
    Custom,
}

struct OutCall {
    kind: CallKind,
    action: String,
    payload: Value,
    reply: Option<oneshot::Sender<CallOutcome>>,
    /// Kept across a lost connection: a StopTransaction must reach the central system eventually.
    keep_offline: bool,
}

impl OutCall {
    fn new(kind: CallKind, action: &str, payload: Value) -> Self {
        Self {
            kind,
            action: action.to_string(),
            payload,
            reply: None,
            keep_offline: false,
        }
    }
}

struct Pending {
    id: String,
    call: OutCall,
    deadline: Instant,
}

/// Delay until the next reconnect attempt: exponential backoff, but never earlier than `blocked_until`.
/// The block models a central system that is unreachable for a defined time (scenario S4).
pub fn reconnect_delay(
    attempt: u32,
    timings: &Timings,
    blocked_until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> std::time::Duration {
    let backoff = backoff_delay(attempt, timings.backoff_base, timings.backoff_max);
    let blocked = blocked_until
        .and_then(|until| (until - now).to_std().ok())
        .unwrap_or_default();
    backoff.max(blocked)
}

/// Starts the actor task for one box and returns its handle.
pub fn spawn(
    id: String,
    config: ChargePointConfig,
    password: Secret,
    sink: Arc<dyn EventSink>,
    timings: Timings,
) -> (BoxHandle, JoinHandle<()>) {
    let (commands_tx, commands_rx) = mpsc::channel(64);
    let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
    let state = BoxState::new(id.clone(), config);
    let (snapshot_tx, snapshot_rx) = watch::channel(state.snapshot());
    let log = Arc::new(Mutex::new(VecDeque::new()));
    let handle = BoxHandle {
        id,
        commands: commands_tx,
        events: events_tx.clone(),
        snapshot: snapshot_rx,
        log: log.clone(),
        password: password.clone(),
    };
    let now = Instant::now();
    let actor = Actor {
        state,
        password,
        throttle: Throttle::new(timings.emit_interval),
        timings,
        sink,
        events: events_tx,
        snapshot_tx,
        log,
        rx: commands_rx,
        ws: None,
        connecting: None,
        reconnect_at: None,
        attempt: 0,
        want_online: false,
        blocked_until: None,
        queue: VecDeque::new(),
        pending: None,
        boot_accepted: false,
        boot_retry_at: None,
        reported_status: None,
        tx_start_requested: false,
        next_tick: now,
        last_tick: now,
        next_heartbeat: None,
        next_meter: None,
        meter_key: (0, false),
        last_published: None,
    };
    let task = tokio::spawn(actor.run());
    (handle, task)
}

struct Actor {
    state: BoxState,
    password: Secret,
    timings: Timings,
    sink: Arc<dyn EventSink>,
    events: broadcast::Sender<BoxEvent>,
    snapshot_tx: watch::Sender<ChargePointSnapshot>,
    log: Arc<Mutex<VecDeque<FrameLogEntry>>>,
    rx: mpsc::Receiver<Command>,
    ws: Option<WsStream>,
    connecting: Option<ConnectFuture>,
    reconnect_at: Option<Instant>,
    attempt: u32,
    want_online: bool,
    blocked_until: Option<DateTime<Utc>>,
    queue: VecDeque<OutCall>,
    pending: Option<Pending>,
    boot_accepted: bool,
    boot_retry_at: Option<Instant>,
    reported_status: Option<OcppStatus>,
    tx_start_requested: bool,
    next_tick: Instant,
    last_tick: Instant,
    next_heartbeat: Option<Instant>,
    next_meter: Option<Instant>,
    meter_key: (u32, bool),
    throttle: Throttle,
    last_published: Option<ChargePointSnapshot>,
}

async fn next_message(ws: &mut Option<WsStream>) -> Option<Result<Message, WsError>> {
    match ws {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

async fn poll_connect(connecting: &mut Option<ConnectFuture>) -> Result<WsStream, ConnectFailure> {
    match connecting {
        Some(future) => future.await,
        None => std::future::pending().await,
    }
}

impl Actor {
    async fn run(mut self) {
        loop {
            let wake = self.next_wake();
            tokio::select! {
                command = self.rx.recv() => match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => self.handle_command(command).await,
                },
                message = next_message(&mut self.ws), if self.ws.is_some() => {
                    self.handle_ws_message(message).await;
                }
                result = poll_connect(&mut self.connecting), if self.connecting.is_some() => {
                    self.handle_connect_result(result).await;
                }
                () = tokio::time::sleep_until(wake) => {}
            }
            self.on_timers().await;
            self.after_event().await;
        }
        self.close_socket().await;
        self.fail_open_calls();
    }

    // ----- time -----

    fn next_wake(&self) -> Instant {
        let mut wake = self.next_tick;
        let candidates = [
            self.next_heartbeat,
            self.next_meter,
            self.pending.as_ref().map(|p| p.deadline),
            self.reconnect_at,
            self.boot_retry_at,
            self.throttle.next_emission().map(Instant::from_std),
        ];
        for at in candidates.into_iter().flatten() {
            wake = wake.min(at);
        }
        wake
    }

    fn box_time(&self) -> DateTime<Utc> {
        Utc::now() + ChronoDuration::seconds(self.state.config.clock_offset_s)
    }

    async fn on_timers(&mut self) {
        let now = Instant::now();
        if now >= self.next_tick {
            let dt = now.duration_since(self.last_tick).as_secs_f64();
            self.last_tick = now;
            self.next_tick = now + self.timings.tick;
            self.state.tick(Utc::now(), dt);
        }
        self.update_meter_schedule(now);
        if self.next_heartbeat.is_some_and(|at| now >= at) {
            self.fire_heartbeat(now);
        }
        if self.next_meter.is_some_and(|at| now >= at) {
            self.fire_meter(now);
        }
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            self.on_call_timeout().await;
        }
        if self.reconnect_at.is_some_and(|at| now >= at) {
            self.start_connect();
        }
        if self.boot_retry_at.is_some_and(|at| now >= at) {
            self.boot_retry_at = None;
            if self.ws.is_some() {
                self.queue_boot();
            }
        }
    }

    fn update_meter_schedule(&mut self, now: Instant) {
        let key = (
            self.state.meter_interval_s,
            self.state.phase() == Phase::InTransaction,
        );
        if key == self.meter_key {
            return;
        }
        self.meter_key = key;
        self.next_meter = match key {
            (seconds, true) if seconds > 0 => {
                Some(now + std::time::Duration::from_secs(u64::from(seconds)))
            }
            _ => None,
        };
    }

    fn fire_heartbeat(&mut self, now: Instant) {
        let interval = self
            .state
            .heartbeat_interval_s
            .map_or(FALLBACK_HEARTBEAT_S, u64::from);
        self.next_heartbeat = Some(now + std::time::Duration::from_secs(interval));
        if self.boot_accepted && !self.queue.iter().any(|c| c.kind == CallKind::Heartbeat) {
            self.queue
                .push_back(OutCall::new(CallKind::Heartbeat, "Heartbeat", json!({})));
        }
    }

    fn fire_meter(&mut self, now: Instant) {
        let seconds = u64::from(self.state.meter_interval_s.max(1));
        self.next_meter = Some(now + std::time::Duration::from_secs(seconds));
        // Offline samples are dropped: the simulator does not buffer them (documented limitation).
        if self.boot_accepted && !self.queue.iter().any(|c| c.kind == CallKind::Meter) {
            let call = self.meter_call();
            self.queue.push_back(call);
        }
    }

    // ----- commands -----

    async fn handle_command(&mut self, command: Command) {
        match command {
            Command::Connect(reply) => {
                self.cmd_connect();
                answer(reply, Ok(()));
            }
            Command::Disconnect(reply) => {
                self.cmd_disconnect().await;
                answer(reply, Ok(()));
            }
            Command::PlugIn(vehicle, reply) => {
                let result = policy::validate_vehicle(&vehicle)
                    .and_then(|()| self.state.plug_in(vehicle, Utc::now()));
                if result.is_ok() {
                    self.tx_start_requested = false;
                }
                answer(reply, result);
            }
            Command::Unplug(reply) => {
                let result = self.cmd_unplug();
                answer(reply, result);
            }
            Command::Reboot(reply) => {
                let result = if self.ws.is_some() {
                    self.reconnect_now().await;
                    Ok(())
                } else {
                    Err(SimError::NotConnected)
                };
                answer(reply, result);
            }
            Command::SetConfig(config, reply) => {
                let result = policy::validate_box_values(&config);
                if result.is_ok() {
                    self.state.config = *config;
                    self.state.refresh(Utc::now());
                }
                answer(reply, result);
            }
            Command::SetPassword(password, reply) => {
                self.password = password;
                answer(reply, Ok(()));
            }
            Command::DropConnection { block_until, reply } => {
                self.cmd_drop_connection(block_until);
                answer(reply, Ok(()));
            }
            Command::SendCall {
                action,
                payload,
                reply,
            } => self.cmd_send_call(action, payload, reply),
            Command::Shutdown => {}
        }
    }

    fn cmd_connect(&mut self) {
        if self.ws.is_some() || self.connecting.is_some() {
            return;
        }
        self.want_online = true;
        self.attempt = 0;
        self.blocked_until = None;
        self.start_connect();
    }

    async fn cmd_disconnect(&mut self) {
        self.want_online = false;
        self.reconnect_at = None;
        self.connecting = None;
        self.close_socket().await;
        self.reset_connection_state();
        self.state.connection = ConnectionState::Disconnected;
    }

    fn cmd_unplug(&mut self) -> Result<(), SimError> {
        let ended = self.state.unplug(Utc::now())?;
        self.tx_start_requested = false;
        if let Some(ended) = ended {
            self.queue_stop(&ended, StopReason::EVDisconnected);
            if self.ws.is_none() {
                // The StopTransaction waits in the queue; the connector itself is free right now.
                self.state.finish_if_unplugged();
            }
        }
        Ok(())
    }

    fn cmd_drop_connection(&mut self, block_until: Option<DateTime<Utc>>) {
        self.blocked_until = block_until;
        self.want_online = true;
        self.connecting = None;
        self.ws = None;
        self.on_connection_lost(Some(
            "Die Verbindung wurde absichtlich getrennt.".to_string(),
        ));
    }

    fn cmd_send_call(
        &mut self,
        action: String,
        payload: Value,
        reply: oneshot::Sender<CallOutcome>,
    ) {
        if self.ws.is_none() || !self.boot_accepted {
            // The receiver may have given up already; nothing to do then.
            let _ = reply.send(CallOutcome::NotConnected);
            return;
        }
        let mut call = OutCall::new(CallKind::Custom, &action, payload);
        call.reply = Some(reply);
        self.queue.push_back(call);
    }

    // ----- connection -----

    fn start_connect(&mut self) {
        self.reconnect_at = None;
        self.blocked_until = None;
        self.state.connection = ConnectionState::Connecting;
        // No subscriber is not an error: scenarios only listen while they run.
        let _ = self.events.send(BoxEvent::ConnectAttempt);
        let base_url = self.state.config.base_url.clone();
        let identity = self.state.config.identity.clone();
        let password = self.password.clone();
        let timeout = self.timings.connect_timeout;
        self.connecting = Some(Box::pin(async move {
            client::connect(&base_url, &identity, &password, timeout).await
        }));
    }

    async fn handle_connect_result(&mut self, result: Result<WsStream, ConnectFailure>) {
        self.connecting = None;
        match result {
            Ok(ws) => {
                self.ws = Some(ws);
                self.attempt = 0;
                self.state.last_error = None;
                self.state.connection = ConnectionState::Connected { since: Utc::now() };
                self.queue_boot();
            }
            Err(failure) => {
                self.state.last_error = Some(failure.reason.clone());
                if failure.retryable && self.want_online {
                    self.schedule_reconnect();
                } else {
                    self.want_online = false;
                    self.state.connection = ConnectionState::Failed {
                        reason: failure.reason,
                    };
                }
            }
        }
    }

    fn schedule_reconnect(&mut self) {
        self.attempt += 1;
        let now = Utc::now();
        let delay = reconnect_delay(self.attempt, &self.timings, self.blocked_until, now);
        self.reconnect_at = Some(Instant::now() + delay);
        let wait = ChronoDuration::from_std(delay).unwrap_or_else(|_| ChronoDuration::seconds(0));
        self.state.connection = ConnectionState::Reconnecting {
            attempt: self.attempt,
            next_attempt_at: now + wait,
        };
    }

    /// Everything tied to one socket is reset; calls that must survive stay queued.
    fn reset_connection_state(&mut self) {
        self.boot_accepted = false;
        self.boot_retry_at = None;
        self.next_heartbeat = None;
        self.reported_status = None;
        self.tx_start_requested = false;
        if let Some(pending) = self.pending.take() {
            if pending.call.keep_offline {
                self.queue.push_front(pending.call);
            } else if let Some(reply) = pending.call.reply {
                let _ = reply.send(CallOutcome::NotConnected);
            }
        }
        let (keep, drop): (VecDeque<OutCall>, VecDeque<OutCall>) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|c| c.keep_offline);
        self.queue = keep;
        for call in drop {
            if let Some(reply) = call.reply {
                let _ = reply.send(CallOutcome::NotConnected);
            }
        }
    }

    fn on_connection_lost(&mut self, reason: Option<String>) {
        self.ws = None;
        self.reset_connection_state();
        if let Some(reason) = reason {
            self.state.last_error = Some(reason);
        }
        if self.want_online {
            self.schedule_reconnect();
        } else {
            self.state.connection = ConnectionState::Disconnected;
        }
    }

    async fn close_socket(&mut self) {
        if let Some(mut ws) = self.ws.take() {
            // A slow or dead peer must not hold the actor: give the close handshake one second.
            let _ = tokio::time::timeout(CLOSE_GRACE, ws.close(None)).await;
        }
    }

    async fn reconnect_now(&mut self) {
        self.close_socket().await;
        self.reset_connection_state();
        self.want_online = true;
        self.attempt = 0;
        self.start_connect();
    }
}

fn answer(reply: CommandReply, result: Result<(), SimError>) {
    // The caller may have stopped waiting; the command was applied either way.
    let _ = reply.send(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }

    #[test]
    fn reconnect_delay_follows_backoff_without_a_block() {
        let t = Timings::default();
        assert_eq!(reconnect_delay(1, &t, None, now()).as_secs(), 1);
        assert_eq!(reconnect_delay(3, &t, None, now()).as_secs(), 4);
        assert_eq!(reconnect_delay(30, &t, None, now()).as_secs(), 60);
    }

    #[test]
    fn a_block_postpones_the_attempt_until_its_end() {
        let t = Timings::default();
        let until = now() + ChronoDuration::seconds(600);
        assert_eq!(reconnect_delay(1, &t, Some(until), now()).as_secs(), 600);
        let later = now() + ChronoDuration::seconds(599);
        assert_eq!(
            reconnect_delay(1, &t, Some(until), later).as_secs(),
            1,
            "backoff wins near the end"
        );
    }

    #[test]
    fn counter_check_an_expired_block_changes_nothing() {
        let t = Timings::default();
        let past = now() - ChronoDuration::seconds(10);
        assert_eq!(reconnect_delay(2, &t, Some(past), now()).as_secs(), 2);
    }
}
