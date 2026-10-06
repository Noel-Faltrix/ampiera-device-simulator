//! The outside view of a running box actor: commands in, events and snapshots out.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::error::SimError;
use crate::gate::ConnectGate;
use crate::model::{ChargePointConfig, ChargePointSnapshot, FrameLogEntry, VehicleConfig};
use crate::ocpp::client::Secret;

/// Frames kept in memory per box (CONTRACT: last 2000 entries).
pub const FRAME_LOG_CAPACITY: usize = 2000;

/// Capacity of the per-box event channel. Scenarios drain it continuously; a slow receiver only loses old
/// frames (and is told so), it never blocks the box.
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// All time constants of the actor. The defaults are the production values; tests shorten them so a run
/// takes seconds instead of minutes.
#[derive(Debug, Clone)]
pub struct Timings {
    /// Interval of the vehicle/meter/limit simulation.
    pub tick: Duration,
    /// Minimum time between `charge_point_updated` events of one box (CONTRACT: at most 4 per second).
    pub emit_interval: Duration,
    /// Time to wait for the answer to one CALL (OCPP-J practice and the backend's own value).
    pub call_timeout: Duration,
    /// First reconnect delay.
    pub backoff_base: Duration,
    /// Longest reconnect delay.
    pub backoff_max: Duration,
    /// Time allowed for the TCP/TLS/websocket handshake.
    pub connect_timeout: Duration,
    /// A connection must have been up this long after an accepted BootNotification before the backoff
    /// counter resets; otherwise a central system that accepts and immediately closes would be hit once a second.
    pub stable_after: Duration,
    /// Pause before connecting again after HTTP 401 or 429 (protects the central system's failure counter).
    pub cooldown: Duration,
    /// Pause before a failed StartTransaction is tried again.
    pub start_retry: Duration,
    /// Shortest accepted heartbeat and MeterValues interval; the central system must not be able to make the
    /// box send every second.
    pub min_interval: Duration,
    /// Window in which lost connections are counted for flap detection.
    pub flap_window: Duration,
    /// More lost connections than this within the window put the box into `failed`.
    pub flap_max_losses: usize,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            emit_interval: Duration::from_millis(250),
            call_timeout: Duration::from_secs(30),
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(15),
            stable_after: Duration::from_secs(60),
            cooldown: Duration::from_secs(60),
            start_retry: Duration::from_secs(30),
            min_interval: Duration::from_secs(5),
            flap_window: Duration::from_secs(300),
            flap_max_losses: 5,
        }
    }
}

/// Answer to a CALL that a scenario sent through the box.
#[derive(Debug, Clone, PartialEq)]
pub enum CallOutcome {
    /// CALLRESULT payload.
    Result(Value),
    /// CALLERROR.
    Error {
        /// OCPP error code as received.
        code: String,
        /// Description as received.
        description: String,
    },
    /// No answer within the call timeout.
    Timeout,
    /// The box was not connected (or the connection dropped before the answer).
    NotConnected,
}

/// Result channel of a command.
pub type CommandReply = oneshot::Sender<Result<(), SimError>>;

/// Instructions for the actor.
#[derive(Debug)]
pub enum Command {
    /// Open the connection (and keep it open).
    Connect(CommandReply),
    /// Close the connection and stay offline.
    Disconnect(CommandReply),
    /// Plug a vehicle in.
    PlugIn(VehicleConfig, CommandReply),
    /// Unplug the vehicle.
    Unplug(CommandReply),
    /// Close and reconnect with a new BootNotification.
    Reboot(CommandReply),
    /// Replace the configuration (scenarios change e.g. `supports_soc` temporarily).
    SetConfig(Box<ChargePointConfig>, CommandReply),
    /// Replace the password used for the next connection attempt.
    SetPassword(Secret, CommandReply),
    /// Cut the connection without a close handshake and keep reconnecting, but not before `block_until`.
    DropConnection {
        /// Earliest time of the next attempt.
        block_until: Option<DateTime<Utc>>,
        /// Result channel.
        reply: CommandReply,
    },
    /// Send an arbitrary CALL and report the answer.
    SendCall {
        /// OCPP action.
        action: String,
        /// Payload.
        payload: Value,
        /// Receives the outcome.
        reply: oneshot::Sender<CallOutcome>,
    },
    /// End the actor task.
    Shutdown,
}

/// Something that happened on a box, for scenarios that wait for it.
#[derive(Debug, Clone)]
pub enum BoxEvent {
    /// A frame went out or came in.
    Frame(FrameLogEntry),
    /// The connection ended (closed by the central system or lost).
    Closed {
        /// WebSocket close code; `None` when the connection dropped without a close frame.
        code: Option<u16>,
        /// Close reason text.
        reason: String,
    },
    /// A connection attempt started.
    ConnectAttempt,
    /// A connection attempt failed.
    ConnectFailed {
        /// HTTP status of the handshake response, if there was one.
        http_status: Option<u16>,
        /// Start of the response body, if there was one.
        body: Option<String>,
    },
}

/// Cheap-to-clone handle to one box.
#[derive(Clone)]
pub struct BoxHandle {
    /// Local id of the box.
    pub id: String,
    pub(crate) commands: mpsc::Sender<Command>,
    pub(crate) events: broadcast::Sender<BoxEvent>,
    pub(crate) snapshot: watch::Receiver<ChargePointSnapshot>,
    pub(crate) log: Arc<Mutex<VecDeque<FrameLogEntry>>>,
    pub(crate) password: Secret,
    pub(crate) gate: Arc<ConnectGate>,
    pub(crate) timings: Timings,
}

impl BoxHandle {
    /// Latest snapshot, not throttled.
    pub fn snapshot(&self) -> ChargePointSnapshot {
        self.snapshot.borrow().clone()
    }

    /// A receiver that follows snapshot changes.
    pub fn watch_snapshot(&self) -> watch::Receiver<ChargePointSnapshot> {
        self.snapshot.clone()
    }

    /// Subscribes to frames and connection events. Subscribe before triggering the action you want to observe.
    pub fn subscribe(&self) -> broadcast::Receiver<BoxEvent> {
        self.events.subscribe()
    }

    /// The time constants this box runs with (scenarios use the connect timeout for their own connections).
    pub fn timings(&self) -> &Timings {
        &self.timings
    }

    /// The password the box was created with (needed by scenarios that open a second connection).
    pub fn password(&self) -> &Secret {
        &self.password
    }

    /// Copy of the in-memory frame log.
    pub fn log_entries(&self) -> Vec<FrameLogEntry> {
        self.log
            .lock()
            .map(|log| log.iter().cloned().collect())
            .unwrap_or_default()
    }

    async fn request(&self, build: impl FnOnce(CommandReply) -> Command) -> Result<(), SimError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(build(tx))
            .await
            .map_err(|_| SimError::BoxStopped)?;
        rx.await.map_err(|_| SimError::BoxStopped)?
    }

    /// Opens the connection. Towards the Produktivserver this takes one of the three allowed slots; the slot is
    /// reserved before the command is sent, so parallel connects cannot exceed the limit.
    pub async fn connect(&self) -> Result<(), SimError> {
        let _slot = self.gate.reserve(&self.id)?;
        self.request(Command::Connect).await
    }

    /// Whether this box may open one more connection next to its own (a live box may not when the limit of
    /// live connections is used up). Scenarios that open their own socket must ask first.
    pub fn check_extra_connection(&self) -> Result<(), SimError> {
        self.gate.check_extra_connection(&self.id)
    }

    /// Closes the connection.
    pub async fn disconnect(&self) -> Result<(), SimError> {
        self.request(Command::Disconnect).await
    }

    /// Plugs a vehicle in.
    pub async fn plug_in(&self, vehicle: VehicleConfig) -> Result<(), SimError> {
        self.request(|reply| Command::PlugIn(vehicle, reply)).await
    }

    /// Unplugs the vehicle.
    pub async fn unplug(&self) -> Result<(), SimError> {
        self.request(Command::Unplug).await
    }

    /// Reconnects with a new BootNotification.
    pub async fn reboot(&self) -> Result<(), SimError> {
        self.request(Command::Reboot).await
    }

    /// Replaces the configuration.
    pub async fn set_config(&self, config: ChargePointConfig) -> Result<(), SimError> {
        self.request(|reply| Command::SetConfig(Box::new(config), reply))
            .await
    }

    /// Replaces the password for the next connection attempt.
    pub async fn set_password(&self, password: Secret) -> Result<(), SimError> {
        self.request(|reply| Command::SetPassword(password, reply))
            .await
    }

    /// Cuts the connection and blocks reconnecting until `block_until`.
    pub async fn drop_connection(
        &self,
        block_until: Option<DateTime<Utc>>,
    ) -> Result<(), SimError> {
        self.request(|reply| Command::DropConnection { block_until, reply })
            .await
    }

    /// Sends a CALL through the box and waits for the answer (at most the call timeout plus queueing time).
    pub async fn send_call(&self, action: &str, payload: Value) -> Result<CallOutcome, SimError> {
        let (tx, rx) = oneshot::channel();
        let command = Command::SendCall {
            action: action.to_string(),
            payload,
            reply: tx,
        };
        self.commands
            .send(command)
            .await
            .map_err(|_| SimError::BoxStopped)?;
        rx.await.map_err(|_| SimError::BoxStopped)
    }

    /// Stops the actor.
    pub async fn shutdown(&self) {
        // The actor may already be gone; there is nothing left to stop in that case.
        let _ = self.commands.send(Command::Shutdown).await;
    }
}
