//! Files in the app config directory: `boxes.json` (box configs, never secrets) and `device_id`.

use serde::{Deserialize, Serialize};
use sim_core::model::ChargePointConfig;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

const BOXES_FILE: &str = "boxes.json";
const DEVICE_ID_FILE: &str = "device_id";
const FILE_VERSION: u32 = 1;
// The backend accepts 8 to 200 characters.
const DEVICE_ID_LEN: std::ops::RangeInclusive<usize> = 8..=200;

/// One saved wallbox. `store_id` is the keychain account; it equals the local id of the session in
/// which the box was created, because the core assigns a new local id on every start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredBox {
    /// Keychain account of the OCPP password.
    pub store_id: String,
    /// Wallbox configuration without secrets.
    pub config: ChargePointConfig,
    /// Whether the user confirmed live use when creating it; needed again to restore a live box.
    #[serde(default)]
    pub live_confirmed: bool,
}

#[derive(Serialize, Deserialize)]
struct BoxFile {
    version: u32,
    boxes: Vec<StoredBox>,
}

/// Saved wallboxes, kept in memory and mirrored to `boxes.json` on every change.
pub struct BoxStore {
    path: PathBuf,
    boxes: Mutex<Vec<StoredBox>>,
}

impl BoxStore {
    /// Loads `boxes.json`. A missing file is an empty list. A damaged file is renamed to
    /// `boxes.json.defekt` so it is not overwritten, and a warning is returned.
    pub async fn load(dir: &Path) -> (Self, Option<String>) {
        let path = dir.join(BOXES_FILE);
        let mut warning = None;
        let boxes = match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<BoxFile>(&bytes) {
                Ok(file) if file.version == FILE_VERSION => file.boxes,
                _ => {
                    let backup = dir.join(format!("{BOXES_FILE}.defekt"));
                    let moved = tokio::fs::rename(&path, &backup).await.is_ok();
                    warning = Some(format!(
                        "Die gespeicherte Wallbox-Liste ist unlesbar oder stammt von einer anderen Programmversion; sie wurde {}.",
                        if moved { "als boxes.json.defekt gesichert" } else { "nicht gesichert (Umbenennen fehlgeschlagen)" }
                    ));
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                warning = Some(format!(
                    "Die gespeicherte Wallbox-Liste konnte nicht gelesen werden ({}).",
                    e.kind()
                ));
                Vec::new()
            }
        };
        (
            Self {
                path,
                boxes: Mutex::new(boxes),
            },
            warning,
        )
    }

    /// Snapshot of all saved boxes.
    pub async fn list(&self) -> Vec<StoredBox> {
        self.boxes.lock().await.clone()
    }

    /// Saves a new box.
    pub async fn add(&self, entry: StoredBox) -> Result<(), String> {
        let mut boxes = self.boxes.lock().await;
        let mut next = boxes.clone();
        next.push(entry);
        self.write(&next).await?;
        *boxes = next;
        Ok(())
    }

    /// Removes the box with this keychain account. Unknown ids are ignored.
    pub async fn remove(&self, store_id: &str) -> Result<(), String> {
        let mut boxes = self.boxes.lock().await;
        let next: Vec<StoredBox> = boxes
            .iter()
            .filter(|b| b.store_id != store_id)
            .cloned()
            .collect();
        self.write(&next).await?;
        *boxes = next;
        Ok(())
    }

    // Write to a temporary file first so a crash cannot leave a half-written list behind.
    async fn write(&self, boxes: &[StoredBox]) -> Result<(), String> {
        let failed = |e: std::io::Error| {
            format!(
                "Die Wallbox-Liste konnte nicht gespeichert werden ({}).",
                e.kind()
            )
        };
        let file = BoxFile {
            version: FILE_VERSION,
            boxes: boxes.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(|_| {
            "Die Wallbox-Liste konnte nicht gespeichert werden (Serialisierung).".to_owned()
        })?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, bytes).await.map_err(failed)?;
        tokio::fs::rename(&tmp, &self.path).await.map_err(failed)
    }
}

/// Reads the app's device id, creating a random UUID v4 on first start. An unusable file content
/// is replaced, because the backend would reject it anyway.
pub fn load_or_create_device_id(dir: &Path) -> Result<String, String> {
    let path = dir.join(DEVICE_ID_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = text.trim();
        if DEVICE_ID_LEN.contains(&id.chars().count()) && id.chars().all(|c| c.is_ascii_graphic()) {
            return Ok(id.to_owned());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::write(&path, &id).map_err(|e| {
        format!(
            "Die Geräte-Kennung konnte nicht gespeichert werden ({}).",
            e.kind()
        )
    })?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ampiera-sim-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn entry(store_id: &str, label: &str) -> StoredBox {
        let mut config = ChargePointConfig::default_local("AP7K2M9QX4RT");
        config.label = label.to_owned();
        StoredBox {
            store_id: store_id.to_owned(),
            config,
            live_confirmed: false,
        }
    }

    #[tokio::test]
    async fn boxes_survive_a_restart_and_contain_no_password_field() {
        let dir = temp_dir();
        let (store, warning) = BoxStore::load(&dir).await;
        assert!(warning.is_none());
        store.add(entry("id-1", "Box 1")).await.unwrap();
        store.add(entry("id-2", "Box 2")).await.unwrap();
        store.remove("id-1").await.unwrap();

        let (again, warning) = BoxStore::load(&dir).await;
        assert!(warning.is_none());
        assert_eq!(again.list().await, vec![entry("id-2", "Box 2")]);
        let text = std::fs::read_to_string(dir.join("boxes.json")).unwrap();
        assert!(!text.to_lowercase().contains("password"), "{text}");
        assert!(!dir.join("boxes.json.tmp").exists());
    }

    #[tokio::test]
    async fn damaged_file_is_kept_aside_and_reported() {
        let dir = temp_dir();
        std::fs::write(dir.join("boxes.json"), "{ kaputt").unwrap();
        let (store, warning) = BoxStore::load(&dir).await;
        assert!(store.list().await.is_empty());
        assert!(warning.unwrap().contains("boxes.json.defekt"));
        assert!(dir.join("boxes.json.defekt").exists());
        store.add(entry("id-1", "Box 1")).await.unwrap();
        assert!(dir.join("boxes.json").exists());
    }

    #[test]
    fn device_id_is_created_once_and_reused() {
        let dir = temp_dir();
        let first = load_or_create_device_id(&dir).unwrap();
        assert!((8..=200).contains(&first.len()));
        assert_eq!(load_or_create_device_id(&dir).unwrap(), first);
    }

    #[test]
    fn unusable_device_id_file_is_replaced() {
        let dir = temp_dir();
        std::fs::write(dir.join("device_id"), "kurz").unwrap();
        let id = load_or_create_device_id(&dir).unwrap();
        assert_ne!(id, "kurz");
        assert_eq!(std::fs::read_to_string(dir.join("device_id")).unwrap(), id);
    }
}
