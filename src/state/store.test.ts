import { describe, expect, it } from "vitest";
import { makeBox } from "../test/fixtures";
import { initialState, reducer } from "./store";

describe("reducer", () => {
  it("does not overwrite a snapshot that arrived by event with an older list result", () => {
    const newer = makeBox({ status: "Charging", updatedAt: "2026-10-06T18:20:00Z" });
    const older = makeBox({ status: "Available", updatedAt: "2026-10-06T18:10:00Z" });
    const afterEvent = reducer(initialState, { type: "updated", snapshot: newer });
    const afterList = reducer(afterEvent, { type: "loaded", boxes: [older] });
    expect(afterList.boxes[newer.id]?.status).toBe("Charging");
    expect(afterList.ready).toBe(true);
  });

  it("takes the list result when it is newer", () => {
    const old = makeBox({ status: "Available", updatedAt: "2026-10-06T18:10:00Z" });
    const fresh = makeBox({ status: "Charging", updatedAt: "2026-10-06T18:20:00Z" });
    const state = reducer(initialState, { type: "updated", snapshot: old });
    expect(reducer(state, { type: "loaded", boxes: [fresh] }).boxes[old.id]?.status).toBe(
      "Charging",
    );
  });

  it("caps the frames per box", () => {
    let state = initialState;
    for (let i = 0; i < 2005; i++) {
      state = reducer(state, {
        type: "frame",
        entry: { chargePointId: "a", at: "2026-10-06T10:00:00Z", direction: "out", raw: String(i) },
      });
    }
    expect(state.frames["a"]).toHaveLength(2000);
    expect(state.frames["a"]?.at(-1)?.raw).toBe("2004");
    expect(state.frames["a"]?.[0]?.raw).toBe("5");
  });
});
