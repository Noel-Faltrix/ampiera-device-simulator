//! Registry of all boxes that decides, in one place and under one lock, whether a box may be added and
//! whether it may open a connection. Every connect path (UI, scenarios, reboot) goes through `reserve`.

use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use crate::error::SimError;
use crate::model::{ChargePointConfig, ChargePointSnapshot, ConnectionState, TargetKind};
use crate::policy;

struct Entry {
    id: String,
    key: String,
    snapshot: watch::Receiver<ChargePointSnapshot>,
    /// Set while a connect command is on its way to the actor: the slot counts as taken even though the
    /// snapshot does not show `Connecting` yet. This closes the race between two parallel connects.
    reserved: bool,
}

/// Shared by the simulator and all box handles.
#[derive(Default)]
pub(crate) struct ConnectGate {
    entries: Mutex<Vec<Entry>>,
}

/// Holds a reserved live slot; releases it when dropped.
pub(crate) struct Reservation {
    gate: Arc<ConnectGate>,
    id: Option<String>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let mut entries = self.gate.lock();
            if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                entry.reserved = false;
            }
        }
    }
}

fn is_active(connection: &ConnectionState) -> bool {
    matches!(
        connection,
        ConnectionState::Connecting
            | ConnectionState::Connected { .. }
            | ConnectionState::Reconnecting { .. }
    )
}

impl ConnectGate {
    fn lock(&self) -> MutexGuard<'_, Vec<Entry>> {
        // A poisoned lock only means another thread panicked while holding it; the list itself stays valid.
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Registers a new box; refuses a 21st box and a second box with the same address and identity.
    pub(crate) fn register(
        &self,
        id: &str,
        config: &ChargePointConfig,
        snapshot: watch::Receiver<ChargePointSnapshot>,
    ) -> Result<(), SimError> {
        let mut entries = self.lock();
        if entries.len() >= policy::MAX_BOXES {
            return Err(SimError::TooManyBoxes {
                max: policy::MAX_BOXES,
            });
        }
        let key = policy::endpoint_key(config);
        if entries.iter().any(|e| e.key == key) {
            return Err(SimError::DuplicateBox);
        }
        entries.push(Entry {
            id: id.to_string(),
            key,
            snapshot,
            reserved: false,
        });
        Ok(())
    }

    /// Forgets a box.
    pub(crate) fn unregister(&self, id: &str) {
        self.lock().retain(|e| e.id != id);
    }

    /// Refuses an extra connection of box `id` (scenario S7 opens a second one) when the box is live and the
    /// live limit is already used up by active live boxes, itself included.
    pub(crate) fn check_extra_connection(&self, id: &str) -> Result<(), SimError> {
        let entries = self.lock();
        let Some(entry) = entries.iter().find(|e| e.id == id) else {
            return Ok(());
        };
        if policy::effective_kind(&entry.snapshot.borrow().config) != TargetKind::Live {
            return Ok(());
        }
        let active = entries
            .iter()
            .filter(|e| {
                let snapshot = e.snapshot.borrow();
                policy::effective_kind(&snapshot.config) == TargetKind::Live
                    && (e.reserved || is_active(&snapshot.connection))
            })
            .count();
        if active >= policy::MAX_LIVE_BOXES_CONNECTED {
            return Err(SimError::TooManyLiveBoxes {
                max: policy::MAX_LIVE_BOXES_CONNECTED,
            });
        }
        Ok(())
    }

    /// Reserves a live connection slot for `id`, or refuses when three other live boxes are active.
    /// Local boxes and boxes that are already active need no slot.
    pub(crate) fn reserve(self: &Arc<Self>, id: &str) -> Result<Reservation, SimError> {
        let mut entries = self.lock();
        let none = Reservation {
            gate: self.clone(),
            id: None,
        };
        let Some(index) = entries.iter().position(|e| e.id == id) else {
            return Ok(none);
        };
        let own = entries[index].snapshot.borrow().clone();
        if policy::effective_kind(&own.config) != TargetKind::Live
            || is_active(&own.connection)
            || entries[index].reserved
        {
            return Ok(none);
        }
        let others = entries
            .iter()
            .filter(|e| e.id != id)
            .filter(|e| {
                let snapshot = e.snapshot.borrow();
                policy::effective_kind(&snapshot.config) == TargetKind::Live
                    && (e.reserved || is_active(&snapshot.connection))
            })
            .count();
        if policy::live_limit_reached(others) {
            return Err(SimError::TooManyLiveBoxes {
                max: policy::MAX_LIVE_BOXES_CONNECTED,
            });
        }
        entries[index].reserved = true;
        Ok(Reservation {
            gate: self.clone(),
            id: Some(id.to_string()),
        })
    }
}
