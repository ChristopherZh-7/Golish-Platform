import { describe, expect, it } from "vitest";
import { isNearScrollBottom, shouldStickToBottomAfterScroll } from "./scroll-stickiness";

describe("scroll stickiness", () => {
  it("treats positions within the threshold as near the bottom", () => {
    expect(
      isNearScrollBottom({
        scrollTop: 804,
        scrollHeight: 1000,
        clientHeight: 120,
      })
    ).toBe(true);
  });

  it("does not re-enable stickiness when the user scrolls upward near the bottom", () => {
    expect(
      shouldStickToBottomAfterScroll(850, {
        scrollTop: 830,
        scrollHeight: 1000,
        clientHeight: 120,
      })
    ).toBe(false);
  });

  it("re-enables stickiness after the user scrolls back to the bottom", () => {
    expect(
      shouldStickToBottomAfterScroll(760, {
        scrollTop: 880,
        scrollHeight: 1000,
        clientHeight: 120,
      })
    ).toBe(true);
  });
});
