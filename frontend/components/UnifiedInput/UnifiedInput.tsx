import { memo, useMemo } from "react";
import { cn } from "@/lib/utils";
import { BlockCaret } from "./BlockCaret";
import { ContextBar } from "./ContextBar";
import { InputBadges, SendButton, ToolParamsPanel } from "./InputToolbar";
import { InputPopups } from "./InputPopups";
import { useInputKeyboard } from "./useInputKeyboard";
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
  const style = useMemo(
    () => ({ ...ghostTextBaseStyle, left: `${inputLength}ch` }),
    [inputLength],
  );

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
  const state = useInputState({ sessionId });
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

  return (
    <>
      <div className="border-t border-[var(--border-subtle)]">
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
                value={
                  showHistorySearch ? "" : isToolSearchMode ? toolSearchQuery : input
                }
                onChange={handleInputChange}
                onKeyDown={handleKeyDown}
                disabled={isSessionDead}
                placeholder={
                  showHistorySearch
                    ? ""
                    : isSessionDead
                      ? "Session limit exceeded. Please start a new session."
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
                  "w-full max-h-[200px] py-0 min-h-[26px]",
                  "bg-transparent border-none shadow-none resize-none",
                  "font-mono text-[13px] text-foreground leading-[26px] align-middle",
                  "focus:outline-none focus:ring-0",
                  "disabled:opacity-50 disabled:cursor-not-allowed",
                  "placeholder:text-muted-foreground",
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
              disabled={!input.trim() || isInputDisabled}
            />
          </div>
        </div>
      </div>
    </>
  );
}
