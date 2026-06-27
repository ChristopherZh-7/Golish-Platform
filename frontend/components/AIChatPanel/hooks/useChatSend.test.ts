import { describe, expect, it } from "vitest";
import { shouldInjectPentestSystemPrompt } from "./useChatSend";

describe("useChatSend system prompt injection", () => {
  it("keeps full pentest context only in chat mode", () => {
    expect(shouldInjectPentestSystemPrompt("chat")).toBe(true);
    expect(shouldInjectPentestSystemPrompt("assessment")).toBe(false);
    expect(shouldInjectPentestSystemPrompt("red_team")).toBe(false);
    expect(shouldInjectPentestSystemPrompt("task")).toBe(false);
  });
});
