//! `Simulator`: owns all simulated charge points and enforces the rules of CONTRACT section 4.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{watch, Mutex as AsyncMutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::error::SimError;
use crate::gate::ConnectGate;
use crate::handle::{BoxHandle, Timings};
use crate::model::{
    ChargePointConfig, ChargePointSnapshot, ScenarioId, ScenarioInfo, ScenarioReport, VehicleConfig,
};
use crate::ocpp::actor;
use crate::ocpp::client::Secret;
use crate::policy;
use crate::scenarios::{self, AppProbe, ScenarioCtx, ScenarioTuning};
use crate::EventSink;

/// Time allowed for an actor to stop before its task is aborted.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// All tunable times: production defaults, shortened in tests.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Actor timers.
    pub timings: Timings,
    /// Scenario waits and thresholds.
    pub tuning: ScenarioTuning,
}

/// Abort switch of the scenario running on a box, if any. Shared with [`ScenarioSlot`], which clears it from
/// `drop` (a synchronous context), so the slot is freed even when the scenario future is cancelled.
type SharedSlot = Arc<Mutex<Option<watch::Sender<bool>>>>;

struct BoxEntry {
    handle: BoxHandle,
    task: JoinHandle<()>,
    scenario: SharedSlot,
}

/// Marks a box as "scenario running" and frees it again when dropped, however the run ends.
struct ScenarioSlot(SharedSlot);

impl Drop for ScenarioSlot {
    fn drop(&mut self) {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = None;
    }
}

struct Inner {
    gate: Arc<ConnectGate>,
    sink: Arc<dyn EventSink>,
    settings: Settings,
    probe: Mutex<Option<Arc<dyn AppProbe>>>,
    boxes: AsyncMutex<Vec<BoxEntry>>,
}

/// Owns all simulated charge points. Cheap to clone.
#[derive(Clone)]
pub struct Simulator {
    inner: Arc<Inner>,
}

fn find(boxes: &[BoxEntry], id: &str) -> Result<usize, SimError> {
    boxes
        .iter()
        .position(|b| b.handle.id == id)
        .ok_or_else(|| SimError::UnknownBox(id.to_string()))
}

impl Simulator {
    /// A simulator with production timings.
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self::with_settings(sink, Settings::default())
    }

    /// A simulator with custom timings (tests).
    pub fn with_settings(sink: Arc<dyn EventSink>, settings: Settings) -> Self {
        Self {
            inner: Arc::new(Inner {
                gate: Arc::new(ConnectGate::default()),
                sink,
                settings,
                probe: Mutex::new(None),
                boxes: AsyncMutex::new(Vec::new()),
            }),
        }
    }

    /// Registers the app view that scenario S11 compares against. Replaces an earlier one.
    pub fn set_app_probe(&self, probe: Arc<dyn AppProbe>) {
        if let Ok(mut slot) = self.inner.probe.lock() {
            *slot = Some(probe);
        }
    }

    /// Snapshots of all boxes in the order they were added.
    pub async fn list(&self) -> Vec<ChargePointSnapshot> {
        self.inner
            .boxes
            .lock()
            .await
            .iter()
            .map(|b| b.handle.snapshot())
            .collect()
    }

    /// Adds a box (not connected yet) and returns its local id.
    pub async fn add(
        &self,
        config: ChargePointConfig,
        password: String,
        live_confirmed: bool,
    ) -> Result<String, SimError> {
        policy::validate_new_box(&config, live_confirmed)?;
        policy::validate_password(&password)?;
        let id = Uuid::new_v4().to_string();
        let (handle, task) = actor::spawn(
            id.clone(),
            config.clone(),
            Secret::new(password),
            self.inner.sink.clone(),
            self.inner.settings.timings.clone(),
            self.inner.gate.clone(),
        );
        // Box count and duplicate address are checked in the registry under its lock; a refused box is stopped
        // again before anyone has seen it.
        if let Err(error) = self
            .inner
            .gate
            .register(&id, &config, handle.watch_snapshot())
        {
            task.abort();
            return Err(error);
        }
        self.inner.sink.charge_point_updated(&handle.snapshot());
        self.inner.boxes.lock().await.push(BoxEntry {
            handle,
            task,
            scenario: Arc::new(Mutex::new(None)),
        });
        Ok(id)
    }

    /// Removes a box: aborts a running scenario and stops its task.
    pub async fn remove(&self, id: &str) -> Result<(), SimError> {
        let entry = {
            let mut boxes = self.inner.boxes.lock().await;
            let index = find(&boxes, id)?;
            boxes.remove(index)
        };
        let running = entry
            .scenario
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        if let Some(abort) = running {
            // The scenario may have ended already; then nobody listens.
            let _ = abort.send(true);
        }
        entry.handle.shutdown().await;
        let mut task = entry.task;
        if tokio::time::timeout(SHUTDOWN_WAIT, &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
        // Only now is the box really gone: its live slot is free after the actor has stopped.
        self.inner.gate.unregister(id);
        self.inner.sink.charge_point_removed(id);
        Ok(())
    }

    async fn handle_of(&self, id: &str) -> Result<BoxHandle, SimError> {
        let boxes = self.inner.boxes.lock().await;
        let index = find(&boxes, id)?;
        Ok(boxes[index].handle.clone())
    }

    /// Opens the connection. Towards the Produktivserver at most 3 boxes may be active at the same time; the
    /// check lives in the box handle, so scenarios are held to it as well.
    pub async fn connect(&self, id: &str) -> Result<(), SimError> {
        self.handle_of(id).await?.connect().await
    }

    /// Closes the connection and stays offline.
    pub async fn disconnect(&self, id: &str) -> Result<(), SimError> {
        self.handle_of(id).await?.disconnect().await
    }

    /// Plugs a vehicle in.
    pub async fn plug_in(&self, id: &str, vehicle: VehicleConfig) -> Result<(), SimError> {
        self.handle_of(id).await?.plug_in(vehicle).await
    }

    /// Unplugs the vehicle.
    pub async fn unplug(&self, id: &str) -> Result<(), SimError> {
        self.handle_of(id).await?.unplug().await
    }

    /// Closes the connection and reconnects with a new BootNotification.
    pub async fn reboot(&self, id: &str) -> Result<(), SimError> {
        self.handle_of(id).await?.reboot().await
    }

    /// Static info of all scenarios.
    pub fn scenarios(&self) -> Vec<ScenarioInfo> {
        scenarios::catalog()
    }

    /// Runs a scenario on a box and returns its report. Scenarios that must not touch the Produktivserver fail
    /// immediately.
    pub async fn run_scenario(
        &self,
        id: &str,
        scenario: ScenarioId,
    ) -> Result<ScenarioReport, SimError> {
        let info = scenarios::info(scenario);
        let (handle, abort_rx, _slot) = {
            let boxes = self.inner.boxes.lock().await;
            let index = find(&boxes, id)?;
            let entry = &boxes[index];
            if !scenarios::allowed_on(&info, &entry.handle.snapshot().config) {
                return Err(SimError::ScenarioNotAllowedOnLive(scenario));
            }
            let mut slot = entry
                .scenario
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_some() {
                return Err(SimError::ScenarioAlreadyRunning);
            }
            let (abort_tx, abort_rx) = watch::channel(false);
            *slot = Some(abort_tx);
            (
                entry.handle.clone(),
                abort_rx,
                ScenarioSlot(entry.scenario.clone()),
            )
        };
        let probe = self.inner.probe.lock().ok().and_then(|slot| slot.clone());
        let ctx = ScenarioCtx::new(
            handle,
            info,
            self.inner.settings.tuning.clone(),
            probe,
            abort_rx,
        );
        Ok(scenarios::run(scenario, ctx).await)
    }

    /// Asks the running scenario of a box to stop; its report then has the outcome `aborted`.
    pub async fn abort_scenario(&self, id: &str) -> Result<(), SimError> {
        let boxes = self.inner.boxes.lock().await;
        let index = find(&boxes, id)?;
        let running = boxes[index]
            .scenario
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        match running {
            Some(abort) => {
                // The scenario may have finished a moment ago; aborting a finished run is harmless.
                let _ = abort.send(true);
                Ok(())
            }
            None => Err(SimError::NoScenarioRunning),
        }
    }

    /// The in-memory frame log (last 2000 entries) as a JSON array of `FrameLogEntry`.
    pub async fn export_log(&self, id: &str) -> Result<String, SimError> {
        let entries = self.handle_of(id).await?.log_entries();
        serde_json::to_string(&entries).map_err(|e| SimError::LogExport(e.to_string()))
    }
}
