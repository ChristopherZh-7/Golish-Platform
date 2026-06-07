import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { filterCommands } from "@/components/SlashCommandPopup";
import { useCommandHistory } from "@/hooks/useCommandHistory";
import { useFileCommands } from "@/hooks/useFileCommands";
import { type HistoryMatch, useHistorySearch } from "@/hooks/useHistorySearch";
import { usePathCompletion } from "@/hooks/usePathCompletion";
import { type SlashCommand, useSlashCommands } from "@/hooks/useSlashCommands";
import { useToolSearch } from "@/hooks/useToolSearch";
import type { FileInfo } from "@/lib/api/files";
import { ptyWrite } from "@/lib/api/pty";
import type { PathCompletion } from "@/lib/api/shell";
import { imeGetSource, imeSetSource } from "@/lib/api/shell";
import { notify } from "@/lib/notify";
import type { ToolConfig } from "@/lib/pentest/types";
import { type CaretSettings, DEFAULT_CARET_SETTINGS, getSettings } from "@/lib/settings";
import {
  clearTerminalAutoFocusSuppression,
  isTerminalAutoFocusSuppressed,
} from "@/lib/terminal/terminalAutoFocus";
import { usePendingCommand, useStore } from "@/store";
import { useUnifiedInputState as useStoreInputState } from "@/store/selectors/unified-input";

export interface ToolParam {
  label: string;
  flag: string;
  type: string;
  required?: boolean;
  placeholder?: string;
  default?: string | number | boolean;
  options?: { value: string; label: string }[];
  description?: string;
}

export function extractWordAtCursor(
  input: string,
  cursorPos: number
): { word: string; startIndex: number } {
  const beforeCursor = input.slice(0, cursorPos);
  const match = beforeCursor.match(/[^\s|;&]+$/);
  if (!match) return { word: "", startIndex: cursorPos };
  return { word: match[0], startIndex: cursorPos - match[0].length };
}

export function isCursorOnFirstLine(text: string, cursorPos: number): boolean {
  return !text.substring(0, cursorPos).includes("\n");
}

export function isCursorOnLastLine(text: string, cursorPos: number): boolean {
  return !text.substring(cursorPos).includes("\n");
}

export const clearTerminal = (sessionId: string) => {
  const store = useStore.getState();
  store.clearBlocks(sessionId);
  store.requestTerminalClear(sessionId);
};

export interface StateRefValue {
  input: string;
  inputMode: "terminal";
  showSlashPopup: boolean;
  filteredSlashCommands: SlashCommand[];
  slashSelectedIndex: number;
  showFilePopup: boolean;
  files: FileInfo[];
  fileSelectedIndex: number;
  showPathPopup: boolean;
  pathCompletions: PathCompletion[];
  pathSelectedIndex: number;
  showHistorySearch: boolean;
  historySearchQuery: string;
  historyMatches: HistoryMatch[];
  historySelectedIndex: number;
  originalInput: string;
  commands: SlashCommand[];
  showToolPopup: boolean;
  toolMatches: ToolConfig[];
  toolSelectedIndex: number;
  activeTool: ToolConfig | null;
}

export function useInputState({
  sessionId,
  desiredHeight = 0,
}: {
  sessionId: string;
  /**
   * Height (in px) the user has dragged the input panel to. `0` means
   * "user has never dragged" → legacy auto-grow (one line, cap 200px).
   * `>0` means textarea must be at least this tall, up to 800px.
   */
  desiredHeight?: number;
}) {
  const { t } = useTranslation();
  const workingDirectory = useStore((state) => state.sessions[sessionId]?.workingDirectory);

  const [input, setInput] = useState("");
  const [showSlashPopup, setShowSlashPopup] = useState(false);
  const [slashSelectedIndex, setSlashSelectedIndex] = useState(0);
  const [showFilePopup, setShowFilePopup] = useState(false);
  const [fileSelectedIndex, setFileSelectedIndex] = useState(0);
  const [showPathPopup, setShowPathPopup] = useState(false);
  const [pathSelectedIndex, setPathSelectedIndex] = useState(0);
  const [pathQuery, setPathQuery] = useState("");
  const [showHistorySearch, setShowHistorySearch] = useState(false);
  const [historySearchQuery, setHistorySearchQuery] = useState("");
  const [historySelectedIndex, setHistorySelectedIndex] = useState(0);
  const [originalInput, setOriginalInput] = useState("");
  const [showToolPopup, setShowToolPopup] = useState(false);
  const [toolSelectedIndex, setToolSelectedIndex] = useState(0);
  const [activeTool, setActiveTool] = useState<ToolConfig | null>(null);
  const [toolParams, setToolParams] = useState<ToolParam[]>([]);
  const [isFocused, setIsFocused] = useState(false);
  const [caretSettings, setCaretSettings] = useState<CaretSettings>(DEFAULT_CARET_SETTINGS);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const dropZoneRef = useRef<HTMLDivElement>(null);
  const inputContainerRef = useRef<HTMLDivElement>(null);
  const prevImeSourceRef = useRef<string | null>(null);

  const { isSessionDead } = useStoreInputState(sessionId);
  const inputMode = "terminal" as const;
  const pendingCommand = usePendingCommand(sessionId);

  // Warp-style interactive input mode: when the backend `stdin_wait`
  // detector fires for the running command, the textarea can route
  // keystrokes straight to the PTY (Enter sends `input + \n` via
  // `ptyWrite` without creating a new command block) as a fallback
  // stdin sink. `RunningCommandCard` is the primary sink (its own
  // offscreen capture textarea takes focus), but `UnifiedInput` keeps
  // the UI affordance (orange ring / banner) and offers a secondary
  // path for keystrokes when the card hasn't focused yet.
  const interactiveMode = useStore((state) => state.sessions[sessionId]?.interactiveMode) ?? null;
  const isInteractive = !!interactiveMode?.active;

  // `isProcessRunning` controls focus/blur for the textarea — when a
  // command is running we normally drop focus so keystrokes don't pile
  // up uselessly. Interactive mode is the explicit exception: a
  // command IS running, but those keystrokes are the whole point.
  const isProcessRunning = !!pendingCommand && !isInteractive;

  useEffect(() => {
    getSettings().then((s) => setCaretSettings(s.terminal.caret ?? DEFAULT_CARET_SETTINGS));
    const onSettingsUpdated = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.terminal?.caret) {
        setCaretSettings(detail.terminal.caret);
      }
    };
    window.addEventListener("settings-updated", onSettingsUpdated);
    return () => window.removeEventListener("settings-updated", onSettingsUpdated);
  }, []);

  const isBlockCaret = caretSettings.style === "block";
  const setLastSentCommand = useStore((state) => state.setLastSentCommand);

  const {
    history,
    add: addToHistory,
    navigateUp,
    navigateDown,
    reset: resetHistory,
  } = useCommandHistory({ entryType: inputMode === "terminal" ? "cmd" : "prompt" });

  const { matches: historyMatches } = useHistorySearch({ history, query: historySearchQuery });

  const { commands } = useSlashCommands(workingDirectory);
  const slashInput = input.startsWith("/") ? input.slice(1) : "";
  const slashSpaceIndex = slashInput.indexOf(" ");
  const slashCommandName =
    slashSpaceIndex === -1 ? slashInput : slashInput.slice(0, slashSpaceIndex);
  const filteredSlashCommands = useMemo(
    () => filterCommands(commands, slashCommandName),
    [commands, slashCommandName]
  );

  const atMatch = input.match(/@([^\s@]*)$/);
  const fileQuery = atMatch?.[1] ?? "";
  const { files } = useFileCommands(workingDirectory, fileQuery);

  const isToolSearchMode = inputMode === "terminal" && /^\/t\s/i.test(input);
  const toolSearchQuery = isToolSearchMode ? input.replace(/^\/t\s+/i, "") : "";
  const toolSearchEnabled = isToolSearchMode && toolSearchQuery.length > 0 && !activeTool;
  const { matches: toolMatches } = useToolSearch(toolSearchQuery, toolSearchEnabled);

  const { completions: pathCompletions, totalCount: pathTotalCount } = usePathCompletion({
    sessionId,
    partialPath: pathQuery,
    enabled: showPathPopup && inputMode === "terminal",
  });

  const ghostText = useMemo(() => {
    if (!showPathPopup || pathCompletions.length === 0 || !pathQuery) return "";
    const completion = pathCompletions[pathSelectedIndex] || pathCompletions[0];
    const nameLower = completion.name.toLowerCase();
    const queryLower = pathQuery.toLowerCase();
    if (nameLower.startsWith(queryLower)) return completion.name.slice(pathQuery.length);
    return "";
  }, [showPathPopup, pathCompletions, pathQuery, pathSelectedIndex]);

  const isInputDisabled = isSessionDead;

  // ── stateRef: lets stable callbacks read the latest values ──
  const stateRef = useRef<StateRefValue>({
    input: "",
    inputMode: "terminal",
    showSlashPopup: false,
    filteredSlashCommands: [],
    slashSelectedIndex: 0,
    showFilePopup: false,
    files: [],
    fileSelectedIndex: 0,
    showPathPopup: false,
    pathCompletions: [],
    pathSelectedIndex: 0,
    showHistorySearch: false,
    historySearchQuery: "",
    historyMatches: [],
    historySelectedIndex: 0,
    originalInput: "",
    commands: [],
    showToolPopup: false,
    toolMatches: [],
    toolSelectedIndex: 0,
    activeTool: null,
  });

  const ref = stateRef.current;
  ref.input = input;
  ref.showSlashPopup = showSlashPopup;
  ref.filteredSlashCommands = filteredSlashCommands;
  ref.slashSelectedIndex = slashSelectedIndex;
  ref.showFilePopup = showFilePopup;
  ref.files = files;
  ref.fileSelectedIndex = fileSelectedIndex;
  ref.showPathPopup = showPathPopup;
  ref.pathCompletions = pathCompletions;
  ref.pathSelectedIndex = pathSelectedIndex;
  ref.showHistorySearch = showHistorySearch;
  ref.historySearchQuery = historySearchQuery;
  ref.historyMatches = historyMatches;
  ref.historySelectedIndex = historySelectedIndex;
  ref.originalInput = originalInput;
  ref.commands = commands;
  ref.showToolPopup = showToolPopup;
  ref.toolMatches = toolMatches;
  ref.toolSelectedIndex = toolSelectedIndex;
  ref.activeTool = activeTool;

  // ── textarea auto-resize ──
  const lastTextareaHeightRef = useRef<number>(0);
  const desiredHeightRef = useRef(desiredHeight);
  desiredHeightRef.current = desiredHeight;
  const adjustTextareaHeight = useCallback(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    requestAnimationFrame(() => {
      const desired = desiredHeightRef.current;

      if (desired > 0) {
        // User has dragged the input panel to a fixed minimum height.
        // Set the textarea to that height FIRST, then check whether the
        // content actually needs more. This avoids the previous
        //   height='auto' → measure scrollHeight → restore
        // cycle that briefly shrunk the textarea down to one line (~26px)
        // before snapping it back to `desired` (~80px). That brief shrink
        // propagated up the parent flex column and caused the browser to
        // clamp `scrollTop` on the timeline above us, producing the
        // visible "jump up one row" glitch every time the user typed into
        // an otherwise single-line input. (Bug: 主页 timeline 输入时跳一格)
        const desiredStr = `${desired}px`;
        if (textarea.style.height !== desiredStr) {
          textarea.style.height = desiredStr;
        }
        const scrollHeight = textarea.scrollHeight;
        if (scrollHeight > desired) {
          const newHeight = Math.min(scrollHeight, 800);
          const newStr = `${newHeight}px`;
          if (textarea.style.height !== newStr) {
            textarea.style.height = newStr;
          }
          lastTextareaHeightRef.current = newHeight;
        } else {
          lastTextareaHeightRef.current = desired;
        }
        return;
      }

      // Legacy auto-grow path (user has never dragged the resize handle).
      // We must use height='auto' here so the textarea can SHRINK back when
      // content is deleted — without it the textarea would only ever grow.
      // The jump-glitch is much less noticeable on this path because the
      // delta is small (single-line ≈26px ↔ cap 200px) and there is no
      // fixed "minimum" the textarea snaps back to.
      textarea.style.height = "auto";
      const scrollHeight = textarea.scrollHeight;
      const newHeight = Math.min(scrollHeight, 200);
      const newStr = `${newHeight}px`;
      if (textarea.style.height !== newStr) {
        textarea.style.height = newStr;
      }
      lastTextareaHeightRef.current = newHeight;
    });
  }, []);

  useEffect(() => {
    void inputMode;
    if (isProcessRunning) return;
    // Chat-first: a terminal opened from the AI chat panel's "+" must not pull
    // the cursor out of the chat input during its startup window.
    if (isTerminalAutoFocusSuppressed(sessionId)) return;
    const handle = requestAnimationFrame(() => textareaRef.current?.focus());
    return () => cancelAnimationFrame(handle);
  }, [sessionId, inputMode, isProcessRunning]);

  useEffect(() => {
    if (isProcessRunning) {
      textareaRef.current?.blur();
    } else {
      // The shell prompt becomes ready a few seconds after the tab is created;
      // honour the chat-first suppression window so focus doesn't jump here then.
      if (isTerminalAutoFocusSuppressed(sessionId)) return;
      requestAnimationFrame(() => textareaRef.current?.focus());
    }
  }, [sessionId, isProcessRunning]);

  useEffect(() => {
    adjustTextareaHeight();
  }, [input, desiredHeight, adjustTextareaHeight]);

  // ── tool context refs ──
  const toolContextRef = useRef<{ cdPrefix: string; baseCmd: string }>({
    cdPrefix: "",
    baseCmd: "",
  });
  const toolHistoryRef = useRef<
    Map<string, { tool: ToolConfig; cdPrefix: string; baseCmd: string }>
  >(new Map());

  const clearToolMode = useCallback(() => {
    setActiveTool(null);
    setToolParams([]);
    toolContextRef.current = { cdPrefix: "", baseCmd: "" };
  }, []);

  // ── handlers ──

  const handleSubmit = useCallback(() => {
    const { input: currentInput } = stateRef.current;

    // Interactive-mode handler: pipe whatever the user typed (even
    // empty — hitting Enter on `[Y/n]` with no input is a valid
    // "accept default") straight to the running command's stdin via
    // `ptyWrite`. No history entry, no command tracking — those
    // concerns belong to the start-a-new-command path only.
    // `RunningCommandCard` is the primary stdin sink, this is a
    // fallback for keystrokes that reach `UnifiedInput` first.
    if (isInteractive) {
      setInput("");
      ptyWrite(sessionId, `${currentInput}\n`).catch((err) =>
        console.error("[UnifiedInput] interactive ptyWrite failed:", err)
      );
      return;
    }

    // Non-interactive: while a command is running suppress submit so
    // the user's freshly-typed text doesn't accidentally pipe into the
    // PTY as half-stdin / half-shell-command. The buffered text stays
    // in the textarea so the user can submit it as the next command
    // the moment the current one ends.
    const storeState = useStore.getState();
    if (storeState.pendingCommand[sessionId]?.command) return;

    if (!currentInput.trim()) return;

    const value = currentInput.trim();
    setInput("");
    resetHistory();

    if (/^\/t(\s|$)/i.test(value)) return;

    if (value === "clear") {
      clearTerminal(sessionId);
      setLastSentCommand(sessionId, "clear");
      ptyWrite(sessionId, "clear\n");
      return;
    }

    const ctx = toolContextRef.current;
    const currentActiveTool = stateRef.current.activeTool;
    let fullCmd = value;
    let historyEntry = value;
    if (ctx.baseCmd && currentActiveTool) {
      const args = value.trim();
      const toolCmd = args ? `${ctx.baseCmd} ${args}` : ctx.baseCmd;
      fullCmd = ctx.cdPrefix ? `${ctx.cdPrefix}${toolCmd}` : toolCmd;
      historyEntry = toolCmd;
      toolHistoryRef.current.set(historyEntry, {
        tool: currentActiveTool,
        cdPrefix: ctx.cdPrefix,
        baseCmd: ctx.baseCmd,
      });
      clearToolMode();
    }

    addToHistory(historyEntry);
    setLastSentCommand(sessionId, historyEntry);
    ptyWrite(sessionId, `${fullCmd}\n`).catch((err) =>
      console.error("[UnifiedInput] ptyWrite failed:", err)
    );
  }, [sessionId, addToHistory, resetHistory, setLastSentCommand, clearToolMode, isInteractive]);

  const handleSlashSelect = useCallback(async (command: SlashCommand, _args?: string) => {
    if (command.type === "builtin" && command.name === "t") {
      setShowSlashPopup(false);
      setInput("/t ");
      requestAnimationFrame(() => textareaRef.current?.focus());
      return;
    }
    setShowSlashPopup(false);
    setInput("");
    notify.info("Slash commands with AI are available in the AI Chat panel (right sidebar).");
  }, []);

  const handleFileSelect = useCallback(
    (file: FileInfo) => {
      setShowFilePopup(false);
      const newInput = input.replace(/@[^\s@]*$/, file.relative_path);
      setInput(newInput);
      setFileSelectedIndex(0);
    },
    [input]
  );

  const handlePathSelect = useCallback((completion: PathCompletion) => {
    const currentInput = stateRef.current.input;
    const cursorPos = textareaRef.current?.selectionStart ?? currentInput.length;
    const { startIndex } = extractWordAtCursor(currentInput, cursorPos);
    const newInput =
      currentInput.slice(0, startIndex) + completion.insert_text + currentInput.slice(cursorPos);
    setInput(newInput);
    setPathSelectedIndex(0);
    if (completion.entry_type === "directory") {
      setPathQuery(completion.insert_text);
    } else {
      setShowPathPopup(false);
    }
  }, []);

  const handlePathSelectFinal = useCallback((completion: PathCompletion) => {
    const currentInput = stateRef.current.input;
    const cursorPos = textareaRef.current?.selectionStart ?? currentInput.length;
    const { startIndex } = extractWordAtCursor(currentInput, cursorPos);
    const newInput =
      currentInput.slice(0, startIndex) + completion.insert_text + currentInput.slice(cursorPos);
    setInput(newInput);
    setShowPathPopup(false);
    setPathSelectedIndex(0);
  }, []);

  const handleHistorySelect = useCallback((match: HistoryMatch) => {
    setInput(match.command);
    setShowHistorySearch(false);
    setHistorySearchQuery("");
    setHistorySelectedIndex(0);
    textareaRef.current?.focus();
  }, []);

  const handleToolSelect = useCallback(
    async (tool: ToolConfig) => {
      setShowToolPopup(false);
      setToolSelectedIndex(0);

      if (!tool.installed) {
        setInput("");
        notify.warning(t("install.notInstalledWarning", { name: tool.name }));
        return;
      }

      try {
        const { buildCommand } = await import("@/lib/pentest/api");
        const result = await buildCommand(tool, "");
        toolContextRef.current = { cdPrefix: "", baseCmd: result.command };
      } catch (e) {
        console.warn("[ToolMode] buildCommand failed, trying legacy getToolCommand:", e);
        try {
          const { getToolCommand } = await import("@/lib/pentest/api");
          const cmd = await getToolCommand(tool);
          toolContextRef.current = { cdPrefix: "", baseCmd: cmd };
        } catch {
          toolContextRef.current = { cdPrefix: "", baseCmd: tool.name };
        }
      }

      try {
        const { readToolConfig } = await import("@/lib/pentest/api");
        let rawJson = "";
        try {
          rawJson = await readToolConfig(tool.name.toLowerCase());
        } catch {
          rawJson = await readToolConfig(tool.id);
        }
        const parsed = JSON.parse(rawJson) as { tool?: { params?: unknown } };
        const toolDef = parsed?.tool;
        const rawParams = toolDef?.params;
        setToolParams(Array.isArray(rawParams) ? rawParams : []);
      } catch {
        setToolParams([]);
      }

      setActiveTool(tool);
      setInput("");
      requestAnimationFrame(() => textareaRef.current?.focus());
    },
    [t]
  );

  // onChange handler — reads dynamic values from stateRef for callback stability
  const handleInputChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const {
        input: currentInput,
        showPathPopup: curShowPathPopup,
        activeTool: curActiveTool,
        commands: curCommands,
      } = stateRef.current;
      const curIsToolSearchMode = /^\/t\s/i.test(currentInput);

      let value = e.target.value;
      // Strip ASCII control bytes that some IMEs / Ctrl key chords
      // inject into the textarea value (e.g. Ctrl-H produces U+0008
      // BACKSPACE on macOS / Linux). They render as `⬛` boxes in
      // monospace fonts and would poison any command we eventually
      // ship to the shell. Keep `\n` (Shift+Enter / multi-line
      // commands) and `\t` (tab) — both are legitimate.
      value = value.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, "");
      if (curIsToolSearchMode && !value.toLowerCase().startsWith("/t")) {
        value = `/t ${value}`;
      }

      setInput(value);
      resetHistory();

      if (curShowPathPopup) {
        const newCursorPos = e.target.selectionStart ?? value.length;
        const { word } = extractWordAtCursor(value, newCursorPos);
        if (word) {
          setPathQuery(word);
          setPathSelectedIndex(0);
        } else {
          setShowPathPopup(false);
        }
      }

      const isToolInput = /^\/t\s/i.test(value);
      if (isToolInput && !curActiveTool) {
        const query = value.replace(/^\/t\s+/i, "");
        if (query.length > 0) {
          setShowToolPopup(true);
          setToolSelectedIndex(0);
        } else {
          setShowToolPopup(false);
        }
        setShowSlashPopup(false);
        setShowFilePopup(false);
      } else {
        setShowToolPopup(false);

        if (value.startsWith("/") && value.length >= 1) {
          const afterSlash = value.slice(1);
          const spaceIdx = afterSlash.indexOf(" ");
          const commandPart = spaceIdx === -1 ? afterSlash : afterSlash.slice(0, spaceIdx);
          const exactMatch = curCommands.some((c) => c.name === commandPart);
          if (spaceIdx === -1 || !exactMatch) {
            setShowSlashPopup(true);
            setSlashSelectedIndex(0);
          } else {
            setShowSlashPopup(false);
          }
          setShowFilePopup(false);
        } else {
          setShowSlashPopup(false);
        }

        if (/@[^\s@]*$/.test(value)) {
          setShowFilePopup(false);
        } else {
          setShowFilePopup(false);
        }
      }
    },
    [resetHistory]
  );

  const handleFocus = useCallback(() => {
    setIsFocused(true);
    // The user explicitly focused the terminal input — drop any chat-first
    // suppression so subsequent auto-focus (e.g. after a command finishes)
    // behaves normally.
    clearTerminalAutoFocusSuppression(sessionId);
    imeGetSource()
      .then((src) => {
        if (src && src !== "com.apple.keylayout.ABC") {
          prevImeSourceRef.current = src;
          imeSetSource("com.apple.keylayout.ABC");
        }
      })
      .catch(() => {});
  }, [sessionId]);

  const handleBlur = useCallback(() => {
    setIsFocused(false);
    if (prevImeSourceRef.current) {
      imeSetSource(prevImeSourceRef.current).catch(() => {});
      prevImeSourceRef.current = null;
    }
  }, []);

  return {
    t,
    sessionId,
    input,
    setInput,
    showSlashPopup,
    setShowSlashPopup,
    slashSelectedIndex,
    setSlashSelectedIndex,
    showFilePopup,
    setShowFilePopup,
    fileSelectedIndex,
    setFileSelectedIndex,
    showPathPopup,
    setShowPathPopup,
    pathSelectedIndex,
    setPathSelectedIndex,
    pathQuery,
    setPathQuery,
    showHistorySearch,
    setShowHistorySearch,
    historySearchQuery,
    setHistorySearchQuery,
    historySelectedIndex,
    setHistorySelectedIndex,
    originalInput,
    setOriginalInput,
    showToolPopup,
    setShowToolPopup,
    toolSelectedIndex,
    setToolSelectedIndex,
    activeTool,
    setActiveTool,
    toolParams,
    isFocused,
    caretSettings,
    textareaRef,
    dropZoneRef,
    inputContainerRef,
    stateRef,
    toolHistoryRef,
    toolContextRef,
    isSessionDead,
    inputMode,
    isProcessRunning,
    interactiveMode,
    isInteractive,
    isBlockCaret,
    isInputDisabled,
    isToolSearchMode,
    toolSearchQuery,
    ghostText,
    filteredSlashCommands,
    files,
    historyMatches,
    toolMatches,
    pathCompletions,
    pathTotalCount,
    commands,
    handleSubmit,
    handleSlashSelect,
    handleFileSelect,
    handlePathSelect,
    handlePathSelectFinal,
    handleHistorySelect,
    handleToolSelect,
    clearToolMode,
    handleInputChange,
    handleFocus,
    handleBlur,
    navigateUp,
    navigateDown,
    resetHistory,
  };
}

export type InputStateReturn = ReturnType<typeof useInputState>;
