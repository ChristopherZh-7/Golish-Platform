import { memo, useMemo } from "react";
import { cn } from "@/lib/utils";
import { BlockCaret } from "./BlockCaret";
import { ContextBar } from "./ContextBar";
import { InputPopups } from "./InputPopups";
import { InputBadges, SendButton, ToolParamsPanel } from "./InputToolbar";
import { useInputKeyboard } from "./useInputKeyboard";
import { useInputResize } from "./useInputResize";
import { useInputState } from "./useUnifiedInputState";

interface UnifiedInputProps {
  sessionId: string;
}

const ghostTextBaseStyle = { top: 0 } as const;

const GhostTextHint = memo(function GhostTextHint({
  text,
  inputLength,
}: {
  text: string;
  inputLength: number;
}) {
  const style = useMemo(() => ({ ...ghostTextBaseStyle, left: `${inputLength}ch` }), [inputLength]);

  return (
    <span
      className="absolute pointer-events-none font-mono text-[13px] text-muted-foreground/50 leading-[26px] whitespace-pre"
      style={style}
      aria-hidden="true"
    >
      {text}
    </span>
  );
});

export function UnifiedInput({ sessionId }: UnifiedInputProps) {
  const { desiredHeight, handlePointerDown, resetToDefault } = useInputResize();
  const state = useInputState({ sessionId, desiredHeight });
  const handleKeyDown = useInputKeyboard(state);

  const {
    t,
    input,
    showSlashPopup,
    setShowSlashPopup,
    showFilePopup,
    setShowFilePopup,
    fileSelectedIndex,
    showPathPopup,
    setShowPathPopup,
    pathSelectedIndex,
    showHistorySearch,
    setShowHistorySearch,
    historySelectedIndex,
    historySearchQuery,
    showToolPopup,
    setShowToolPopup,
    toolSelectedIndex,
    activeTool,
    toolParams,
    isFocused,
    caretSettings,
    textareaRef,
    dropZoneRef,
    inputContainerRef,
    isSessionDead,
    inputMode,
    interactiveMode,
    isInteractive,
    isProcessRunning,
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
    slashSelectedIndex,
    handleSubmit,
    handleSlashSelect,
    handleFileSelect,
    handlePathSelect,
    handleHistorySelect,
    handleToolSelect,
    clearToolMode,
    handleInputChange,
    handleFocus,
    handleBlur,
    setInput,
  } = state;

  const interactivePlaceholder = useMemo(() => {
    if (!isInteractive || !interactiveMode) return null;
    const cmd = interactiveMode.command?.trim() ?? "";
    const target = cmd ? `\`${cmd}\`` : "the running command";
    switch (interactiveMode.detector) {
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
  }, [isInteractive, interactiveMode]);

  const interactiveBannerLabel = useMemo(() => {
    if (!isInteractive || !interactiveMode) return null;
    const cmd = interactiveMode.command?.trim();
    return cmd ? `正在与 ${cmd} 交互` : "正在向运行中的命令发送输入";
  }, [isInteractive, interactiveMode]);

  // When a command is running but no `stdin_wait` has fired yet (so we
  // aren't in interactive mode), still surface a banner that hints at
  // the Esc-to-cancel escape hatch. Without it, users whose detector
  // missed the prompt (e.g. bash `select` PS3 / PS2 continuation —
  // see `stdin_wait_detector.rs`) were stranded with a non-interactive
  // input box and no obvious way to recover. Hiding the banner once we
  // enter interactive mode avoids stacking two competing banners.
  const runningBannerLabel = useMemo(() => {
    if (isInteractive) return null;
    if (!isProcessRunning) return null;
    return "命令运行中 · 按 Esc 取消";
  }, [isInteractive, isProcessRunning]);

  return (
    <div
      className={cn(
        "border-t border-[var(--border-subtle)]",
        isInteractive && "ring-1 ring-amber-500/40"
      )}
      data-interactive={isInteractive ? "true" : "false"}
    >
      <div
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize command input"
        title="Drag to resize · double-click to reset"
        className="h-1 -mt-px cursor-ns-resize bg-transparent hover:bg-accent/40 active:bg-accent/60 transition-colors titlebar-no-drag"
        onPointerDown={handlePointerDown}
        onDoubleClick={resetToDefault}
      />

      {isInteractive && interactiveBannerLabel && (
        <div
          className="flex items-center justify-between gap-2 px-3 py-1.5 bg-amber-500/10 border-b border-amber-500/30 text-[12px] text-amber-200"
          role="status"
          aria-live="polite"
          data-testid="interactive-mode-banner"
        >
          <span className="flex items-center gap-1.5 min-w-0 truncate">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />
            {interactiveBannerLabel}
          </span>
          <span className="text-amber-300/70 shrink-0">Esc 退出交互</span>
        </div>
      )}

      {!isInteractive && runningBannerLabel && (
        <div
          className="flex items-center justify-between gap-2 px-3 py-1.5 bg-sky-500/10 border-b border-sky-500/30 text-[12px] text-sky-200"
          role="status"
          aria-live="polite"
          data-testid="running-mode-banner"
        >
          <span className="flex items-center gap-1.5 min-w-0 truncate">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-sky-400 animate-pulse" />
            {runningBannerLabel}
          </span>
          <span className="text-sky-300/70 shrink-0">点击此处 → Esc 强制结束</span>
        </div>
      )}

      <ContextBar sessionId={sessionId} />

      <ToolParamsPanel
        activeTool={activeTool}
        toolParams={toolParams}
        input={input}
        setInput={setInput}
        textareaRef={textareaRef}
      />

      {/* Input row */}
      <div className="px-3 py-2 border-y border-[var(--border-subtle)]">
        <div
          ref={dropZoneRef}
          className="relative flex items-center gap-2 rounded-md bg-background px-2 py-1.5"
        >
          <InputBadges
            isToolSearchMode={isToolSearchMode}
            activeTool={activeTool}
            setInput={setInput}
            clearToolMode={clearToolMode}
            toolSearchTitle={t("toolSearch.title")}
          />

          <div ref={inputContainerRef} className="relative flex-1 min-w-0">
            <textarea
              ref={textareaRef}
              data-testid="unified-input"
              data-mode={inputMode}
              value={showHistorySearch ? "" : isToolSearchMode ? toolSearchQuery : input}
              onChange={handleInputChange}
              onKeyDown={handleKeyDown}
              disabled={isSessionDead}
              placeholder={
                showHistorySearch
                  ? ""
                  : isSessionDead
                    ? "Session limit exceeded. Please start a new session."
                    : isInteractive && interactivePlaceholder
                      ? interactivePlaceholder
                      : activeTool
                        ? (() => {
                            const req = toolParams.find((p) => p.required);
                            return req
                              ? `${req.flag} ${req.placeholder || req.label}...`
                              : t("input.enterParamsHint");
                          })()
                        : isToolSearchMode
                          ? t("input.searchToolName")
                          : t("input.enterCommand")
              }
              rows={1}
              className={cn(
                "w-full py-0 min-h-[26px] max-h-[800px]",
                "bg-transparent border-none shadow-none resize-none",
                "font-mono text-[13px] text-foreground leading-[26px] align-middle",
                "focus:outline-none focus:ring-0",
                "disabled:opacity-50 disabled:cursor-not-allowed",
                "placeholder:text-muted-foreground"
              )}
              style={isBlockCaret ? { caretColor: "transparent" } : undefined}
              onFocus={handleFocus}
              onBlur={handleBlur}
              spellCheck={false}
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
            />

            <BlockCaret
              textareaRef={textareaRef}
              text={input}
              settings={caretSettings}
              visible={isFocused && isBlockCaret && !isSessionDead}
            />

            {ghostText && inputMode === "terminal" && !showHistorySearch && (
              <GhostTextHint text={ghostText} inputLength={input.length} />
            )}

            <InputPopups
              containerRef={inputContainerRef}
              showHistorySearch={showHistorySearch}
              setShowHistorySearch={setShowHistorySearch}
              historyMatches={historyMatches}
              historySelectedIndex={historySelectedIndex}
              historySearchQuery={historySearchQuery}
              onHistorySelect={handleHistorySelect}
              showPathPopup={showPathPopup}
              setShowPathPopup={setShowPathPopup}
              pathCompletions={pathCompletions}
              pathTotalCount={pathTotalCount}
              pathSelectedIndex={pathSelectedIndex}
              onPathSelect={handlePathSelect}
              showSlashPopup={showSlashPopup}
              setShowSlashPopup={setShowSlashPopup}
              filteredSlashCommands={filteredSlashCommands}
              slashSelectedIndex={slashSelectedIndex}
              onSlashSelect={handleSlashSelect}
              showFilePopup={showFilePopup}
              setShowFilePopup={setShowFilePopup}
              files={files}
              fileSelectedIndex={fileSelectedIndex}
              onFileSelect={handleFileSelect}
              showToolPopup={showToolPopup}
              setShowToolPopup={setShowToolPopup}
              toolMatches={toolMatches}
              toolSelectedIndex={toolSelectedIndex}
              onToolSelect={handleToolSelect}
            />
          </div>

          <SendButton
            onSubmit={handleSubmit}
            disabled={(!isInteractive && !input.trim()) || isInputDisabled}
          />
        </div>
      </div>
    </div>
  );
}
