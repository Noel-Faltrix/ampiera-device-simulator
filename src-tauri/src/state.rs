//! Shared state of the shell and the box lifecycle (create, restore, remove) across core,
//! keychain and `boxes.json`.

use crate::events::TauriSink;
use crate::keychain;
use crate::persist::{load_or_create_device_id, BoxStore, StoredBox};
use crate::probe::AppProbeAdapter;
use app_view::AppClient;
use sim_core::model::ChargePointConfig;
use sim_core::Simulator;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

/// Everything the commands need. Registered once as Tauri state.
pub struct AppState {
    /// The simulator core.
    pub sim: Simulator,
    /// Client for the customer API; tokens stay in its memory.
    pub app_view: Arc<AppClient>,
    /// Random id of this installation, sent as `geraete_id` at app login.
    pub device_id: String,
    boxes: BoxStore,
    // Local id of the core (new on every start) -> keychain account / `boxes.json` key.
    store_ids: Mutex<HashMap<String, String>>,
}

impl AppState {
    /// Builds the state from the config directory. Returns startup warnings for the log.
    pub async fn init(app: AppHandle, config_dir: &Path) -> Result<(Self, Vec<String>), String> {
        std::fs::create_dir_all(config_dir).map_err(|e| {
            format!(
                "Der Konfigurationsordner konnte nicht angelegt werden ({}).",
                e.kind()
            )
        })?;
        let device_id = load_or_create_device_id(config_dir)?;
        let (boxes, warning) = BoxStore::load(config_dir).await;
        let app_view = Arc::new(AppClient::new());
        let sim = Simulator::new(Arc::new(TauriSink::new(app)));
        sim.set_app_probe(Arc::new(AppProbeAdapter::new(app_view.clone())));
        let state = Self {
            sim,
            app_view,
            device_id,
            boxes,
            store_ids: Mutex::new(HashMap::new()),
        };
        Ok((state, warning.into_iter().collect()))
    }

    fn map_ids(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.store_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Creates a box. Nothing stays behind if the keychain or the file cannot be written.
    pub async fn add_box(
        &self,
        config: ChargePointConfig,
        password: String,
        live_confirmed: bool,
    ) -> Result<String, String> {
        let id = self
            .sim
            .add(config.clone(), password.clone(), live_confirmed)
            .await
            .map_err(|e| e.to_string())?;
        if let Err(e) = keychain::set_password(&id, &password).await {
            self.discard(&id).await;
            return Err(format!("Die Box wurde nicht angelegt: {e}"));
        }
        let stored = StoredBox {
            store_id: id.clone(),
            config,
        };
        if let Err(e) = self.boxes.add(stored).await {
            let _ = keychain::delete_password(&id).await;
            self.discard(&id).await;
            return Err(format!("Die Box wurde nicht angelegt: {e}"));
        }
        self.map_ids().insert(id.clone(), id.clone());
        Ok(id)
    }

    // Rollback of a box that exists in the core but could not be saved. The core cannot fail for a
    // box it just created, and the caller already reports the original error.
    async fn discard(&self, id: &str) {
        let _ = self.sim.remove(id).await;
    }

    /// Removes a box from the core, `boxes.json` and the keychain.
    pub async fn remove_box(&self, id: &str) -> Result<(), String> {
        self.sim.remove(id).await.map_err(|e| e.to_string())?;
        let store_id = self.map_ids().remove(id).unwrap_or_else(|| id.to_owned());
        let mut problems = Vec::new();
        if let Err(e) = self.boxes.remove(&store_id).await {
            problems.push(e);
        }
        if let Err(e) = keychain::delete_password(&store_id).await {
            problems.push(e);
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Die Box wurde entfernt, aber nicht vollständig aufgeräumt: {}",
                problems.join(" ")
            ))
        }
    }

    /// Re-creates the saved boxes without connecting them. A box whose password is missing in the
    /// keychain stays in `boxes.json` and is reported, so a later start can pick it up again.
    pub async fn restore_boxes(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for stored in self.boxes.list().await {
            let label = stored.config.label.clone();
            let password = match keychain::get_password(&stored.store_id).await {
                Ok(Some(password)) => password,
                Ok(None) => {
                    warnings.push(format!("Box „{label}“ nicht wiederhergestellt: Das Passwort fehlt im Schlüsselbund."));
                    continue;
                }
                Err(e) => {
                    warnings.push(format!("Box „{label}“ nicht wiederhergestellt: {e}"));
                    continue;
                }
            };
            // The box was confirmed for live use when it was created.
            match self.sim.add(stored.config, password, true).await {
                Ok(id) => {
                    self.map_ids().insert(id, stored.store_id);
                }
                Err(e) => warnings.push(format!("Box „{label}“ nicht wiederhergestellt: {e}")),
            }
        }
        warnings
    }
}
