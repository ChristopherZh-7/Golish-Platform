/**
 * Warp-style interactive cell.
 *
 * Shown in place of both `RunningCommandCard` AND the bottom
 * `UnifiedInput` while the backend has flagged the running command as
 * waiting on stdin (`stdin_wait` event → `session.interactiveMode`).
 *
 * Visual layout mirrors Warp's behaviour:
 *
 *   ┌────────────────────────────────────────────┐
 *   │  $ select yn in "Yes" "No"; do ...   ●     │  ← command head
 *   ├────────────────────────────────────────────┤
 *   │  1) Yes   2) No                            │  ← live stdout
 *   │  ?# █                                       │  ← user typing
 *   ├────────────────────────────────────────────┤
 *   │  · 正在与 select 交互 · Esc 退出交互       │  ← hint footer
 *   └────────────────────────────────────────────┘
 *
 * The cell owns both the output buffer (subscribes to `terminal_output`
 * the same way `RunningCommandCard` does) and a textarea that pipes
 * directly to PTY stdin via `ptyWrite`. Enter sends `input + \n`, Esc
 * leaves interactive mode (the command keeps running — only the input
 * routing changes).
 *
 * Once the command exits (`command_end` → `setInteractiveMode(null)`)
 * the parent renders the regular `RunningCommandCard` / final
 * `CommandBlock` flow again; this cell unmounts.
 */

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Ansi } from "@/components/Ansi/Ansi";
import { stripOscSequences } from "@/lib/ansi";
import { ptyWrite } from "@/lib/api/pty";
import { onEvent } from "@/lib/events";
import { logger } from "@/lib/logger";
import { usePendingCommand, useStore } from "@/store";
import { getOutputBuffer } from "@/store/slices/session-helpers";
import type { InteractiveModeState } from "@/store/store-types";

interface InteractiveCellProps {
  sessionId: string;
  /** Active interactive-mode state from `session.interactiveMode`. */
  mode: InteractiveModeState;
  /** Command being executed (from `pendingCommand`). May be `null` if
   *  OSC 133;C did not carry a command label — we fall back to a
   *  generic "the running command" hint in that case. */
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
 * Trim a possibly multi-line compound command into a single short label
 * for `placeholder`/aria-status banners (HTML attribute values strip
 * literal newlines, so the renderer can't fully honour multi-line
 * here). Keep the first non-empty line and append an ellipsis when
 * additional lines exist, so the user can still tell at a glance which
 * command is currently waiting on stdin.
 */
function summariseCommand(command: string | null): string {
  if (!command) return "";
  const lines = command
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  if (lines.length === 0) return "";
  if (lines.length === 1) return lines[0];
  return `${lines[0]} …`;
}

function stripLeadingEcho(output: string, command: string | null): string {
  if (!command) return output;
  const cmd = command.trim();
  if (!cmd) return output;
  const lines = output.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const visible = lines[i]
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

function placeholderFor(mode: InteractiveModeState, command: string | null): string {
  // Reduce multi-line compounds (`select … do\n echo $y\n break\n done`)
  // to a one-line summary — HTML `placeholder` attribute values can't
  // carry literal newlines anyway.
  const cmd = summariseCommand(command) || summariseCommand(mode.command);
  const target = cmd ? `\`${cmd}\`` : "运行中的命令";
  switch (mode.detector) {
    case "yn_choice":
      return `回复 ${target} (Y/N)…`;
    case "password":
      return `输入密码发给 ${target}…`;
    case "powershell_choice":
      return `选择选项发给 ${target}…`;
    case "continue":
      return `回车继续 ${target}…`;
    default:
      return `回复 ${target}…`;
  }
}

function bannerLabel(command: string | null, modeCommand: string | null): string {
  const cmd = summariseCommand(command) || summariseCommand(modeCommand);
  return cmd ? `正在与 ${cmd} 交互` : "正在向运行中的命令发送输入";
}

export const InteractiveCell = memo(function InteractiveCell({
  sessionId,
  mode,
  command,
}: InteractiveCellProps) {
  const bufferRef = useRef("");
  const [output, setOutput] = useState("");
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Pending command status: when the backend has acknowledged a
  // command (`pendingCommand` populated) we know the PTY is mid-run.
  // The moment the backend drops it (command_end fired and
  // handlePromptStart flushed) the user's submitted choice has been
  // accepted and the interactive surface should yield back to the
  // normal Warp input box. Without this the cell hung around until
  // the user hit Esc themselves — confusing because the command was
  // already done.
  const pendingCommand = usePendingCommand(sessionId);

  useEffect(() => {
    // Seed from the running session's accumulated output buffer so the
    // user actually sees the prompt that triggered `stdin_wait` in the
    // first place. Without this seed the cell mounts AFTER the
    // `terminal_output` event for the prompt has already fired (the
    // detector waits 300 ms for the PTY to go idle, by which time the
    // prompt has been pushed to listeners and the previous
    // `RunningCommandCard` listener has been torn down). The seed only
    // backfills bytes that were emitted while a `pendingCommand` was
    // active — older sessions' residue is gone because `handleCommandStart`
    // deletes the buffer.
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
      unlistenPromise.then((u) => u()).catch(() => {});
    };
  }, [sessionId]);

  // Autofocus the textarea on mount so the user can start typing the
  // response immediately — Warp's defining feature is that the input
  // caret sits *right under* the prompt without an extra click.
  useEffect(() => {
    const handle = requestAnimationFrame(() => textareaRef.current?.focus());
    return () => cancelAnimationFrame(handle);
  }, []);

  // Auto-exit: when the running command finishes (pendingCommand
  // drops) close the interactive cell so the user lands back on the
  // regular input. Mirrors Warp's behaviour where the inline edit
  // disappears the moment the prompt redraws. Guarded against a
  // brief "no pending yet but we just mounted" race by requiring at
  // least one observed transition true → false. We rely on
  // `command_end` (in `useTauriEvents.ts`) already calling
  // `setInteractiveMode(sid, null)` for the common path, but a stale
  // cell can survive when the shell sends only `OSC 133;A` without
  // a matching `D` — this is the safety net.
  const sawPendingRef = useRef(false);
  useEffect(() => {
    if (pendingCommand) {
      sawPendingRef.current = true;
      return;
    }
    if (sawPendingRef.current) {
      useStore.getState().setInteractiveMode(sessionId, null);
    }
  }, [pendingCommand, sessionId]);

  const visibleOutput = useMemo(() => {
    const stripped = stripOscSequences(output);
    return stripLeadingEcho(stripped, command).replace(/^\s+/, "");
  }, [output, command]);

  // Display the command verbatim (multi-line `select … do … done` keeps
  // its original layout) so the cell visually mirrors Warp. Trim only
  // the leading/trailing whitespace so we don't introduce an empty
  // first row when the shell sent us a stray `\n` at the head.
  const displayCommand = useMemo(() => command?.replace(/^\s+|\s+$/g, "") ?? "", [command]);
  const inputPlaceholder = useMemo(() => placeholderFor(mode, command), [mode, command]);
  const banner = useMemo(() => bannerLabel(command, mode.command ?? null), [command, mode.command]);

  const submit = useCallback(() => {
    const value = input;
    setInput("");
    // Empty Enter is a legitimate "accept default" gesture for prompts
    // like `[Y/n]` so we still ship it (with a newline) to stdin.
    ptyWrite(sessionId, `${value}\n`).catch((err) => {
      logger.error("[InteractiveCell] ptyWrite failed:", err);
    });
  }, [sessionId, input]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // `isComposing` lives on the native event, not the React
      // synthetic wrapper — guard against IME pre-edits stealing
      // Enter (which should commit the candidate rather than send
      // to PTY).
      if (e.nativeEvent.isComposing) return;
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        submit();
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        // Exiting interactive mode without releasing the PTY would
        // leave the running command (`select`, `read -p`, …) blocked
        // on stdin forever — the user would then type into the
        // regular Warp input box, nothing visible would happen, and
        // they'd have to manually Ctrl-C from a separate terminal.
        // Send SIGINT to the PTY so the command actually unwinds,
        // then clear interactive state. handleCommandEnd from the
        // backend will collapse this cell in any case, but doing
        // both keeps the UI responsive even when the shell takes a
        // beat to react.
        ptyWrite(sessionId, "\x03").catch(() => {});
        useStore.getState().setInteractiveMode(sessionId, null);
        useStore.getState().handlePromptStart(sessionId);
        return;
      }
      // Ctrl-C / Ctrl-D pipe through as ASCII control bytes so the
      // running command sees the interrupt / EOF directly. We mirror
      // the behaviour of `useInputKeyboard.ts` so the muscle memory
      // works regardless of which input surface the user is in.
      if (e.ctrlKey && e.key === "c") {
        e.preventDefault();
        ptyWrite(sessionId, "\x03").catch(() => {});
        return;
      }
      if (e.ctrlKey && e.key === "d") {
        e.preventDefault();
        ptyWrite(sessionId, "\x04").catch(() => {});
        return;
      }
    },
    [sessionId, submit]
  );

  return (
    <div
      className="border border-amber-500/40 rounded-md bg-amber-500/[0.04] overflow-hidden ring-1 ring-amber-500/30"
      data-testid="interactive-cell"
      data-detector={mode.detector}
    >
      <div className="flex items-start gap-2 px-3 py-2 border-b border-amber-500/30 bg-amber-500/[0.06]">
        {/* Warp-style multi-line header: preserve the user's original
            line breaks and indentation. `block` + `whitespace-pre-wrap`
            on a `<code>` element keeps each `do` / `echo` / `done` on
            its own row while still exposing the ARIA "code" role to
            assistive tech (and existing tests). Long compounds get an
            inline scrollbar (`max-h-[200px]`) so the cell doesn't
            push the textarea off-screen. */}
        <code
          className="block m-0 flex-1 whitespace-pre-wrap break-words text-foreground max-h-[200px] overflow-auto"
          style={codeStyle}
        >
          {displayCommand ? (
            <>
              <span className="text-[var(--ansi-green)]">$ </span>
              {displayCommand}
            </>
          ) : (
            <span className="text-muted-foreground">{banner}</span>
          )}
        </code>
        <span
          className="w-2 h-2 mt-1.5 bg-amber-400 rounded-full animate-pulse flex-shrink-0"
          role="img"
          aria-label="Awaiting input"
        />
      </div>

      {visibleOutput.length > 0 && (
        <pre
          className="m-0 px-3 py-2 max-h-[360px] overflow-auto whitespace-pre-wrap break-words border-b border-amber-500/20"
          style={codeStyle}
          data-testid="interactive-cell-output"
        >
          <Ansi>{visibleOutput}</Ansi>
        </pre>
      )}

      <div className="flex items-center gap-2 px-3 py-2 bg-background/40">
        <span
          className="text-[var(--ansi-green)] flex-shrink-0 select-none"
          style={codeStyle}
          aria-hidden="true"
        >
          ›
        </span>
        <textarea
          ref={textareaRef}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={inputPlaceholder}
          rows={1}
          className="flex-1 min-h-[24px] bg-transparent border-none shadow-none resize-none font-mono text-[12px] text-foreground leading-[24px] focus:outline-none focus:ring-0 placeholder:text-muted-foreground"
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          data-testid="interactive-cell-input"
        />
      </div>

      <div
        className="flex items-center justify-between gap-2 px-3 py-1 bg-amber-500/10 border-t border-amber-500/30 text-[11px] text-amber-200"
        role="status"
        aria-live="polite"
      >
        <span className="flex items-center gap-1.5 min-w-0 truncate">
          <span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
          {banner}
        </span>
        <span className="text-amber-300/70 shrink-0">Esc 退出交互 · Ctrl-C 中断命令</span>
      </div>
    </div>
  );
});
