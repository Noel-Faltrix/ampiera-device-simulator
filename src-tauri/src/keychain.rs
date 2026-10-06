//! OCPP passwords in the operating system keychain (macOS Keychain, Windows Credential Manager).
//! Calls block, so they run on the blocking thread pool. Errors never contain the secret.

use keyring::{Entry, Error};

/// Keychain service name shared by all entries of this app.
pub const SERVICE: &str = "de.ampiera.device-simulator";

fn describe(error: &Error) -> String {
    format!("Der Schlüsselbund des Betriebssystems meldet einen Fehler ({error}).")
}

async fn blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|_| "Der Zugriff auf den Schlüsselbund wurde unerwartet abgebrochen.".to_owned())?
        .map_err(|e| describe(&e))
}

/// Stores or replaces the password for `account`.
pub async fn set_password(account: &str, password: &str) -> Result<(), String> {
    let (account, password) = (account.to_owned(), password.to_owned());
    blocking(move || Entry::new(SERVICE, &account)?.set_password(&password)).await
}

/// Reads the password for `account`; `None` if there is no entry.
pub async fn get_password(account: &str) -> Result<Option<String>, String> {
    let account = account.to_owned();
    blocking(
        move || match Entry::new(SERVICE, &account)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(e),
        },
    )
    .await
}

/// Deletes the entry for `account`; a missing entry counts as success.
pub async fn delete_password(account: &str) -> Result<(), String> {
    let account = account.to_owned();
    blocking(
        move || match Entry::new(SERVICE, &account)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(e),
        },
    )
    .await
}
