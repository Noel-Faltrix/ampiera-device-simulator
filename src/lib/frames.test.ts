import { describe, expect, it } from "vitest";
import { frame } from "../test/fixtures";
import { annotateFrames, filterFrames, prettyFrame } from "./frames";

const entries = [
  frame([2, "a1", "BootNotification", { chargePointVendor: "X" }], "out"),
  frame([3, "a1", { status: "Accepted" }], "in"),
  frame([2, "a2", "MeterValues", {}], "out"),
  frame([3, "a2", {}], "in"),
  frame([2, "c1", "ChangeConfiguration", {}], "in"),
  frame([4, "zz", "NotImplemented", "", {}], "in"),
];

describe("annotateFrames", () => {
  it("gives responses the action of the call they answer", () => {
    const result = annotateFrames(entries);
    expect(result[1]).toMatchObject({ kind: "CALLRESULT", action: "BootNotification" });
    expect(result[3]).toMatchObject({ kind: "CALLRESULT", action: "MeterValues" });
  });

  it("leaves answers to unknown calls without an action", () => {
    expect(annotateFrames(entries)[5]).toMatchObject({ kind: "CALLERROR", action: null });
  });

  it("survives text that is not an OCPP frame", () => {
    const result = annotateFrames([{ ...entries[0]!, raw: "not json" }]);
    expect(result[0]).toMatchObject({ kind: "unknown", action: null });
  });
});

describe("filterFrames", () => {
  const annotated = annotateFrames(entries);

  it("keeps everything for an empty filter", () => {
    expect(filterFrames(annotated, "  ")).toHaveLength(entries.length);
  });

  it("matches the action name case-insensitively, including the answers", () => {
    const result = filterFrames(annotated, "metervalues");
    expect(result).toHaveLength(2);
    expect(result.map((f) => f.entry.direction)).toEqual(["out", "in"]);
  });

  it("returns nothing when no action matches", () => {
    expect(filterFrames(annotated, "Heartbeat")).toEqual([]);
  });
});

describe("prettyFrame", () => {
  it("indents JSON and returns other text unchanged", () => {
    expect(prettyFrame('[2,"a","Heartbeat",{}]')).toContain("\n");
    expect(prettyFrame("plain")).toBe("plain");
  });
});
