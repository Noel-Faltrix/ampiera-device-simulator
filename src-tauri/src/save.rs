//! Writes exports to the user's Downloads folder. A webview cannot offer file downloads reliably
//! (WKWebView ignores Blob links), so the shell saves the file and returns its path.

use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const MAX_ATTEMPTS: u32 = 1000;

/// Keeps letters, digits, dot, underscore and hyphen so identities and ids cannot escape the folder.
pub fn safe_part(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "unbekannt".to_owned()
    } else {
        cleaned
    }
}

/// Creates `<dir>/<stem>.<ext>`, or `<stem>-2.<ext>`, `-3` and so on if the name is taken, and
/// writes `contents`. An existing file is never touched. Returns the path written.
pub async fn write_unique(
    dir: &Path,
    stem: &str,
    ext: &str,
    contents: &[u8],
) -> Result<PathBuf, String> {
    let failed =
        |e: std::io::Error| format!("Die Datei konnte nicht gespeichert werden ({}).", e.kind());
    tokio::fs::create_dir_all(dir).await.map_err(failed)?;
    for attempt in 1..=MAX_ATTEMPTS {
        let name = if attempt == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{attempt}.{ext}")
        };
        let path = dir.join(name);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(contents).await {
                    // Do not leave a half-written export behind.
                    let _ = tokio::fs::remove_file(&path).await;
                    return Err(failed(e));
                }
                file.flush().await.map_err(failed)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(failed(e)),
        }
    }
    Err("Die Datei konnte nicht gespeichert werden (zu viele gleichnamige Dateien im Downloads-Ordner).".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("ampiera-sim-save-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn existing_files_are_never_overwritten() {
        let dir = temp_dir();
        let first = write_unique(&dir, "bericht", "md", b"eins").await.unwrap();
        let second = write_unique(&dir, "bericht", "md", b"zwei").await.unwrap();
        let third = write_unique(&dir, "bericht", "md", b"drei").await.unwrap();
        assert_eq!(first.file_name().unwrap(), "bericht.md");
        assert_eq!(second.file_name().unwrap(), "bericht-2.md");
        assert_eq!(third.file_name().unwrap(), "bericht-3.md");
        assert_eq!(std::fs::read(&first).unwrap(), b"eins");
        assert_eq!(std::fs::read(&third).unwrap(), b"drei");
    }

    #[tokio::test]
    async fn unwritable_target_gives_a_german_error() {
        let dir = temp_dir();
        std::fs::write(&dir, "ich bin eine Datei").unwrap();
        let error = write_unique(&dir, "x", "json", b"{}").await.unwrap_err();
        assert!(
            error.starts_with("Die Datei konnte nicht gespeichert werden"),
            "{error}"
        );
    }

    #[test]
    fn file_name_parts_cannot_escape_the_folder() {
        assert_eq!(safe_part("AP7K2M9QX4RT"), "AP7K2M9QX4RT");
        assert_eq!(safe_part("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(safe_part(".."), "unbekannt");
        assert_eq!(safe_part(""), "unbekannt");
    }
}
