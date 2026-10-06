import type {
  ChargePointSnapshot,
  FrameLogEntry,
  ScenarioInfo,
  AppViewSummary,
} from "../api/types";

export function makeBox(
  overrides: Partial<ChargePointSnapshot> = {},
  live = false,
): ChargePointSnapshot {
  return {
    id: live ? "box-live" : "box-local",
    config: {
      label: live ? "Live-Box" : "Lokale Box",
      targetKind: live ? "live" : "local",
      baseUrl: live ? "wss://api.ampiera.de/ocpp" : "ws://localhost:9000/ocpp",
      identity: live ? "APLIVE0001" : "APLOCAL001",
      vendor: "Ampiera Sim",
      model: "Device Simulator",
      phases: 3,
      maxPowerW: 11000,
      supportsSoc: true,
      acceptedRateUnits: ["W", "A"],
      rejectProfiles: false,
      clockOffsetS: 0,
    },
    connection: { state: "connected", since: "2026-10-06T10:00:00Z" },
    status: "Charging",
    vehicle: null,
    powerW: 7400,
    energyWh: 12345,
    transactionId: 7,
    activeLimit: null,
    profileCount: 0,
    heartbeatIntervalS: 300,
    lastError: null,
    updatedAt: "2026-10-06T18:15:03Z",
    ...overrides,
  };
}

export function makeScenario(overrides: Partial<ScenarioInfo> = {}): ScenarioInfo {
  return {
    id: "S1",
    title: "Anmelden, Status, Heartbeat",
    description: "Die Box meldet sich an.",
    liveAllowed: true,
    needsHuman: false,
    timeoutS: 90,
    ...overrides,
  };
}

export function makeSummary(overrides: Partial<AppViewSummary> = {}): AppViewSummary {
  return {
    connection: "online",
    lastContact: "2026-10-06T10:00:00Z",
    deviceStatus: "online",
    livePowerW: 7400,
    geoState: null,
    geoHeadline: null,
    lastQuarterKwh: 1.8,
    lastQuarterAt: "2026-10-06T10:00:00Z",
    nextScheduleSteps: [],
    ...overrides,
  };
}

export function frame(
  raw: unknown,
  direction: "out" | "in" = "out",
  at = "2026-10-06T10:00:00.000Z",
): FrameLogEntry {
  return { chargePointId: "box-local", at, direction, raw: JSON.stringify(raw) };
}
