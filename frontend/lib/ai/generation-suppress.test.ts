import { beforeEach, describe, expect, it } from "vitest";
import {
  clearGenerationSuppressForAiSession,
  isGenerationSuppressedForAiSession,
  suppressGenerationForAiSession,
} from "./generation-suppress";

describe("generation suppression", () => {
  beforeEach(() => {
    clearGenerationSuppressForAiSession("session-a");
    clearGenerationSuppressForAiSession("session-b");
  });

  it("suppresses one AI session without affecting another", () => {
    suppressGenerationForAiSession("session-a");

    expect(isGenerationSuppressedForAiSession("session-a")).toBe(true);
    expect(isGenerationSuppressedForAiSession("session-b")).toBe(false);
  });

  it("clears suppression before the next prompt starts", () => {
    suppressGenerationForAiSession("session-a");
    clearGenerationSuppressForAiSession("session-a");

    expect(isGenerationSuppressedForAiSession("session-a")).toBe(false);
  });
});
