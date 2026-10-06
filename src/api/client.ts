// The only module that talks to Tauri. Tests mock this module.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppLoginResult,
  AppViewSnapshot,
  ChargePointConfig,
  ChargePointSnapshot,
  FrameLogEntry,
  ScenarioId,
  ScenarioInfo,
  ScenarioReport,
  VehicleConfig,
} from "./types";

export const listChargePoints = () => invoke<ChargePointSnapshot[]>("list_charge_points");

export const addChargePoint = (args: {
  config: ChargePointConfig;
  password: string;
  liveConfirmed: boolean;
}) => invoke<string>("add_charge_point", args);

export const removeChargePoint = (id: string) => invoke<void>("remove_charge_point", { id });
export const connect = (id: string) => invoke<void>("connect", { id });
export const disconnect = (id: string) => invoke<void>("disconnect", { id });
export const plugIn = (id: string, vehicle: VehicleConfig) =>
  invoke<void>("plug_in", { id, vehicle });
export const unplug = (id: string) => invoke<void>("unplug", { id });
export const reboot = (id: string) => invoke<void>("reboot", { id });

export const listScenarios = () => invoke<ScenarioInfo[]>("list_scenarios");
export const runScenario = (id: string, scenarioId: ScenarioId) =>
  invoke<ScenarioReport>("run_scenario", { id, scenarioId });
export const abortScenario = (id: string) => invoke<void>("abort_scenario", { id });

export const exportLog = (id: string) => invoke<string>("export_log", { id });
export const exportReport = (report: ScenarioReport) => invoke<string>("export_report", { report });

export const saveLog = (id: string) => invoke<string>("save_log", { id });
export const saveReport = (report: ScenarioReport) => invoke<string>("save_report", { report });

export const appRedeemInvite = (args: {
  baseUrl: string;
  email: string;
  inviteToken: string;
  password: string;
}) => invoke<void>("app_redeem_invite", args);
export const appLogin = (args: { baseUrl: string; email: string; password: string }) =>
  invoke<AppLoginResult>("app_login", args);
export const appVerifyDevice = (code: string) => invoke<void>("app_verify_device", { code });
export const appLogout = () => invoke<void>("app_logout");
export const appSnapshot = () => invoke<AppViewSnapshot>("app_snapshot");

export interface EventHandlers {
  onChargePointUpdated: (snapshot: ChargePointSnapshot) => void;
  onFrameLogged: (entry: FrameLogEntry) => void;
  onChargePointRemoved: (id: string) => void;
  onRestoreProblem: (message: string) => void;
}

/** Registers all event listeners; resolves to one function that removes them. */
export async function subscribe(handlers: EventHandlers): Promise<() => void> {
  const unlisteners: UnlistenFn[] = [];
  try {
    unlisteners.push(
      await listen<ChargePointSnapshot>("charge-point-updated", (e) =>
        handlers.onChargePointUpdated(e.payload),
      ),
      await listen<FrameLogEntry>("frame-logged", (e) => handlers.onFrameLogged(e.payload)),
      await listen<{ id: string }>("charge-point-removed", (e) =>
        handlers.onChargePointRemoved(e.payload.id),
      ),
      await listen<{ message: string }>("restore-problem", (e) =>
        handlers.onRestoreProblem(e.payload.message),
      ),
    );
  } catch (error) {
    unlisteners.forEach((off) => off());
    throw error;
  }
  return () => unlisteners.forEach((off) => off());
}
