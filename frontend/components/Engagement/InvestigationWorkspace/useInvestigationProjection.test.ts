import { describe, expect, it } from "vitest";
import { acceptsProjection, type ProjectionStamp } from "./useInvestigationProjection";

function stamp(changeSeq: number, observedAsOf: string, authorityEpochSetHash = "epoch-a") {
  return {
    projectionSchemaVersion: 1,
    changeSeq,
    observedAsOf,
    authorityEpochSetHash,
  } satisfies ProjectionStamp;
}

describe("acceptsProjection", () => {
  it("rejects an older response that resolves after a newer change sequence", () => {
    const current = stamp(12, "2026-07-29T12:00:00Z");
    expect(acceptsProjection(current, stamp(11, "2026-07-29T12:01:00Z"))).toBe(false);
  });

  it("orders same-change observations only by server time and fails closed on epoch ambiguity", () => {
    const current = stamp(12, "2026-07-29T12:00:00Z", "epoch-a");
    expect(acceptsProjection(current, stamp(12, "2026-07-29T12:00:01Z", "epoch-b"))).toBe(true);
    expect(acceptsProjection(current, stamp(12, "2026-07-29T12:00:00Z", "epoch-b"))).toBe(false);
  });
});
