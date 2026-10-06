//! Forwards core events to the webview. The core already limits `charge-point-updated` to four per
//! second per wallbox (every emit goes through its throttle), so nothing is added here.

use sim_core::model::{ChargePointSnapshot, FrameLogEntry};
use sim_core::EventSink;
use tauri::{AppHandle, Emitter};

/// `EventSink` that emits Tauri events (CONTRACT section 3).
pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    /// Creates a sink that emits on `app`.
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

// An emit only fails when the window is already closed; there is nobody left to tell.
impl EventSink for TauriSink {
    fn charge_point_updated(&self, snapshot: &ChargePointSnapshot) {
        let _ = self.app.emit("charge-point-updated", snapshot);
    }

    fn frame_logged(&self, entry: &FrameLogEntry) {
        let _ = self.app.emit("frame-logged", entry);
    }

    fn charge_point_removed(&self, id: &str) {
        let _ = self
            .app
            .emit("charge-point-removed", serde_json::json!({ "id": id }));
    }
}
