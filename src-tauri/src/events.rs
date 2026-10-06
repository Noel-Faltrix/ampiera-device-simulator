//! Forwards core events to the webview. State updates are coalesced per box so the UI gets at most
//! four per second, while the newest state is never dropped.

use sim_core::model::{ChargePointSnapshot, FrameLogEntry};
use sim_core::EventSink;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Minimum spacing of `charge-point-updated` events per box (4 per second).
const MIN_INTERVAL: Duration = Duration::from_millis(250);

/// What the caller has to do with an offered value.
#[derive(Debug, PartialEq)]
pub enum Decision<T> {
    /// Emit this value now.
    EmitNow(T),
    /// Keep the value; call `flush` after this delay.
    FlushAfter(Duration),
    /// Keep the value; a flush is already scheduled and will pick up the newest one.
    Held,
}

struct Slot<T> {
    last_emit: Option<Instant>,
    pending: Option<T>,
    flush_scheduled: bool,
}

/// Rate limiter with trailing edge: the last value of a burst is emitted when the interval ends.
pub struct Coalescer<T> {
    interval: Duration,
    slots: HashMap<String, Slot<T>>,
}

impl<T> Coalescer<T> {
    /// Creates a limiter that spaces emits per key by `interval`.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            slots: HashMap::new(),
        }
    }

    /// Registers a new value for `key`.
    pub fn offer(&mut self, key: &str, value: T, now: Instant) -> Decision<T> {
        let slot = self.slots.entry(key.to_owned()).or_insert(Slot {
            last_emit: None,
            pending: None,
            flush_scheduled: false,
        });
        if slot.flush_scheduled {
            slot.pending = Some(value);
            return Decision::Held;
        }
        if let Some(last) = slot.last_emit {
            let since = now.saturating_duration_since(last);
            if since < self.interval {
                slot.pending = Some(value);
                slot.flush_scheduled = true;
                return Decision::FlushAfter(self.interval - since);
            }
        }
        slot.last_emit = Some(now);
        Decision::EmitNow(value)
    }

    /// Called when a scheduled flush is due; returns the value to emit, if any.
    pub fn flush(&mut self, key: &str, now: Instant) -> Option<T> {
        let slot = self.slots.get_mut(key)?;
        slot.flush_scheduled = false;
        let value = slot.pending.take()?;
        slot.last_emit = Some(now);
        Some(value)
    }

    /// Drops all state of `key`, including a pending value (the box is gone).
    pub fn forget(&mut self, key: &str) {
        self.slots.remove(key);
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere must not silence the UI events for good.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `EventSink` that emits Tauri events (CONTRACT section 3).
pub struct TauriSink {
    app: AppHandle,
    updates: Arc<Mutex<Coalescer<ChargePointSnapshot>>>,
}

impl TauriSink {
    /// Creates a sink that emits on `app`.
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            updates: Arc::new(Mutex::new(Coalescer::new(MIN_INTERVAL))),
        }
    }
}

// An emit only fails when the window is already closed; there is nobody left to tell.
fn emit_update(app: &AppHandle, snapshot: &ChargePointSnapshot) {
    let _ = app.emit("charge-point-updated", snapshot);
}

impl EventSink for TauriSink {
    fn charge_point_updated(&self, snapshot: &ChargePointSnapshot) {
        let decision = locked(&self.updates).offer(&snapshot.id, snapshot.clone(), Instant::now());
        match decision {
            Decision::EmitNow(value) => emit_update(&self.app, &value),
            Decision::Held => {}
            Decision::FlushAfter(delay) => {
                let (app, updates, key) =
                    (self.app.clone(), self.updates.clone(), snapshot.id.clone());
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let value = locked(&updates).flush(&key, Instant::now());
                    if let Some(value) = value {
                        emit_update(&app, &value);
                    }
                });
            }
        }
    }

    fn frame_logged(&self, entry: &FrameLogEntry) {
        let _ = self.app.emit("frame-logged", entry);
    }

    fn charge_point_removed(&self, id: &str) {
        locked(&self.updates).forget(id);
        let _ = self
            .app
            .emit("charge-point-removed", serde_json::json!({ "id": id }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn first_value_is_emitted_immediately() {
        let mut c = Coalescer::new(ms(250));
        assert_eq!(c.offer("a", 1, Instant::now()), Decision::EmitNow(1));
    }

    #[test]
    fn burst_is_reduced_to_first_and_last_value() {
        let mut c = Coalescer::new(ms(250));
        let t0 = Instant::now();
        assert_eq!(c.offer("a", 1, t0), Decision::EmitNow(1));
        assert_eq!(c.offer("a", 2, t0 + ms(10)), Decision::FlushAfter(ms(240)));
        assert_eq!(c.offer("a", 3, t0 + ms(20)), Decision::Held);
        assert_eq!(c.offer("a", 4, t0 + ms(30)), Decision::Held);
        assert_eq!(c.flush("a", t0 + ms(250)), Some(4));
        assert_eq!(c.flush("a", t0 + ms(251)), None);
    }

    #[test]
    fn value_after_the_interval_is_emitted_directly() {
        let mut c = Coalescer::new(ms(250));
        let t0 = Instant::now();
        c.offer("a", 1, t0);
        assert_eq!(c.offer("a", 2, t0 + ms(250)), Decision::EmitNow(2));
    }

    #[test]
    fn boxes_are_limited_independently() {
        let mut c = Coalescer::new(ms(250));
        let t0 = Instant::now();
        c.offer("a", 1, t0);
        assert_eq!(c.offer("b", 1, t0 + ms(5)), Decision::EmitNow(1));
    }

    #[test]
    fn forgotten_box_loses_its_pending_value() {
        let mut c = Coalescer::new(ms(250));
        let t0 = Instant::now();
        c.offer("a", 1, t0);
        c.offer("a", 2, t0 + ms(10));
        c.forget("a");
        assert_eq!(c.flush("a", t0 + ms(250)), None);
        assert_eq!(c.offer("a", 3, t0 + ms(260)), Decision::EmitNow(3));
    }

    #[test]
    fn flush_restarts_the_interval() {
        let mut c = Coalescer::new(ms(250));
        let t0 = Instant::now();
        c.offer("a", 1, t0);
        c.offer("a", 2, t0 + ms(10));
        assert_eq!(c.flush("a", t0 + ms(250)), Some(2));
        assert_eq!(c.offer("a", 3, t0 + ms(300)), Decision::FlushAfter(ms(200)));
    }
}
