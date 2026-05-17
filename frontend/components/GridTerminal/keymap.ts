/**
 * Pure key-event → ANSI byte translation for the GridTerminal.
 *
 * Mirrors the xterm.js / alacritty defaults so existing TUI muscle
 * memory (vim, tmux, htop, …) keeps working. No DOM access, no React —
 * exported as plain functions so the unit tests can exercise every
 * branch without a JSDOM keyboard event factory.
 *
 * The split between *application cursor mode* (`appCursorMode = true`)
 * and *normal mode* matches alacritty: most TUIs flip on app-cursor
 * mode while running so the arrow keys send `ESC O A` instead of
 * `ESC [ A`. The backend exposes the bit via
 * `TerminalGridUpdatePayload.app_cursor_mode`.
 */

import type React from "react";

/**
 * Minimal shape extracted from `React.KeyboardEvent` so the function is
 * trivially mockable in tests (no need to construct a real React
 * synthetic event). Real call sites pass the full event but we only
 * read these properties.
 */
export interface KeymapInput {
  key: string;
  code?: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  /** True while an IME composition is in progress; we always return
   *  `null` and let the matching `compositionend` handler send the
   *  finalised string instead. */
  isComposing?: boolean;
}

/** Wrap pasted text in bracketed paste markers so PowerShell / vim
 *  can distinguish a paste from a series of fast keystrokes. */
export function bracketedPaste(text: string): string {
  return `\x1b[200~${text}\x1b[201~`;
}

/**
 * Fold the Unicode "fullwidth ASCII" block (U+FF01..U+FF5E and the
 * ideographic space U+3000) down to its ASCII equivalent. macOS users
 * who toggle CapsLock as their IME switch (or never realised pinyin
 * shipped fullwidth punctuation by default) end up sending `：` /
 * `！` / `？` to vim, which doesn't recognise any of them as command
 * triggers. The grid terminal has zero legitimate use for fullwidth
 * Latin glyphs — they belong in a textarea where the user is
 * actually composing CJK — so we normalise unconditionally here
 * rather than gating on a settings flag.
 */
export function foldFullwidthAscii(s: string): string {
  let changed = false;
  for (let i = 0; i < s.length; i++) {
    const code = s.charCodeAt(i);
    if ((code >= 0xff01 && code <= 0xff5e) || code === 0x3000) {
      changed = true;
      break;
    }
  }
  if (!changed) return s;
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const code = s.charCodeAt(i);
    if (code >= 0xff01 && code <= 0xff5e) {
      out += String.fromCharCode(code - 0xfee0);
    } else if (code === 0x3000) {
      out += " ";
    } else {
      out += s.charAt(i);
    }
  }
  return out;
}

/**
 * Translate a single keyboard event to the bytes that should be
 * written to the PTY. Returns `null` when the event should be passed
 * through to the browser (no PTY action), e.g. system shortcuts or
 * unhandled meta-key combos.
 *
 * Conventions:
 *  - Printable ASCII / Unicode → the character itself (`event.key`).
 *  - Ctrl + a..z / [ \ ] ^ _ → matching C0 control byte.
 *  - Alt + char → `ESC <char>` (xterm "metaSendsEscape" convention).
 *  - Named keys → CSI / SS3 sequences per xterm DECCKM tables.
 */
export function keyEventToAnsiBytes(event: KeymapInput, appCursorMode: boolean): string | null {
  if (event.isComposing) return null;

  // Cmd / Win key combos go to the OS — leave the browser to handle them.
  if (event.metaKey) return null;

  // Named keys come first so e.g. Ctrl+Enter still maps to a single CR.
  const named = namedKeyToBytes(event, appCursorMode);
  if (named !== null) return named;

  // Ctrl + ASCII letter / common symbol → C0 control byte.
  if (event.ctrlKey && !event.altKey && event.key.length === 1) {
    const ctrl = ctrlByte(event.key);
    if (ctrl !== null) return ctrl;
  }

  // Alt + printable → ESC <char> (e.g. Alt-F in bash readline).
  if (event.altKey && event.key.length === 1) {
    return `\x1b${foldFullwidthAscii(event.key)}`;
  }

  // Plain printable character (length === 1 catches the vast majority;
  // surrogate-pair emoji come through as length 2, which is also fine
  // to forward verbatim).
  if (!event.ctrlKey && !event.altKey && event.key.length >= 1 && !isModifierKey(event.key)) {
    return foldFullwidthAscii(event.key);
  }

  return null;
}

function isModifierKey(key: string): boolean {
  return (
    key === "Shift" ||
    key === "Control" ||
    key === "Alt" ||
    key === "Meta" ||
    key === "CapsLock" ||
    key === "NumLock" ||
    key === "ScrollLock" ||
    key === "Dead" ||
    key === "Unidentified"
  );
}

function ctrlByte(key: string): string | null {
  const ch = key.length === 1 ? key.toLowerCase() : key;
  // a..z → 0x01..0x1a
  if (ch >= "a" && ch <= "z") {
    return String.fromCharCode(ch.charCodeAt(0) - 96);
  }
  // The standard "Ctrl+symbol" punctuation table.
  switch (ch) {
    case "@":
    case " ":
      return "\x00";
    case "[":
      return "\x1b";
    case "\\":
      return "\x1c";
    case "]":
      return "\x1d";
    case "^":
      return "\x1e";
    case "_":
    case "/":
    case "?":
      return "\x1f";
    default:
      return null;
  }
}

function namedKeyToBytes(event: KeymapInput, app: boolean): string | null {
  switch (event.key) {
    case "Enter":
      return "\r";
    case "Backspace":
      // Most TUIs expect DEL (0x7f) for Backspace; Ctrl-Backspace
      // sends ^W (0x17, kill-word) like bash readline.
      return event.ctrlKey ? "\x17" : "\x7f";
    case "Tab":
      return event.shiftKey ? "\x1b[Z" : "\t";
    case "Escape":
      return "\x1b";

    case "ArrowUp":
      return arrow(app, "A", event);
    case "ArrowDown":
      return arrow(app, "B", event);
    case "ArrowRight":
      return arrow(app, "C", event);
    case "ArrowLeft":
      return arrow(app, "D", event);

    case "Home":
      return arrow(app, "H", event);
    case "End":
      return arrow(app, "F", event);

    case "PageUp":
      return `\x1b[5${modifierSuffix(event)}~`;
    case "PageDown":
      return `\x1b[6${modifierSuffix(event)}~`;
    case "Insert":
      return `\x1b[2${modifierSuffix(event)}~`;
    case "Delete":
      return `\x1b[3${modifierSuffix(event)}~`;

    case "F1":
      return "\x1bOP";
    case "F2":
      return "\x1bOQ";
    case "F3":
      return "\x1bOR";
    case "F4":
      return "\x1bOS";
    case "F5":
      return "\x1b[15~";
    case "F6":
      return "\x1b[17~";
    case "F7":
      return "\x1b[18~";
    case "F8":
      return "\x1b[19~";
    case "F9":
      return "\x1b[20~";
    case "F10":
      return "\x1b[21~";
    case "F11":
      return "\x1b[23~";
    case "F12":
      return "\x1b[24~";

    default:
      return null;
  }
}

/**
 * xterm-style modifier suffix encoding for CSI sequences that take
 * trailing parameters (`PageUp`, `Insert`, `Delete`, …). Returns
 * `""` for no-modifier, else `;<n>` per the standard table:
 *
 *   Shift = 1, Alt = 2, Shift+Alt = 3, Ctrl = 4, Shift+Ctrl = 5,
 *   Alt+Ctrl = 6, Shift+Alt+Ctrl = 7.
 */
function modifierSuffix(event: KeymapInput): string {
  const n = encodeModifiers(event);
  return n === 0 ? "" : `;${n + 1}`;
}

function encodeModifiers(event: KeymapInput): number {
  let n = 0;
  if (event.shiftKey) n += 1;
  if (event.altKey) n += 2;
  if (event.ctrlKey) n += 4;
  return n;
}

function arrow(app: boolean, final: string, event: KeymapInput): string {
  const modifiers = encodeModifiers(event);
  // With any modifier present, xterm always uses CSI form with the
  // modifier suffix regardless of app-cursor mode.
  if (modifiers !== 0) {
    return `\x1b[1;${modifiers + 1}${final}`;
  }
  return app ? `\x1bO${final}` : `\x1b[${final}`;
}

/**
 * Convenience overload that takes a React keyboard event directly.
 * Kept as a thin adapter so the underlying [`keyEventToAnsiBytes`]
 * stays test-friendly.
 */
export function reactKeyEventToAnsiBytes(
  event: React.KeyboardEvent,
  appCursorMode: boolean
): string | null {
  return keyEventToAnsiBytes(
    {
      key: event.key,
      code: event.code,
      ctrlKey: event.ctrlKey,
      altKey: event.altKey,
      shiftKey: event.shiftKey,
      metaKey: event.metaKey,
      isComposing: event.nativeEvent.isComposing,
    },
    appCursorMode
  );
}
