import { describe, expect, it } from "vitest";
import {
  IDENTITY_PATTERN,
  DEFAULT_BASE_URL,
  parseVehicleForm,
  toChargePointConfig,
  validateBaseUrl,
  validateBoxForm,
  type BoxFormValues,
} from "./validation";

const valid: BoxFormValues = {
  label: "Box 1",
  targetKind: "local",
  baseUrl: DEFAULT_BASE_URL.local,
  identity: "AP7K2M9QX4RT",
  password: "geheim",
  vendor: "Ampiera Sim",
  model: "Device Simulator",
  phases: 3,
  maxPowerKw: "11",
  supportsSoc: true,
  unitW: true,
  unitA: true,
  rejectProfiles: false,
  clockOffsetS: "0",
};

describe("validateBaseUrl", () => {
  it("allows ws only for local and private hosts", () => {
    for (const host of [
      "localhost",
      "127.0.0.1",
      "[::1]",
      "10.1.2.3",
      "172.16.0.5",
      "172.31.255.1",
      "192.168.1.20",
    ]) {
      expect(validateBaseUrl(`ws://${host}:9000/ocpp`), host).toBeNull();
    }
    for (const host of [
      "api.ampiera.de",
      "8.8.8.8",
      "172.32.0.1",
      "192.169.0.1",
      "localhost.example.com",
    ]) {
      expect(validateBaseUrl(`ws://${host}/ocpp`), host).not.toBeNull();
    }
  });

  it("always allows wss and rejects other schemes", () => {
    expect(validateBaseUrl("wss://api.ampiera.de/ocpp")).toBeNull();
    expect(validateBaseUrl("https://api.ampiera.de")).not.toBeNull();
    expect(validateBaseUrl("kein url")).not.toBeNull();
  });
});

describe("identity pattern", () => {
  it("accepts the allowed characters up to 48 chars", () => {
    expect(IDENTITY_PATTERN.test("AP7K2M9QX4RT")).toBe(true);
    expect(IDENTITY_PATTERN.test("a.b_c-1")).toBe(true);
    expect(IDENTITY_PATTERN.test("a".repeat(48))).toBe(true);
  });

  it("rejects everything else", () => {
    expect(IDENTITY_PATTERN.test("")).toBe(false);
    expect(IDENTITY_PATTERN.test("a".repeat(49))).toBe(false);
    expect(IDENTITY_PATTERN.test("mit leerzeichen")).toBe(false);
    expect(IDENTITY_PATTERN.test("über")).toBe(false);
  });
});

describe("validateBoxForm", () => {
  it("accepts the defaults with a kennung and password", () => {
    expect(validateBoxForm(valid)).toEqual({});
  });

  it("limits vendor and model to 20 characters", () => {
    const errors = validateBoxForm({ ...valid, vendor: "x".repeat(21), model: "y".repeat(21) });
    expect(errors.vendor).toBeDefined();
    expect(errors.model).toBeDefined();
    expect(validateBoxForm({ ...valid, vendor: "x".repeat(20) }).vendor).toBeUndefined();
  });

  it("requires at least one rate unit", () => {
    expect(validateBoxForm({ ...valid, unitW: false, unitA: false }).unitW).toBeDefined();
  });
});

describe("toChargePointConfig", () => {
  it("converts kW to W, strips a trailing slash and collects the units", () => {
    const config = toChargePointConfig({
      ...valid,
      maxPowerKw: "7,4",
      baseUrl: "ws://localhost:9000/ocpp/",
      unitA: false,
    });
    expect(config.maxPowerW).toBe(7400);
    expect(config.baseUrl).toBe("ws://localhost:9000/ocpp");
    expect(config.acceptedRateUnits).toEqual(["W"]);
    expect(config).not.toHaveProperty("password");
  });
});

describe("parseVehicleForm", () => {
  it("converts the form into a vehicle config", () => {
    expect(
      parseVehicleForm({ capacityKwh: "60", socPct: "30", maxPowerKw: "11", phases: 3 }),
    ).toEqual({
      ok: true,
      vehicle: { capacityKwh: 60, socPct: 30, maxPowerW: 11000, phases: 3 },
    });
  });

  it("rejects a state of charge above 100 percent", () => {
    expect(
      parseVehicleForm({ capacityKwh: "60", socPct: "130", maxPowerKw: "11", phases: 3 }).ok,
    ).toBe(false);
  });
});
