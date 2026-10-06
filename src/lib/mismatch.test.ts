import { describe, expect, it } from "vitest";
import { makeBox, makeSummary } from "../test/fixtures";
import { compareBoxWithApp, powersAgree, type CompareRow } from "./mismatch";

function row(rows: CompareRow[], key: CompareRow["key"]): CompareRow {
  const found = rows.find((r) => r.key === key);
  if (!found) throw new Error(`row ${key} missing`);
  return found;
}

describe("powersAgree", () => {
  it("accepts values within 10 percent", () => {
    expect(powersAgree(7400, 7000)).toBe(true);
    expect(powersAgree(7400, 8100)).toBe(true);
  });

  it("rejects values further apart", () => {
    expect(powersAgree(7400, 6000)).toBe(false);
    expect(powersAgree(7400, 0)).toBe(false);
  });

  it("does not trip on rounding around zero", () => {
    expect(powersAgree(0, 20)).toBe(true);
    expect(powersAgree(0, 500)).toBe(false);
  });
});

describe("compareBoxWithApp", () => {
  it("marks equal values as matching", () => {
    const rows = compareBoxWithApp(makeBox(), makeSummary());
    expect(row(rows, "power").state).toBe("match");
    expect(row(rows, "connection").state).toBe("match");
    expect(row(rows, "status").state).not.toBe("mismatch");
  });

  it("flags a power deviation", () => {
    const rows = compareBoxWithApp(makeBox({ powerW: 7400 }), makeSummary({ livePowerW: 3000 }));
    expect(row(rows, "power").state).toBe("mismatch");
  });

  it("treats a missing app value as missing, not as a mismatch", () => {
    const rows = compareBoxWithApp(
      makeBox(),
      makeSummary({
        livePowerW: null,
        deviceStatus: null,
        geoState: null,
        connection: null,
        lastQuarterKwh: null,
      }),
    );
    for (const key of ["power", "status", "connection", "quarter"] as const) {
      expect(row(rows, key).state).toBe("missing");
      expect(row(rows, key).appValue).toBe("—");
      expect(row(rows, key).note).not.toBeNull();
    }
  });

  it("flags a connected box that the app shows as offline", () => {
    const rows = compareBoxWithApp(
      makeBox(),
      makeSummary({ deviceStatus: "offline", connection: "offline" }),
    );
    expect(row(rows, "status").state).toBe("mismatch");
    expect(row(rows, "connection").state).toBe("mismatch");
  });

  it("does not flag an offline app when the box is disconnected too", () => {
    const box = makeBox({ connection: { state: "disconnected" }, powerW: null });
    const rows = compareBoxWithApp(
      box,
      makeSummary({ deviceStatus: "offline", connection: "offline", livePowerW: null }),
    );
    expect(row(rows, "status").state).not.toBe("mismatch");
    expect(row(rows, "connection").state).toBe("match");
  });

  it("does not compare power when the box itself reports none", () => {
    const rows = compareBoxWithApp(makeBox({ powerW: null }), makeSummary({ livePowerW: 5000 }));
    expect(row(rows, "power").state).toBe("info");
  });

  it("shows the quarter-hour value with its time", () => {
    const rows = compareBoxWithApp(makeBox(), makeSummary({ lastQuarterKwh: 1.8 }));
    expect(row(rows, "quarter").appValue).toContain("1,8 kWh");
    expect(row(rows, "quarter").appValue).toContain("Uhr");
  });
});
