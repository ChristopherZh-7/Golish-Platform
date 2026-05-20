import { describe, expect, it } from "vitest";
import {
  bracketedPaste,
  foldFullwidthAscii,
  type KeymapInput,
  keyEventToAnsiBytes,
} from "./keymap";

function key(overrides: Partial<KeymapInput> & { key: string }): KeymapInput {
  return {
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    isComposing: false,
    ...overrides,
  };
}

describe("keyEventToAnsiBytes · printable input", () => {
  it("returns the character verbatim for plain ASCII", () => {
    expect(keyEventToAnsiBytes(key({ key: "a" }), false)).toBe("a");
    expect(keyEventToAnsiBytes(key({ key: "Z" }), false)).toBe("Z");
    expect(keyEventToAnsiBytes(key({ key: "5" }), false)).toBe("5");
    expect(keyEventToAnsiBytes(key({ key: "+" }), false)).toBe("+");
  });

  it("forwards Unicode and emoji unchanged", () => {
    expect(keyEventToAnsiBytes(key({ key: "你" }), false)).toBe("你");
    expect(keyEventToAnsiBytes(key({ key: "🎉" }), false)).toBe("🎉");
  });

  it("returns null while IME composition is active", () => {
    expect(
      keyEventToAnsiBytes(key({ key: "a", isComposing: true }), false)
    ).toBeNull();
  });

  it("ignores Cmd / Win modifier combos so the browser handles them", () => {
    expect(keyEventToAnsiBytes(key({ key: "c", metaKey: true }), false)).toBeNull();
  });

  it("ignores bare modifier-down events", () => {
    expect(keyEventToAnsiBytes(key({ key: "Shift" }), false)).toBeNull();
    expect(keyEventToAnsiBytes(key({ key: "Control" }), false)).toBeNull();
    expect(keyEventToAnsiBytes(key({ key: "Alt" }), false)).toBeNull();
    expect(keyEventToAnsiBytes(key({ key: "Meta" }), false)).toBeNull();
  });
});

describe("keyEventToAnsiBytes · control / alt", () => {
  it("Ctrl+letter maps to the matching C0 byte", () => {
    expect(keyEventToAnsiBytes(key({ key: "c", ctrlKey: true }), false)).toBe("\x03");
    expect(keyEventToAnsiBytes(key({ key: "d", ctrlKey: true }), false)).toBe("\x04");
    expect(keyEventToAnsiBytes(key({ key: "z", ctrlKey: true }), false)).toBe("\x1a");
  });

  it("Ctrl+[ → ESC, Ctrl+] → GS, Ctrl+/ → US", () => {
    expect(keyEventToAnsiBytes(key({ key: "[", ctrlKey: true }), false)).toBe("\x1b");
    expect(keyEventToAnsiBytes(key({ key: "]", ctrlKey: true }), false)).toBe("\x1d");
    expect(keyEventToAnsiBytes(key({ key: "/", ctrlKey: true }), false)).toBe("\x1f");
  });

  it("Alt+letter prefixes ESC", () => {
    expect(keyEventToAnsiBytes(key({ key: "f", altKey: true }), false)).toBe("\x1bf");
    expect(keyEventToAnsiBytes(key({ key: ".", altKey: true }), false)).toBe("\x1b.");
  });
});

describe("keyEventToAnsiBytes · named keys", () => {
  it("Enter → CR, Backspace → DEL, Tab / Shift-Tab", () => {
    expect(keyEventToAnsiBytes(key({ key: "Enter" }), false)).toBe("\r");
    expect(keyEventToAnsiBytes(key({ key: "Backspace" }), false)).toBe("\x7f");
    expect(keyEventToAnsiBytes(key({ key: "Tab" }), false)).toBe("\t");
    expect(keyEventToAnsiBytes(key({ key: "Tab", shiftKey: true }), false)).toBe(
      "\x1b[Z"
    );
  });

  it("Ctrl-Backspace sends ETB (kill-word)", () => {
    expect(keyEventToAnsiBytes(key({ key: "Backspace", ctrlKey: true }), false)).toBe(
      "\x17"
    );
  });

  it("F1-F4 use SS3, F5+ use CSI ~", () => {
    expect(keyEventToAnsiBytes(key({ key: "F1" }), false)).toBe("\x1bOP");
    expect(keyEventToAnsiBytes(key({ key: "F4" }), false)).toBe("\x1bOS");
    expect(keyEventToAnsiBytes(key({ key: "F5" }), false)).toBe("\x1b[15~");
    expect(keyEventToAnsiBytes(key({ key: "F12" }), false)).toBe("\x1b[24~");
  });

  it("PageUp / PageDown / Insert / Delete", () => {
    expect(keyEventToAnsiBytes(key({ key: "PageUp" }), false)).toBe("\x1b[5~");
    expect(keyEventToAnsiBytes(key({ key: "PageDown" }), false)).toBe("\x1b[6~");
    expect(keyEventToAnsiBytes(key({ key: "Insert" }), false)).toBe("\x1b[2~");
    expect(keyEventToAnsiBytes(key({ key: "Delete" }), false)).toBe("\x1b[3~");
  });

  it("Insert with Shift gains a `;2` modifier suffix", () => {
    expect(
      keyEventToAnsiBytes(key({ key: "Insert", shiftKey: true }), false)
    ).toBe("\x1b[2;2~");
  });
});

describe("keyEventToAnsiBytes · arrow keys", () => {
  it("normal mode uses CSI form", () => {
    expect(keyEventToAnsiBytes(key({ key: "ArrowUp" }), false)).toBe("\x1b[A");
    expect(keyEventToAnsiBytes(key({ key: "ArrowDown" }), false)).toBe("\x1b[B");
    expect(keyEventToAnsiBytes(key({ key: "ArrowRight" }), false)).toBe("\x1b[C");
    expect(keyEventToAnsiBytes(key({ key: "ArrowLeft" }), false)).toBe("\x1b[D");
    expect(keyEventToAnsiBytes(key({ key: "Home" }), false)).toBe("\x1b[H");
    expect(keyEventToAnsiBytes(key({ key: "End" }), false)).toBe("\x1b[F");
  });

  it("application cursor mode uses SS3 form", () => {
    expect(keyEventToAnsiBytes(key({ key: "ArrowUp" }), true)).toBe("\x1bOA");
    expect(keyEventToAnsiBytes(key({ key: "ArrowDown" }), true)).toBe("\x1bOB");
    expect(keyEventToAnsiBytes(key({ key: "ArrowRight" }), true)).toBe("\x1bOC");
    expect(keyEventToAnsiBytes(key({ key: "ArrowLeft" }), true)).toBe("\x1bOD");
    expect(keyEventToAnsiBytes(key({ key: "Home" }), true)).toBe("\x1bOH");
    expect(keyEventToAnsiBytes(key({ key: "End" }), true)).toBe("\x1bOF");
  });

  it("modified arrows always use CSI 1; form even in app-cursor mode", () => {
    expect(keyEventToAnsiBytes(key({ key: "ArrowUp", shiftKey: true }), true)).toBe(
      "\x1b[1;2A"
    );
    expect(keyEventToAnsiBytes(key({ key: "ArrowLeft", ctrlKey: true }), true)).toBe(
      "\x1b[1;5D"
    );
    expect(
      keyEventToAnsiBytes(
        key({ key: "ArrowRight", ctrlKey: true, shiftKey: true }),
        true
      )
    ).toBe("\x1b[1;6C");
  });
});

describe("bracketedPaste", () => {
  it("wraps the payload with the standard BPM markers", () => {
    expect(bracketedPaste("hello\nworld")).toBe(
      "\x1b[200~hello\nworld\x1b[201~"
    );
    expect(bracketedPaste("")).toBe("\x1b[200~\x1b[201~");
  });
});

describe("foldFullwidthAscii", () => {
  it("folds fullwidth ASCII into the standard ASCII block", () => {
    expect(foldFullwidthAscii("：")).toBe(":");
    expect(foldFullwidthAscii("！")).toBe("!");
    expect(foldFullwidthAscii("？")).toBe("?");
    expect(foldFullwidthAscii("（）")).toBe("()");
    expect(foldFullwidthAscii("Ａ")).toBe("A");
  });

  it("normalises the ideographic space", () => {
    expect(foldFullwidthAscii("\u3000")).toBe(" ");
  });

  it("leaves non-fullwidth strings untouched", () => {
    expect(foldFullwidthAscii("hello")).toBe("hello");
    expect(foldFullwidthAscii("你好")).toBe("你好");
    expect(foldFullwidthAscii("")).toBe("");
  });
});

describe("keyEventToAnsiBytes · fullwidth ASCII fold", () => {
  it("Chinese pinyin's fullwidth colon becomes vim-friendly ASCII", () => {
    expect(keyEventToAnsiBytes(key({ key: "：" }), false)).toBe(":");
    expect(keyEventToAnsiBytes(key({ key: "！" }), false)).toBe("!");
    expect(keyEventToAnsiBytes(key({ key: "？" }), false)).toBe("?");
  });

  it("Alt+fullwidth also folds before the ESC prefix", () => {
    expect(keyEventToAnsiBytes(key({ key: "．", altKey: true }), false)).toBe("\x1b.");
  });
});
