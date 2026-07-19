import { describe, expect, it } from "vitest";
import { GOLISH_ERROR_CODES, translateErrorCode } from "./error-codes";

describe("translateErrorCode", () => {
  it("returns a message for every canonical code", () => {
    for (const code of GOLISH_ERROR_CODES) {
      const msg = translateErrorCode(code);
      expect(msg.length).toBeGreaterThan(0);
    }
  });

  it("falls back to the raw backend message for unknown codes", () => {
    expect(translateErrorCode("SOMETHING_NEW", "raw backend text")).toBe("raw backend text");
  });

  it("falls back to a generic line when unknown and no raw message", () => {
    expect(translateErrorCode("SOMETHING_NEW")).toBe("An unexpected error occurred.");
  });

  it("explains how to unblock organization deletion", () => {
    expect(translateErrorCode("ORGANIZATION_DELETE_ACTIVE_STAGE_FORK")).toBe(
      "A stage task still has an active executor or unresolved tool outcome. Stop or recover it before deleting."
    );
  });
});
