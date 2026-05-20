/**
 * RunningCommandCard owns the entire on-screen footprint of a running
 * command in Warp-style mode. While `pendingCommand` is non-null the
 * bottom `UnifiedInput` is unmounted (see `PaneLeaf.tsx`), so this card
 * is both the live output viewer AND the stdin pipe — exactly mirroring
 * how Warp / native terminals collapse "the running command" and "where
 * you type to it" into one rectangle.
 *
 * Keystrokes reach the PTY through a zero-size offscreen `<textarea>`:
 * - `onKeyDown` translates control keys / printable chars to byte
 *   sequences and writes them with `ptyWrite`.
 * - `onCompositionEnd` flushes IME-composed text (中文 / 日文 / 韩文).
 * - `onPaste` ships clipboard text in one write.
 * The textarea stays value-less (uncontrolled, reset on every change)
 * so it never grows or echoes locally — the PTY does the echoing.
 *
 * Focus rules:
 * - Mount does NOT steal focus (so `cargo build` doesn't yank focus
 *   from wherever the user was typing).
 * - When `interactiveMode.active` flips true (stdin_wait detector
 *   fires) we focus the hidden textarea so the user can just type
 *   `y` + Enter.
 * - Clicking anywhere on the visible card focuses the textarea too.
 *
 * A blinking block cursor is rendered at the end of the visible output
 * to show where the next keystroke will land. We deliberately don't
 * mirror the in-program cursor position — that would require a full
 * ANSI cursor-position parser and the trade-off isn't worth it for
 * Warp's "block as terminal" model where the prompt text itself ends
 * just before the cursor.
 *
 * See `docs/design/2026-05-15-warp-style-interaction.md`.
 */

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Ansi } from "@/components/Ansi/Ansi";
import { stripOscSequences } from "@/lib/ansi";
import { ptyWrite } from "@/lib/api/pty";
import { onEvent } from "@/lib/events";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { getOutputBuffer } from "@/store/slices/session-helpers";

interface RunningCommandCardProps {
  sessionId: string;
  /** The command being executed (captured from OSC 133;C). */
  command: string | null;
}

const codeStyle = {
  fontSize: "12px",
  lineHeight: 1.4,
  fontFamily: "SF Mono, Menlo, Monaco, JetBrains Mono, Consolas, monospace",
} as const;

const MAX_OUTPUT_BYTES = 256 * 1024;
const OUTPUT_FLUSH_INTERVAL_MS = 60;

/**
 * Apply the standard PTY backspace-erase sequence to a plain-text
 * buffer. When the shell is in cooked / icanon mode (which is the
 * default for `sqlmap`, `python input()`, `read -p`, etc.), pressing
 * the on-screen Backspace key makes the PTY echo back `\b \b`
 * (cursor-left, overwrite with space, cursor-left again). Anser /
 * `<pre>` don't interpret these BS / DEL bytes, so without this
 * pre-pass the user types `y`, hits backspace, and the `y` visually
 * stays on screen — which is the bug the user reported.
 *
 * We implement the conservative interpretation: each `\b` or DEL byte
 * (0x7f) deletes the previous visible character, but never crosses
 * a newline. That matches what the user perceives at single-line
 * prompts like `[Y/n] ` and is the cheap, correct subset of full
 * cursor-aware terminal emulation.
 */
function applyBackspaceErase(s: string): string {
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const ch = s[i];
    if (ch === "\b" || ch === "\x7f") {
      if (out.length > 0 && out[out.length - 1] !== "\n") {
        out = out.slice(0, -1);
      }
    } else {
      out += ch;
    }
  }
  return out;
}

/**
 * Defuse the `\r\x1b[K` / `\r\x1b[0K` (CR + erase-in-line) sequence
 * raw-mode CLI tools (sqlmap's `[Y/n]` Backspace handler, npm spinners,
 * etc.) use to redraw the current line.
 *
 * The downstream `stripOscSequences` function does its own carriage-
 * return collapsing — it splits each line on `\r` and keeps ONLY the
 * last non-empty segment (the standard "progress bar" interpretation
 * where `loading 10%\rloading 20%\r…\rloading 100%` becomes
 * `loading 100%`). That's wrong for the sqlmap shape `prompt-text…
 * \r\x1b[0K user-input` where, on a real terminal, the visible
 * outcome is `prompt-text  user-input` (CR moves the cursor home,
 * EL clears to end of line — but a half-second later the program
 * just continues writing user input on the same row without ever
 * re-rendering the prompt, leaving the original prompt bytes still
 * occupying the cells they wrote into).
 *
 * By removing only the `\r` immediately preceding an EL, we keep
 * progress-bar `\r` handling intact for every other case while
 * preventing prompt-erasure for raw-mode prompts.
 */
function defuseClearLineCarriageReturns(s: string): string {
  if (!s.includes("\r")) return s;
  return s.replace(/\r(?=\x1b\[0?K)/g, "");
}

function stripLeadingEcho(output: string, command: string | null): string {
  if (!command) return output;
  const cmd = command.trim();
  if (!cmd) return output;
  const lines = output.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const visible = lines[i]
      // Strip CSI sequences before equality check so cursor-mode toggles
      // around the echo (e.g. ConPTY's `\x1b[?25l<cmd>\x1b[?25h`) don't
      // hide the match.
      .replace(/\x1b\[[\d;?:<>= ]*[a-zA-Z]/g, "")
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, "")
      .trim();
    if (visible === "") continue;
    if (visible === cmd) {
      return lines.slice(i + 1).join("\n");
    }
    return output;
  }
  return output;
}

/**
 * Translate a React KeyboardEvent into the byte string a terminal app
 * expects on stdin. Returns `null` when the key should be ignored
 * (e.g. IME composition mid-flight, lone modifier presses, OS-level
 * Cmd/Alt chords that aren't terminal sequences). Covers the subset
 * of keys real users press at `[Y/n]` / `Password:` / `select` /
 * REPL prompts; less-common chords can be added if a real-world need
 * surfaces.
 */
function keyToPtyBytes(e: React.KeyboardEvent<Element>): string | null {
  if (e.nativeEvent?.isComposing) return null;
  const k = e.key;

  // Cmd / Alt chords belong to the OS / window manager (Cmd-C copy,
  // Alt-Tab window switch, etc.) and should never be translated into
  // PTY bytes. Ctrl chords are handled below — those *are* terminal
  // sequences in the standard ASCII control range.
  if (e.metaKey || e.altKey) {
    return null;
  }

  // Ctrl + ASCII letter → ASCII control code (Ctrl-C → 0x03 etc.).
  if (e.ctrlKey && k.length === 1) {
    const c = k.toLowerCase().charCodeAt(0);
    if (c >= 97 && c <= 122) {
      return String.fromCharCode(c - 96);
    }
  }

  switch (k) {
    case "Enter":
      return "\r";
    case "Backspace":
      // Most line-editing libraries (readline, linenoise, sqlmap's
      // input prompt) expect 0x7f (DEL), not 0x08 (BS).
      return "\x7f";
    case "Tab":
      return "\t";
    case "Escape":
      return "\x1b";
    case "ArrowUp":
      return "\x1b[A";
    case "ArrowDown":
      return "\x1b[B";
    case "ArrowRight":
      return "\x1b[C";
    case "ArrowLeft":
      return "\x1b[D";
    case "Home":
      return "\x1b[H";
    case "End":
      return "\x1b[F";
    case "PageUp":
      return "\x1b[5~";
    case "PageDown":
      return "\x1b[6~";
    case "Delete":
      return "\x1b[3~";
    default:
      if (k.length === 1 && !e.ctrlKey) {
        return k;
      }
      return null;
  }
}

export const RunningCommandCard = memo(function RunningCommandCard({
  sessionId,
  command,
}: RunningCommandCardProps) {
  // Local accumulator so we never re-render the parent timeline on
  // every PTY chunk — the ref captures bytes synchronously and a
  // 60 ms RAF/interval pump promotes them to state for the Ansi
  // renderer. Keeps `cargo build` style chatter from saturating React.
  const bufferRef = useRef("");
  const [output, setOutput] = useState("");
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Hidden zero-size textarea hosts onKeyDown / onCompositionEnd /
  // onPaste. The visible card around it has onClick → focus(textarea)
  // so the user can "click into the running terminal" to start typing.
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [hasFocus, setHasFocus] = useState(false);

  const interactiveMode = useStore((s) => s.sessions[sessionId]?.interactiveMode ?? null);
  const isInteractive = interactiveMode?.active === true;

  // Counter of characters the user has typed during the current line
  // (since entering interactive mode, or since the most recent
  // Enter). Local-echo backspace will only pop a character when this
  // counter is > 0, preventing the user from erasing into the
  // program's own prompt or earlier output.
  //
  // We deliberately count *user* input rather than anchor a buffer
  // offset: the backend's `stdin_wait` detector re-fires whenever
  // PTY output still looks like a prompt (e.g. after sqlmap echoes
  // back the user's `y`, the tail `[Y/n] y` still matches the
  // yn_choice pattern), which made the previous anchor-based scheme
  // race with the user's keystrokes — by the time the user pressed
  // Backspace the anchor had already been bumped forward to include
  // the freshly-typed `y`, so the erase predicate never fired and
  // Backspace silently no-op'd.
  const interactiveInputCountRef = useRef(0);

  useEffect(() => {
    // Seed from the session's accumulated output buffer so we don't
    // miss bytes that arrived *between* OSC 133;C firing (which sets
    // pendingCommand → triggers our mount) and our `terminal_output`
    // subscription becoming live. Without this, long-running programs
    // like sqlmap that stream a banner / disclaimer / progress before
    // hitting their first interactive prompt would show an empty
    // RunningCommandCard until the detector trips, then dump all the
    // backlog at once — exactly the user-reported regression.
    const seed = getOutputBuffer(sessionId);
    bufferRef.current = seed;
    setOutput(seed);

    let cancelled = false;
    const unlistenPromise = onEvent("terminal_output", (payload) => {
      if (cancelled) return;
      if (payload.session_id !== sessionId) return;
      const next = bufferRef.current + payload.data;
      bufferRef.current =
        next.length > MAX_OUTPUT_BYTES ? next.slice(next.length - MAX_OUTPUT_BYTES) : next;
      if (flushTimerRef.current !== null) return;
      flushTimerRef.current = setTimeout(() => {
        flushTimerRef.current = null;
        setOutput(bufferRef.current);
      }, OUTPUT_FLUSH_INTERVAL_MS);
    });

    return () => {
      cancelled = true;
      if (flushTimerRef.current !== null) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch(() => {
          // listener already cleaned up — ignore.
        });
    };
  }, [sessionId]);

  // Auto-focus the hidden textarea when stdin_wait fires. The
  // detector having tripped is the strongest signal that the user
  // wants to type *right now*, so stealing focus is acceptable here
  // even though we deliberately *don't* steal it on mount.
  //
  // CRITICAL: counter is reset ONLY on the false→true transition
  // (entering interactive mode) or on true→false (leaving it). The
  // backend's `stdin_wait` detector re-fires every time PTY tail
  // still looks like a prompt — even after the user's `y` echoes
  // back, since the tail `[Y/n] y` still matches the yn_choice
  // regex. Each re-fire bumps `enteredAt`, so if we tied the reset
  // to `enteredAt` (which an earlier draft did) the counter would
  // get clobbered to 0 between the keystroke and the very next
  // Backspace press — exactly the race the console traces caught.
  // We track the previous active state explicitly so a re-fire
  // while already active is a no-op.
  const prevInteractiveActiveRef = useRef(false);
  useEffect(() => {
    const isActive = interactiveMode?.active === true;
    if (isActive) {
      inputRef.current?.focus({ preventScroll: true });
      if (!prevInteractiveActiveRef.current) {
        interactiveInputCountRef.current = 0;
      }
    } else {
      interactiveInputCountRef.current = 0;
    }
    prevInteractiveActiveRef.current = isActive;
  }, [interactiveMode?.active, interactiveMode?.enteredAt]);

  const visibleOutput = useMemo(() => {
    // Order matters: defuse `\r\x1b[K` BEFORE stripOscSequences so its
    // line-internal `\r`-split-and-take-last logic doesn't drop the
    // prompt segment (see fn comment for the long story).
    const defused = defuseClearLineCarriageReturns(output);
    const stripped = stripOscSequences(defused);
    const erased = applyBackspaceErase(stripped);
    return stripLeadingEcho(erased, command).replace(/^\s+/, "");
  }, [output, command]);

  // Trim leading/trailing whitespace only — keep internal `\n` and
  // indentation intact so the Warp-style header can render multi-line
  // compound commands (`select … do … done`, heredocs, …) on their
  // original rows.
  const displayCommand = useMemo(() => command?.replace(/^\s+|\s+$/g, "") ?? "", [command]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      const bytes = keyToPtyBytes(e);
      if (bytes === null) return;
      // Always prevent default so the textarea doesn't accumulate the
      // typed character — we treat it as a write-only stdin channel.
      e.preventDefault();

      // Local echo logic — only active while the program is waiting
      // on stdin (sqlmap, [Y/n] prompts, etc.). Outside interactive
      // mode the PTY's own echo is authoritative.
      if (isInteractive) {
        if (bytes === "\x7f") {
          // Backspace: the backend's `apply_backspaces` pre-pass
          // collapses the PTY's `\b \b` round-trip (and some raw-mode
          // tools like sqlmap echo `\r\x1b[0K` instead), so neither
          // round-trip erases the `y` we already shipped to the
          // visible buffer. We pop it ourselves, clamped to the
          // counter of characters the user has typed during the
          // current line. Counter > 0 guards against erasing into
          // the program's own prompt or earlier output.
          if (interactiveInputCountRef.current > 0) {
            interactiveInputCountRef.current -= 1;
            const buf = bufferRef.current;
            // Pop the most-recent user-typed printable character. The
            // tricky bit is the buffer tail may already contain a
            // complete CSI sequence (e.g. `\r\x1b[0K` from a sqlmap
            // re-prompt). Naively chopping the last char would eat
            // the `K` and leave a half-eaten `\x1b[0` that the next
            // paint renders as a literal `0`. So we walk past any
            // non-printable suffix AND skip whole CSI sequences
            // before deleting a real user character.
            let popIdx = buf.length - 1;
            while (popIdx >= 0) {
              const ch = buf[popIdx];
              if (ch === "\r" || ch === "\n") {
                popIdx -= 1;
                continue;
              }
              if (/[a-zA-Z]/.test(ch)) {
                let j = popIdx - 1;
                while (j >= 0 && /[\d;?:<>= ]/.test(buf[j])) j--;
                if (j >= 1 && buf[j] === "[" && buf[j - 1] === "\x1b") {
                  popIdx = j - 2;
                  continue;
                }
              }
              const code = buf.charCodeAt(popIdx);
              if (code >= 0x20 && code < 0x7f) {
                bufferRef.current = buf.slice(0, popIdx) + buf.slice(popIdx + 1);
                setOutput(bufferRef.current);
                break;
              }
              popIdx -= 1;
            }
          }
        } else if (bytes === "\r") {
          interactiveInputCountRef.current = 0;
        } else if (bytes.length === 1) {
          // Single-byte printable / control char the user typed. We
          // intentionally count Ctrl-* sequences too: they're stdin
          // input from the PTY's perspective and the user's Backspace
          // shouldn't reach beyond them.
          interactiveInputCountRef.current += 1;
        } else if (bytes.startsWith("\x1b")) {
          // Arrow keys / function keys etc. don't add a visible char
          // to the line buffer in the typical readline-cooked-mode
          // host, so they don't move the counter.
        }
      }

      ptyWrite(sessionId, bytes).catch((err) =>
        console.error("[RunningCommandCard] ptyWrite failed:", err)
      );
    },
    [sessionId, isInteractive]
  );

  const handleCompositionEnd = useCallback(
    (e: React.CompositionEvent<HTMLTextAreaElement>) => {
      const composed = e.data;
      if (composed) {
        // Count the IME-composed characters as user input so a
        // subsequent Backspace can erase them locally. We count the
        // last segment after any embedded newlines because Enter
        // submits the current line and resets the counter.
        if (isInteractive) {
          const lastSegment = composed.split(/\r?\n/).pop() ?? composed;
          if (composed.includes("\n") || composed.includes("\r")) {
            interactiveInputCountRef.current = lastSegment.length;
          } else {
            interactiveInputCountRef.current += [...composed].length;
          }
        }
        ptyWrite(sessionId, composed).catch((err) =>
          console.error("[RunningCommandCard] composition ptyWrite failed:", err)
        );
      }
      // Reset the textarea so the next composition starts from empty
      // (the value would otherwise grow to include every accepted run).
      if (inputRef.current) {
        inputRef.current.value = "";
      }
    },
    [sessionId, isInteractive]
  );

  // Catch paste events explicitly — shipping the clipboard bytes
  // immediately is faster than waiting for a React state round-trip
  // via onChange, and avoids ever showing the pasted text inside our
  // (intentionally invisible) capture textarea.
  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const text = e.clipboardData.getData("text");
      if (!text) return;
      e.preventDefault();
      // Count pasted characters as user input. If the paste contains
      // a newline we treat it as the user submitting every line up
      // to the last \n and starting fresh on the trailing fragment.
      if (isInteractive) {
        const lastSegment = text.split(/\r?\n/).pop() ?? text;
        if (text.includes("\n") || text.includes("\r")) {
          interactiveInputCountRef.current = lastSegment.length;
        } else {
          interactiveInputCountRef.current += [...text].length;
        }
      }
      ptyWrite(sessionId, text).catch((err) =>
        console.error("[RunningCommandCard] paste ptyWrite failed:", err)
      );
      if (inputRef.current) {
        inputRef.current.value = "";
      }
    },
    [sessionId, isInteractive]
  );

  const focusInput = useCallback(() => {
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  return (
    <div
      role="group"
      aria-label={isInteractive ? "Running command — waiting for your input" : "Running command"}
      className={cn(
        "relative border border-[var(--border-subtle)] rounded-md bg-card/30 overflow-hidden",
        "cursor-text",
        (hasFocus || isInteractive) && "ring-1 ring-accent/50"
      )}
      onClick={focusInput}
      onKeyDown={(e) => {
        // Bubbles from the textarea — no-op here, the textarea handler
        // already shipped the bytes. We keep the listener so clicks
        // that *aren't* keystrokes (e.g. focus-only) still reach the
        // wrapping div.
        void e;
      }}
      data-testid="running-command-card"
      data-interactive={isInteractive ? "true" : "false"}
    >
      <div className="flex items-start gap-2 px-3 py-2 border-b border-[var(--border-subtle)]/60">
        {/* `block` + `whitespace-pre-wrap` on `<code>` preserves both
            internal newlines and indentation, mirroring how Warp shows
            a running compound command. `max-h-[200px]` keeps a heredoc
            or long pipeline from pushing the live output off-screen;
            the inline scrollbar lets the user inspect the full text.
            Sticking with `<code>` keeps the ARIA "code" role used by
            existing UnifiedTimeline tests. */}
        <code
          className="block m-0 flex-1 whitespace-pre-wrap break-words text-foreground max-h-[200px] overflow-auto"
          style={codeStyle}
        >
          <span className="text-[var(--ansi-green)]">$ </span>
          {displayCommand}
        </code>
        {isInteractive ? (
          // Amber pill — Warp's "I am waiting for your input" affordance.
          // Renders next to the running dot so the user can tell at a
          // glance that the card is paused on stdin (vs. just streaming
          // output of a long-running build).
          <span
            className="flex-shrink-0 inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border border-amber-400/40 bg-amber-400/10 text-amber-300"
            role="status"
            aria-label="Waiting for input"
            data-testid="running-command-card-waiting"
          >
            <span
              aria-hidden="true"
              className="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse"
            />
            等待输入
          </span>
        ) : (
          <span
            className="w-2 h-2 mt-1.5 bg-[#7aa2f7] rounded-full animate-pulse flex-shrink-0"
            role="img"
            aria-label="Running"
          />
        )}
      </div>

      <pre
        className="m-0 px-3 py-2 max-h-[480px] overflow-auto whitespace-pre-wrap break-words"
        style={codeStyle}
      >
        {visibleOutput.length > 0 && <Ansi>{visibleOutput}</Ansi>}
        {/* Inline blinking block cursor parked at the tail of the
            output. We don't try to mirror in-program cursor position
            (would require a real ANSI parser); the visible end of the
            output buffer is where the user's next keystroke will be
            echoed by the PTY anyway, so the visual is honest enough
            for [Y/n] / Password / select prompts which sit on the
            last line. */}
        <span
          aria-hidden="true"
          className="inline-block align-baseline ml-[1px]"
          style={{
            width: "0.55em",
            height: "1em",
            backgroundColor: "var(--foreground)",
            opacity: hasFocus || isInteractive ? 0.85 : 0.35,
            verticalAlign: "text-bottom",
            animation: "blink 1s step-end infinite",
          }}
        />
      </pre>

      {/* Zero-size capture textarea, positioned INSIDE the card so
          the browser never has to scroll outside the viewport to
          reveal it when focus lands here. An earlier revision parked
          this offscreen at `top: -9999` which made every keystroke
          jump the page to the top — the browser was scrolling to
          satisfy "focused element must be visible" even though we
          passed `preventScroll: true`. `pointer-events: none` keeps
          mouse interactions on the visible card; programmatic
          `focus()` still works regardless. */}
      <textarea
        ref={inputRef}
        aria-label="Send keys to running command"
        tabIndex={-1}
        style={{
          position: "absolute",
          width: 1,
          height: 1,
          bottom: 0,
          right: 0,
          opacity: 0,
          pointerEvents: "none",
          border: "none",
          padding: 0,
          margin: 0,
          resize: "none",
        }}
        onKeyDown={handleKeyDown}
        onCompositionEnd={handleCompositionEnd}
        onPaste={handlePaste}
        onFocus={() => setHasFocus(true)}
        onBlur={() => setHasFocus(false)}
        onChange={(e) => {
          // Non-composition input rarely fires (we preventDefault on
          // every onKeyDown), but IME on some platforms emits an
          // onChange right after compositionend. Clearing keeps the
          // value bounded so we never re-ship the same bytes twice.
          e.target.value = "";
        }}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
      />
    </div>
  );
});
