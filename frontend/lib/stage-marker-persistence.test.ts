import { afterEach, describe, expect, it } from "vitest";
import type { ChatMessage } from "@/store/slices/conversation";
import {
  collectStageMarkers,
  readStageMarkers,
  STAGE_MARKER_STORAGE_KEY,
  spliceStageMarkers,
  writeStageMarkers,
} from "./stage-marker-persistence";

function asst(id: string): ChatMessage {
  return { id, role: "assistant", content: `c-${id}`, timestamp: 1 };
}
function sys(label: string): ChatMessage {
  return {
    id: `sys-${label}`,
    role: "system",
    content: label,
    timestamp: 1,
    stageEvent: { kind: "stage_completed", label, status: "finished" },
  };
}

describe("stage-marker-persistence", () => {
  afterEach(() => localStorage.clear());

  it("round-trips markers for a conversation", () => {
    const markers = [
      { anchorId: "a1", marker: { kind: "stage_completed" as const, label: "Stage complete: Scoping" } },
    ];
    writeStageMarkers("conv-1", markers);
    expect(readStageMarkers("conv-1")).toEqual(markers);
  });

  it("no-ops a write of an empty marker list (never clobbers)", () => {
    writeStageMarkers("conv-1", [
      { anchorId: null, marker: { kind: "subtask_completed" as const, label: "Step" } },
    ]);
    writeStageMarkers("conv-1", []);
    expect(readStageMarkers("conv-1")).toHaveLength(1);
  });

  it("returns [] for unknown conversation / corrupt payload", () => {
    expect(readStageMarkers("nope")).toEqual([]);
    localStorage.setItem(STAGE_MARKER_STORAGE_KEY, "{not json");
    expect(readStageMarkers("conv-1")).toEqual([]);
  });

  it("collectStageMarkers anchors each marker to the preceding non-system message", () => {
    const messages = [asst("a1"), sys("Stage complete: Scoping"), asst("a2"), sys("Step complete: DNS")];
    const collected = collectStageMarkers(messages);
    expect(collected).toEqual([
      { anchorId: "a1", marker: messages[1].stageEvent },
      { anchorId: "a2", marker: messages[3].stageEvent },
    ]);
  });

  it("collectStageMarkers anchors a leading marker to null", () => {
    const messages = [sys("Stage complete: Scoping"), asst("a1")];
    expect(collectStageMarkers(messages)[0].anchorId).toBeNull();
  });

  it("spliceStageMarkers re-inserts markers after their anchor in order", () => {
    const base = [asst("a1"), asst("a2")];
    const markers = collectStageMarkers([asst("a1"), sys("Stage complete: Scoping"), asst("a2")]);
    const merged = spliceStageMarkers(base, markers);
    expect(merged.map((m) => m.id)).toEqual(["a1", merged[1].id, "a2"]);
    expect(merged[1].role).toBe("system");
    expect(merged[1].content).toBe("Stage complete: Scoping");
    expect(merged[1].stageEvent?.kind).toBe("stage_completed");
  });

  it("spliceStageMarkers appends markers whose anchor is missing (never drops a milestone)", () => {
    const base = [asst("a2")];
    const markers = [
      { anchorId: "gone", marker: { kind: "stage_completed" as const, label: "Stage complete: Scoping" } },
    ];
    const merged = spliceStageMarkers(base, markers);
    expect(merged).toHaveLength(2);
    expect(merged[1].role).toBe("system");
    expect(merged[1].content).toBe("Stage complete: Scoping");
  });

  it("spliceStageMarkers is a no-op with no markers", () => {
    const base = [asst("a1")];
    expect(spliceStageMarkers(base, [])).toBe(base);
  });
});
