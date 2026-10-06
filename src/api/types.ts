// Mirror of docs/CONTRACT.md section 1. Keep in sync with the Rust model.

export type TargetKind = "local" | "live";

export interface ChargePointConfig {
  label: string;
  targetKind: TargetKind;
  baseUrl: string;
  identity: string;
  vendor: string;
  model: string;
  phases: 1 | 3;
  maxPowerW: number;
  supportsSoc: boolean;
  acceptedRateUnits: ("W" | "A")[];
  rejectProfiles: boolean;
  clockOffsetS: number;
}

export interface VehicleConfig {
  capacityKwh: number;
  socPct: number;
  maxPowerW: number;
  phases: 1 | 3;
}

export type ConnectionState =
  | { state: "disconnected" }
  | { state: "connecting" }
  | { state: "connected"; since: string }
  | { state: "reconnecting"; attempt: number; nextAttemptAt: string }
  | { state: "failed"; reason: string };

export type OcppStatus =
  | "Available"
  | "Preparing"
  | "Charging"
  | "SuspendedEV"
  | "SuspendedEVSE"
  | "Finishing"
  | "Reserved"
  | "Unavailable"
  | "Faulted";

export interface ActiveLimit {
  limitW: number;
  rawLimit: number;
  rateUnit: "W" | "A";
  profileId: number;
  purpose: "ChargePointMaxProfile" | "TxDefaultProfile" | "TxProfile";
  validTo: string | null;
}

export interface ChargePointSnapshot {
  id: string;
  config: ChargePointConfig;
  connection: ConnectionState;
  status: OcppStatus;
  vehicle: { config: VehicleConfig; socPct: number; plugged: boolean } | null;
  powerW: number | null;
  energyWh: number;
  transactionId: number | null;
  activeLimit: ActiveLimit | null;
  profileCount: number;
  heartbeatIntervalS: number | null;
  lastError: string | null;
}

export type FrameDirection = "out" | "in";

export interface FrameLogEntry {
  chargePointId: string;
  at: string;
  direction: FrameDirection;
  raw: string;
}

export type ScenarioId =
  "S1" | "S2" | "S3" | "S4" | "S5" | "S6" | "S6b" | "S7" | "S8" | "S9" | "S10" | "S11";

export interface ScenarioInfo {
  id: ScenarioId;
  title: string;
  description: string;
  liveAllowed: boolean;
  needsHuman: boolean;
  timeoutS: number;
}

export type CheckOutcome = "passed" | "failed" | "skipped";

export interface CheckResult {
  name: string;
  outcome: CheckOutcome;
  detail: string;
}

export interface ScenarioReport {
  scenarioId: ScenarioId;
  chargePointId: string;
  targetKind: TargetKind;
  startedAt: string;
  finishedAt: string;
  outcome: "passed" | "failed" | "aborted";
  checks: CheckResult[];
}

export type AppLoginResult = { result: "ok" } | { result: "device_code_required" };

export interface AppViewSummary {
  connection: string | null;
  lastContact: string | null;
  deviceStatus: string | null;
  livePowerW: number | null;
  geoState: string | null;
  geoHeadline: string | null;
  lastQuarterKwh: number | null;
  lastQuarterAt: string | null;
  nextScheduleSteps: { start: string; action: string; targetPowerW: number }[];
}

export interface AppViewSnapshot {
  fetchedAt: string;
  installationId: string | null;
  summary: AppViewSummary;
  raw: Record<string, unknown>;
}
