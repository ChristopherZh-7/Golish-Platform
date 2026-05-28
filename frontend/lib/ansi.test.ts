import { describe, expect, it } from "vitest";
import { expandTerminalTabs } from "./ansi";

describe("expandTerminalTabs", () => {
  it("expands tabs against terminal tab stops", () => {
    expect(expandTerminalTabs("abc\tdef")).toBe("abc     def");
    expect(expandTerminalTabs("abcdefgh\tij")).toBe("abcdefgh        ij");
  });

  it("resets tab stops on new lines", () => {
    expect(expandTerminalTabs("a\tb\nab\tc")).toBe("a       b\nab      c");
  });

  it("does not count ANSI SGR sequences as visible columns", () => {
    expect(expandTerminalTabs("\x1b[32mabc\x1b[0m\tdef")).toBe(
      "\x1b[32mabc\x1b[0m     def"
    );
  });
});
