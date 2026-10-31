import { describe, expect, it } from "vitest";
import { phaseBoundaries } from "./PhaseEditor";

describe("phaseBoundaries", () => {
  it("накапливает минуты фаз как расчёт профиля", () => {
    expect(
      phaseBoundaries([
        { kind: "ramp", minutes: 10, to_pct: 100 },
        { kind: "steps", step_pct: 25, step_minutes: 5, max_pct: 100 },
        { kind: "steady", minutes: 0 },
        { kind: "steady", minutes: 30 },
      ]),
    ).toEqual([10, 30]);
    expect(phaseBoundaries([{ kind: "steady", minutes: 30 }])).toEqual([]);
    expect(phaseBoundaries([])).toEqual([]);
  });
});
