//! The outside view of a running box actor: commands in, events and snapshots out.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::error::SimError;
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
    /// The central system closed the socket.
    Closed {
        /// WebSocket close code, if one was sent.
        code: Option<u16>,
        /// Close reason text.
        reason: String,
    },
    /// A connection attempt started.
    ConnectAttempt,
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

    /// The password the box was created with (needed by scenarios that open a second socket).
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

    /// Opens the connection.
    pub async fn connect(&self) -> Result<(), SimError> {
        self.request(Command::Connect).await
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
