#![warn(missing_docs)]

//! Core of the desktop OCPP 1.6J charge point simulator.
//!
//! `Simulator` owns the simulated boxes; each box runs as an actor task that speaks OCPP-J with the central
//! system. The pure rules (profiles, vehicle, status, answers to central system calls) live in
//! `charge_point` and are tested without any network.

pub mod charge_point;
pub mod error;
pub(crate) mod gate;
pub mod handle;
pub mod model;
pub mod ocpp;
pub mod policy;
pub mod report;
pub mod scenarios;
pub mod simulator;
pub mod throttle;

pub use error::SimError;
pub use report::report_to_markdown;
pub use scenarios::{AppProbe, AppProbeData};
pub use simulator::{Settings, Simulator};

/// Receives state changes; the shell forwards them as Tauri events.
pub trait EventSink: Send + Sync + 'static {
    /// A box changed (at most 4 times per second per box).
    fn charge_point_updated(&self, snapshot: &model::ChargePointSnapshot);
    /// A frame went out or came in.
    fn frame_logged(&self, entry: &model::FrameLogEntry);
    /// A box was removed.
    fn charge_point_removed(&self, id: &str);
}
