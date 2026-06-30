import { describe, expect, it } from "vitest";
import { stageRewindFrontierIndex } from "./StageResetMenu";

const STAGES = [
  "scoping",
  "target_intel",
  "external_attack_surface",
  "enumeration",
  "reporting",
];

describe("stageRewindFrontierIndex", () => {
  it("returns the index of the current frontier stage", () => {
    expect(stageRewindFrontierIndex(STAGES, "external_attack_surface")).toBe(2);
  });

  it("allows rewinding to any stage when there is no current frontier", () => {
    expect(stageRewindFrontierIndex(STAGES, null)).toBe(STAGES.length - 1);
  });

  it("falls back to the last stage for an unknown current stage", () => {
    expect(stageRewindFrontierIndex(STAGES, "does_not_exist")).toBe(STAGES.length - 1);
  });

  it("makes earlier + current stages rewindable and later stages locked", () => {
    const frontier = stageRewindFrontierIndex(STAGES, "external_attack_surface");
    const selectable = STAGES.map((_, i) => i <= frontier);
    // scoping, target_intel, eas selectable; enumeration + reporting locked.
    expect(selectable).toEqual([true, true, true, false, false]);
  });
});
