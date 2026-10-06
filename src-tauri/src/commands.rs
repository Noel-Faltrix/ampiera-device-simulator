//! Tauri commands (CONTRACT section 2). Every error is a German text without secrets.

use crate::state::AppState;
use app_view::AppLoginResult;
use sim_core::model::{
    ChargePointConfig, ChargePointSnapshot, ScenarioId, ScenarioInfo, ScenarioReport, VehicleConfig,
};
use tauri::State;

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Lists all boxes with their current state.
#[tauri::command]
pub async fn list_charge_points(
    state: State<'_, AppState>,
) -> Result<Vec<ChargePointSnapshot>, String> {
    Ok(state.sim.list().await)
}

/// Creates a box; the password goes to the keychain only.
#[tauri::command]
pub async fn add_charge_point(
    state: State<'_, AppState>,
    config: ChargePointConfig,
    password: String,
    live_confirmed: bool,
) -> Result<String, String> {
    state.add_box(config, password, live_confirmed).await
}

/// Removes a box and its saved data.
#[tauri::command]
pub async fn remove_charge_point(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.remove_box(&id).await
}

/// Opens the connection of a box.
#[tauri::command]
pub async fn connect(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.sim.connect(&id).await.map_err(text)
}

/// Closes the connection of a box.
#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.sim.disconnect(&id).await.map_err(text)
}

/// Plugs a vehicle into a box.
#[tauri::command]
pub async fn plug_in(
    state: State<'_, AppState>,
    id: String,
    vehicle: VehicleConfig,
) -> Result<(), String> {
    state.sim.plug_in(&id, vehicle).await.map_err(text)
}

/// Unplugs the vehicle.
#[tauri::command]
pub async fn unplug(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.sim.unplug(&id).await.map_err(text)
}

/// Closes and reopens the connection; the box sends a new BootNotification.
#[tauri::command]
pub async fn reboot(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.sim.reboot(&id).await.map_err(text)
}

/// Lists the available scenarios.
#[tauri::command]
pub fn list_scenarios(state: State<'_, AppState>) -> Vec<ScenarioInfo> {
    state.sim.scenarios()
}

/// Runs a scenario on a box and returns its report.
#[tauri::command]
pub async fn run_scenario(
    state: State<'_, AppState>,
    id: String,
    scenario_id: ScenarioId,
) -> Result<ScenarioReport, String> {
    state.sim.run_scenario(&id, scenario_id).await.map_err(text)
}

/// Stops the running scenario of a box.
#[tauri::command]
pub async fn abort_scenario(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.sim.abort_scenario(&id).await.map_err(text)
}

/// Frame log of a box as a JSON array.
#[tauri::command]
pub async fn export_log(state: State<'_, AppState>, id: String) -> Result<String, String> {
    state.sim.export_log(&id).await.map_err(text)
}

/// Scenario report as Markdown.
#[tauri::command]
pub fn export_report(report: ScenarioReport) -> String {
    sim_core::report_to_markdown(&report)
}

/// Redeems an invite of the customer app and sets the first password.
#[tauri::command]
pub async fn app_redeem_invite(
    state: State<'_, AppState>,
    base_url: String,
    email: String,
    invite_token: String,
    password: String,
) -> Result<(), String> {
    state
        .app_view
        .redeem_invite(&base_url, &email, &invite_token, &password)
        .await
        .map_err(text)
}

/// Signs in to the customer API; tokens stay in memory.
#[tauri::command]
pub async fn app_login(
    state: State<'_, AppState>,
    base_url: String,
    email: String,
    password: String,
) -> Result<AppLoginResult, String> {
    state
        .app_view
        .login(&base_url, &email, &password, &state.device_id)
        .await
        .map_err(text)
}

/// Completes the login with the code from the e-mail.
#[tauri::command]
pub async fn app_verify_device(state: State<'_, AppState>, code: String) -> Result<(), String> {
    state.app_view.verify_device(&code).await.map_err(text)
}

/// Signs out and forgets the tokens.
#[tauri::command]
pub async fn app_logout(state: State<'_, AppState>) -> Result<(), String> {
    state.app_view.logout().await.map_err(text)
}

/// What the customer app shows now (cached for 30 seconds).
#[tauri::command]
pub async fn app_snapshot(state: State<'_, AppState>) -> Result<app_view::AppViewSnapshot, String> {
    state.app_view.snapshot().await.map_err(text)
}
