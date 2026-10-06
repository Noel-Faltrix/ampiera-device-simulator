//! Tauri shell of the Ampiera Device Simulator. The OCPP logic lives in `sim-core`, the customer
//! API client in `app-view`; this crate wires them to the webview, the keychain and the disk.

mod commands;
mod events;
mod keychain;
mod persist;
mod probe;
mod state;

use state::AppState;
use tauri::Manager;

/// Starts the application.
pub fn run() {
    let result = tauri::Builder::default()
        .setup(|app| {
            let config_dir = app.path().app_config_dir().map_err(|_| {
                "Der Konfigurationsordner des Betriebssystems ist nicht verfügbar.".to_owned()
            })?;
            let handle = app.handle().clone();
            let (state, mut warnings) =
                tauri::async_runtime::block_on(AppState::init(handle, &config_dir))?;
            warnings.extend(tauri::async_runtime::block_on(state.restore_boxes()));
            for warning in warnings {
                eprintln!("Hinweis beim Start: {warning}");
            }
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_charge_points,
            commands::add_charge_point,
            commands::remove_charge_point,
            commands::connect,
            commands::disconnect,
            commands::plug_in,
            commands::unplug,
            commands::reboot,
            commands::list_scenarios,
            commands::run_scenario,
            commands::abort_scenario,
            commands::export_log,
            commands::export_report,
            commands::app_redeem_invite,
            commands::app_login,
            commands::app_verify_device,
            commands::app_logout,
            commands::app_snapshot,
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("Der Simulator konnte nicht gestartet werden: {error}");
        std::process::exit(1);
    }
}
