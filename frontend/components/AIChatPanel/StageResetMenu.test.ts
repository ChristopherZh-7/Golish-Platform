import { describe, expect, it } from "vitest";
import { stageResetAvailability } from "./StageResetMenu";

describe("stageResetAvailability", () => {
  it("allows only a reached Company stage while the current stage is still Company", () => {
    expect(
      stageResetAvailability("target_intel", "external_attack_surface", [
        "scoping",
        "target_intel",
      ])
    ).toEqual({ selectable: true, reason: null });
    expect(
      stageResetAvailability("external_attack_surface", "external_attack_surface", [
        "scoping",
        "target_intel",
      ])
    ).toEqual({ selectable: true, reason: null });
  });

  it("does not unlock an unvisited branch stage from its linear display position", () => {
    expect(
      stageResetAvailability("enumeration", "reporting", [
        "scoping",
        "target_intel",
        "external_attack_surface",
        "reporting",
      ])
    ).toEqual({ selectable: false, reason: "当前阶段需要新建测试任务" });
  });

  it("fails closed when the current stage is missing, finished, or unknown", () => {
    for (const current of [null, "does_not_exist"]) {
      expect(stageResetAvailability("target_intel", current, ["target_intel"])).toEqual({
        selectable: false,
        reason: "没有可原地重置的运行中阶段",
      });
    }
  });

  it("explains immutable and not-yet-reached stages instead of enabling them", () => {
    expect(stageResetAvailability("scoping", "external_attack_surface", ["scoping"])).toEqual({
      selectable: false,
      reason: "该阶段需要新建测试任务",
    });
    expect(
      stageResetAvailability("attack_candidate", "vuln_triage", ["attack_candidate"])
    ).toEqual({ selectable: false, reason: "该阶段需要新建测试任务" });
    expect(stageResetAvailability("enumeration", "external_attack_surface", [])).toEqual({
      selectable: false,
      reason: "该阶段尚未运行",
    });
  });
});
