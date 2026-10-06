//! Shared state of the shell and the wallbox lifecycle (create, restore, remove) across core,
//! keychain and `boxes.json`.

use crate::events::TauriSink;
use crate::keychain;
use crate::persist::{load_or_create_device_id, BoxStore, StoredBox};
use crate::probe::AppProbeAdapter;
use app_view::AppClient;
use serde_json::json;
use sim_core::model::ChargePointConfig;
use sim_core::Simulator;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{AppHandle, Emitter};

/// Everything the commands need. Registered once as Tauri state.
pub struct AppState {
    /// The simulator core.
    pub sim: Simulator,
    /// Client for the customer API; tokens stay in its memory.
    pub app_view: Arc<AppClient>,
    /// Random id of this installation, sent as `geraete_id` at app login.
    pub device_id: String,
    app: AppHandle,
    boxes: BoxStore,
    // Local id of the core (new on every start) -> keychain account / `boxes.json` key.
    store_ids: Mutex<HashMap<String, String>>,
    // Startup problems, delivered to the UI once it has subscribed (see `deliver_problems`).
    problems: Mutex<Vec<String>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    /// Builds the state from the config directory. Startup problems are collected and later sent
    /// as `restore-problem` events.
    pub async fn init(app: AppHandle, config_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(config_dir).map_err(|e| {
            format!(
                "Der Konfigurationsordner konnte nicht angelegt werden ({}).",
                e.kind()
            )
        })?;
        let device_id = load_or_create_device_id(config_dir)?;
        let (boxes, warning) = BoxStore::load(config_dir).await;
        let app_view = Arc::new(AppClient::try_new().map_err(|e| e.to_string())?);
        let sim = Simulator::new(Arc::new(TauriSink::new(app.clone())));
        sim.set_app_probe(Arc::new(AppProbeAdapter::new(app_view.clone())));
        Ok(Self {
            sim,
            app_view,
            device_id,
            app,
            boxes,
            store_ids: Mutex::new(HashMap::new()),
            problems: Mutex::new(warning.into_iter().collect()),
        })
    }

    /// Creates a wallbox. Nothing stays behind if the keychain or the file cannot be written.
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
            return Err(format!("Die Wallbox wurde nicht angelegt: {e}"));
        }
        let stored = StoredBox {
            store_id: id.clone(),
            config,
            live_confirmed,
        };
        if let Err(e) = self.boxes.add(stored).await {
            let _ = keychain::delete_password(&id).await;
            self.discard(&id).await;
            return Err(format!("Die Wallbox wurde nicht angelegt: {e}"));
        }
        locked(&self.store_ids).insert(id.clone(), id.clone());
        Ok(id)
    }

    // Rollback of a wallbox that exists in the core but could not be saved. The core cannot fail
    // for one it just created, and the caller already reports the original error.
    async fn discard(&self, id: &str) {
        let _ = self.sim.remove(id).await;
    }

    /// Removes a wallbox from the core, `boxes.json` and the keychain.
    pub async fn remove_box(&self, id: &str) -> Result<(), String> {
        self.sim.remove(id).await.map_err(|e| e.to_string())?;
        let store_id = locked(&self.store_ids)
            .remove(id)
            .unwrap_or_else(|| id.to_owned());
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
                "Die Wallbox wurde entfernt, aber nicht vollständig aufgeräumt: {}",
                problems.join(" ")
            ))
        }
    }

    /// Re-creates the saved wallboxes without connecting them. A wallbox that cannot be restored
    /// (password missing in the keychain, rejected by the core) stays in `boxes.json`; the reason
    /// becomes a startup problem for the UI.
    pub async fn restore_boxes(&self) {
        for stored in self.boxes.list().await {
            let label = stored.config.label.clone();
            let problem =
                |reason: String| format!("Wallbox „{label}“ nicht wiederhergestellt: {reason}");
            let password = match keychain::get_password(&stored.store_id).await {
                Ok(Some(password)) => password,
                Ok(None) => {
                    self.report(problem("Das Passwort fehlt im Schlüsselbund.".to_owned()));
                    continue;
                }
                Err(e) => {
                    self.report(problem(e));
                    continue;
                }
            };
            match self
                .sim
                .add(stored.config, password, stored.live_confirmed)
                .await
            {
                Ok(id) => {
                    locked(&self.store_ids).insert(id, stored.store_id);
                }
                Err(e) => self.report(problem(e.to_string())),
            }
        }
    }

    fn report(&self, message: String) {
        eprintln!("Hinweis beim Start: {message}");
        locked(&self.problems).push(message);
    }

    /// Sends the collected startup problems as `restore-problem` events and forgets them. Called
    /// from `list_charge_points`, because the UI subscribes to events before it lists, while an
    /// event emitted during setup would arrive before any listener exists.
    pub fn deliver_problems(&self) {
        let pending = std::mem::take(&mut *locked(&self.problems));
        for message in pending {
            let _ = self
                .app
                .emit("restore-problem", json!({ "message": message }));
        }
    }
}
