import { describe, expect, it } from "vitest";
import {
  isDetailFocusMode,
  shouldHideAiChatPanel,
  shouldMountAiChatPanel,
} from "./detailFocus";

describe("isDetailFocusMode", () => {
  it("focuses tool and sub-agent details, but not the terminal timeline", () => {
    expect(isDetailFocusMode("tool-detail")).toBe(true);
    expect(isDetailFocusMode("sub-agent-detail")).toBe(true);
    expect(isDetailFocusMode("timeline")).toBe(false);
    expect(isDetailFocusMode(undefined)).toBe(false);
  });

  it("keeps ChatPanel mounted but visually hidden while details own the workspace", () => {
    expect(shouldMountAiChatPanel(false)).toBe(true);
    expect(shouldHideAiChatPanel(true, "tool-detail")).toBe(true);
    expect(shouldHideAiChatPanel(true, "sub-agent-detail")).toBe(true);
    expect(shouldHideAiChatPanel(false, "timeline")).toBe(true);
    expect(shouldHideAiChatPanel(true, "timeline")).toBe(false);
    expect(shouldMountAiChatPanel(true)).toBe(false);
  });
});
