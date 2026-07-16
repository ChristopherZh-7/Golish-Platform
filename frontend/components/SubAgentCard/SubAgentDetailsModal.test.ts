import { describe, expect, it } from "vitest";
import { getSubAgentToolDisplayStatus } from "./SubAgentDetailsModal";

describe("SubAgentDetailsModal tool status", () => {
  it("does not render a restored interrupted tool as still running", () => {
    expect(getSubAgentToolDisplayStatus("interrupted")).toBe("interrupted");
  });

  it("keeps a detached background job in its existing running presentation", () => {
    expect(getSubAgentToolDisplayStatus("backgrounded")).toBe("running");
  });
});
