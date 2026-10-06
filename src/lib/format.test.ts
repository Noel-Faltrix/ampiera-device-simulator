import { describe, expect, it } from "vitest";
import {
  formatClock,
  formatCountdown,
  formatKwh,
  formatMeterWh,
  formatPercent,
  formatPower,
  formatTimeMs,
  formatTimeout,
} from "./format";

describe("formatPower", () => {
  it("shows kilowatts with a decimal comma from 1000 W upwards", () => {
    expect(formatPower(7400)).toBe("7,4 kW");
    expect(formatPower(11000)).toBe("11 kW");
    expect(formatPower(1000)).toBe("1 kW");
  });

  it("keeps small values in watts", () => {
    expect(formatPower(850)).toBe("850 W");
    expect(formatPower(0)).toBe("0 W");
  });

  it("shows a dash for unknown power, not zero", () => {
    expect(formatPower(null)).toBe("—");
  });

  it("uses a thousands separator for large energy values", () => {
    expect(formatKwh(1234.5)).toBe("1.234,5 kWh");
    expect(formatMeterWh(12345)).toBe("12,35 kWh");
    expect(formatKwh(null)).toBe("—");
  });

  it("formats percentages the German way", () => {
    expect(formatPercent(30)).toBe("30 %");
    expect(formatPercent(30.5)).toBe("30,5 %");
  });
});

describe("formatCountdown", () => {
  const now = Date.parse("2026-10-06T10:00:00Z");

  it("counts down minutes and seconds", () => {
    expect(formatCountdown("2026-10-06T10:04:32Z", now)).toBe("endet in 4:32 min");
    expect(formatCountdown("2026-10-06T10:00:05Z", now)).toBe("endet in 0:05 min");
  });

  it("switches to hours for long limits", () => {
    expect(formatCountdown("2026-10-06T11:04:32Z", now)).toBe("endet in 1:04:32 h");
  });

  it("reports expired limits", () => {
    expect(formatCountdown("2026-10-06T10:00:00Z", now)).toBe("abgelaufen");
    expect(formatCountdown("2026-10-06T09:00:00Z", now)).toBe("abgelaufen");
  });

  it("says so when the limit has no end", () => {
    expect(formatCountdown(null, now)).toBe("ohne Ablaufzeit");
  });
});

describe("time formatting", () => {
  it("renders hh:mm:ss.SSS in local time", () => {
    const local = new Date(2026, 9, 6, 14, 3, 9, 45);
    expect(formatTimeMs(local.toISOString())).toBe("14:03:09.045");
    expect(formatClock(local.toISOString())).toBe("14:03");
  });

  it("shows a dash for broken input", () => {
    expect(formatTimeMs("nonsense")).toBe("—");
    expect(formatClock(null)).toBe("—");
  });

  it("formats scenario timeouts", () => {
    expect(formatTimeout(90)).toBe("90 s");
    expect(formatTimeout(1200)).toBe("20 min");
  });
});
