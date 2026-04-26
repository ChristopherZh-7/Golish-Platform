import { useCallback } from "react";
import { ptyWrite } from "@/lib/tauri";
import { useStore } from "@/store";
import {
  type InputStateReturn,
  extractWordAtCursor,
  isCursorOnFirstLine,
  isCursorOnLastLine,
  clearTerminal,
} from "./useUnifiedInputState";

export function useInputKeyboard(state: InputStateReturn) {
  const {
    sessionId,
    stateRef,
    textareaRef,
    toolHistoryRef,
    toolContextRef,
    setInput,
    setShowSlashPopup,
    setSlashSelectedIndex,
    setShowFilePopup,
    setFileSelectedIndex,
    setShowPathPopup,
    setPathSelectedIndex,
    setPathQuery,
    setShowHistorySearch,
    setHistorySearchQuery,
    setHistorySelectedIndex,
    setOriginalInput,
    setShowToolPopup,
    setToolSelectedIndex,
    setActiveTool,
    handleSubmit,
    handleSlashSelect,
    handleFileSelect,
    handlePathSelect,
    handlePathSelectFinal,
    handleHistorySelect,
    handleToolSelect,
    clearToolMode,
    navigateUp,
    navigateDown,
  } = state;

  return useCallback(
    async (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      const {
        input,
        showSlashPopup,
        filteredSlashCommands,
        slashSelectedIndex,
        showFilePopup,
        files,
        fileSelectedIndex,
        showPathPopup,
        pathCompletions,
        pathSelectedIndex,
        showHistorySearch,
        historySearchQuery,
        historyMatches,
        historySelectedIndex,
        originalInput,
        commands,
        showToolPopup,
        toolMatches,
        toolSelectedIndex,
      } = stateRef.current;

      const isProcessRunning = !!useStore.getState().pendingCommand[sessionId];
      const isToolSearchMode = /^\/t\s/i.test(input);
      const toolSearchQuery = isToolSearchMode ? input.replace(/^\/t\s+/i, "") : "";

      // Force-clear stale pendingCommand state on Escape
      if (
        e.key === "Escape" &&
        isProcessRunning &&
        !stateRef.current.activeTool &&
        !showToolPopup &&
        !showHistorySearch
      ) {
        e.preventDefault();
        useStore.getState().handlePromptStart(sessionId);
        return;
      }

      // Exit /t tool search mode on Escape or Backspace on empty query
      if (isToolSearchMode && !stateRef.current.activeTool) {
        if (e.key === "Escape") {
          e.preventDefault();
          setInput("");
          setShowToolPopup(false);
          return;
        }
        if (e.key === "Backspace" && toolSearchQuery === "") {
          e.preventDefault();
          setInput("");
          setShowToolPopup(false);
          return;
        }
      }

      // Exit tool mode on Escape, or Backspace on empty input
      if (stateRef.current.activeTool) {
        if (e.key === "Escape") {
          e.preventDefault();
          clearToolMode();
          setInput("");
          if (isProcessRunning) {
            useStore.getState().handlePromptStart(sessionId);
          }
          return;
        }
        if (e.key === "Backspace" && input === "") {
          e.preventDefault();
          clearToolMode();
          return;
        }
      }

      // Tool search popup keyboard navigation
      if (showToolPopup && toolMatches.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowToolPopup(false);
          return;
        }
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setToolSelectedIndex((prev) => (prev + 1) % toolMatches.length);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setToolSelectedIndex((prev) => (prev - 1 + toolMatches.length) % toolMatches.length);
          return;
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          const sel = toolMatches[toolSelectedIndex];
          if (sel?.installed && sel?.envReady !== false) handleToolSelect(sel);
          return;
        }
        if (e.key === "Tab") {
          e.preventDefault();
          const sel = toolMatches[toolSelectedIndex];
          if (sel?.installed && sel?.envReady !== false) handleToolSelect(sel);
          return;
        }
      }

      // History search mode keyboard navigation
      if (showHistorySearch) {
        if (e.key === "Escape" || (e.ctrlKey && e.key === "g")) {
          e.preventDefault();
          setShowHistorySearch(false);
          setInput(originalInput);
          setHistorySearchQuery("");
          setHistorySelectedIndex(0);
          return;
        }

        if (e.key === "Enter" && !e.shiftKey && historyMatches.length > 0) {
          e.preventDefault();
          handleHistorySelect(historyMatches[historySelectedIndex]);
          return;
        }

        if (e.ctrlKey && e.key === "r") {
          e.preventDefault();
          if (historyMatches.length > 0) {
            setHistorySelectedIndex((prev) =>
              prev < historyMatches.length - 1 ? prev + 1 : 0,
            );
          }
          return;
        }

        if (e.key === "ArrowDown") {
          e.preventDefault();
          if (historyMatches.length > 0) {
            setHistorySelectedIndex((prev) =>
              prev < historyMatches.length - 1 ? prev + 1 : prev,
            );
          }
          return;
        }

        if (e.key === "ArrowUp") {
          e.preventDefault();
          if (historyMatches.length > 0) {
            setHistorySelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          }
          return;
        }

        if (e.key === "Backspace") {
          e.preventDefault();
          if (historySearchQuery.length > 0) {
            setHistorySearchQuery((prev) => prev.slice(0, -1));
            setHistorySelectedIndex(0);
          } else {
            setShowHistorySearch(false);
            setInput(originalInput);
            setHistorySearchQuery("");
            setHistorySelectedIndex(0);
          }
          return;
        }

        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          e.preventDefault();
          setHistorySearchQuery((prev) => prev + e.key);
          setHistorySelectedIndex(0);
          return;
        }

        return;
      }

      // Ctrl+R to open history search
      if (e.ctrlKey && e.key === "r" && !showHistorySearch) {
        e.preventDefault();
        setOriginalInput(input);
        setShowHistorySearch(true);
        setHistorySearchQuery("");
        setHistorySelectedIndex(0);
        return;
      }

      // Path completion keyboard navigation
      if (showPathPopup && pathCompletions.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowPathPopup(false);
          return;
        }
        if (
          e.key === "ArrowDown" ||
          (e.ctrlKey && (e.key === "n" || e.key === "j"))
        ) {
          e.preventDefault();
          e.stopPropagation();
          setPathSelectedIndex((prev) => (prev + 1) % pathCompletions.length);
          return;
        }
        if (
          e.key === "ArrowUp" ||
          (e.ctrlKey && (e.key === "p" || e.key === "k"))
        ) {
          e.preventDefault();
          e.stopPropagation();
          setPathSelectedIndex(
            (prev) => (prev - 1 + pathCompletions.length) % pathCompletions.length,
          );
          return;
        }
        if (e.key === "Tab" && !e.shiftKey) {
          e.preventDefault();
          handlePathSelect(pathCompletions[pathSelectedIndex]);
          return;
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          handlePathSelectFinal(pathCompletions[pathSelectedIndex]);
          return;
        }
      }

      // Slash popup keyboard navigation
      if (showSlashPopup && filteredSlashCommands.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowSlashPopup(false);
          return;
        }
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSlashSelectedIndex((prev) =>
            prev < filteredSlashCommands.length - 1 ? prev + 1 : prev,
          );
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSlashSelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          return;
        }
        if (e.key === "Tab") {
          e.preventDefault();
          const selectedPrompt = filteredSlashCommands[slashSelectedIndex];
          if (selectedPrompt) {
            setInput(`/${selectedPrompt.name} `);
            setShowSlashPopup(false);
          }
          return;
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          const selectedPrompt = filteredSlashCommands[slashSelectedIndex];
          if (selectedPrompt) handleSlashSelect(selectedPrompt);
          return;
        }
      }

      // File popup keyboard navigation
      if (showFilePopup && files.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowFilePopup(false);
          return;
        }
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setFileSelectedIndex((prev) => (prev < files.length - 1 ? prev + 1 : prev));
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setFileSelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          return;
        }
        if (e.key === "Tab") {
          e.preventDefault();
          const selectedFile = files[fileSelectedIndex];
          if (selectedFile) handleFileSelect(selectedFile);
          return;
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          const selectedFile = files[fileSelectedIndex];
          if (selectedFile) handleFileSelect(selectedFile);
          return;
        }
      }

      // Slash commands with args (popup closed)
      if (e.key === "Enter" && !e.shiftKey && input.startsWith("/")) {
        const afterSlash = input.slice(1);
        const spaceIdx = afterSlash.indexOf(" ");
        const cmdName = spaceIdx === -1 ? afterSlash : afterSlash.slice(0, spaceIdx);
        const args = spaceIdx === -1 ? "" : afterSlash.slice(spaceIdx + 1).trim();
        const matchingCommand = commands.find((c) => c.name === cmdName);
        if (matchingCommand) {
          e.preventDefault();
          handleSlashSelect(matchingCommand, args || undefined);
          return;
        }
      }

      // Enter → submit, Shift+Enter → newline
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
        return;
      }

      // History navigation (ArrowUp / ArrowDown)
      if (e.key === "ArrowUp") {
        const cursorPos = textareaRef.current?.selectionStart ?? 0;
        if (isCursorOnFirstLine(input, cursorPos)) {
          e.preventDefault();
          const cmd = navigateUp();
          if (cmd !== null) {
            const toolCtx = toolHistoryRef.current.get(cmd);
            if (toolCtx) {
              setActiveTool(toolCtx.tool);
              toolContextRef.current = {
                cdPrefix: toolCtx.cdPrefix,
                baseCmd: toolCtx.baseCmd,
              };
              const argsOnly = cmd.startsWith(toolCtx.baseCmd)
                ? cmd.slice(toolCtx.baseCmd.length).trimStart()
                : cmd;
              setInput(argsOnly);
            } else {
              clearToolMode();
              setInput(cmd);
            }
          }
        }
        return;
      }

      if (e.key === "ArrowDown") {
        const cursorPos = textareaRef.current?.selectionStart ?? input.length;
        if (isCursorOnLastLine(input, cursorPos)) {
          e.preventDefault();
          const recalled = navigateDown();
          const toolCtx = toolHistoryRef.current.get(recalled);
          if (toolCtx && recalled) {
            setActiveTool(toolCtx.tool);
            toolContextRef.current = {
              cdPrefix: toolCtx.cdPrefix,
              baseCmd: toolCtx.baseCmd,
            };
            const argsOnly = recalled.startsWith(toolCtx.baseCmd)
              ? recalled.slice(toolCtx.baseCmd.length).trimStart()
              : recalled;
            setInput(argsOnly);
          } else {
            clearToolMode();
            setInput(recalled);
          }
        }
        return;
      }

      // Terminal-specific shortcuts
      if (e.key === "Tab") {
        e.preventDefault();

        if (showPathPopup && pathCompletions.length > 0) {
          handlePathSelect(pathCompletions[pathSelectedIndex]);
          return;
        }

        const cursorPos = textareaRef.current?.selectionStart ?? input.length;
        const { word } = extractWordAtCursor(input, cursorPos);
        setPathQuery(word);
        setShowPathPopup(true);
        setPathSelectedIndex(0);
        return;
      }

      if (e.ctrlKey && e.key === "c") {
        e.preventDefault();
        await ptyWrite(sessionId, "\x03");
        setInput("");
        clearToolMode();
        setTimeout(() => {
          const store = useStore.getState();
          if (store.pendingCommand[sessionId]) {
            store.handlePromptStart(sessionId);
          }
        }, 500);
        return;
      }

      if (e.ctrlKey && e.key === "d") {
        e.preventDefault();
        await ptyWrite(sessionId, "\x04");
        return;
      }

      if (e.ctrlKey && e.key === "l") {
        e.preventDefault();
        clearTerminal(sessionId);
      }
    },
    [
      sessionId,
      stateRef,
      textareaRef,
      toolHistoryRef,
      toolContextRef,
      handleSubmit,
      navigateUp,
      navigateDown,
      handleSlashSelect,
      handleFileSelect,
      handlePathSelect,
      handlePathSelectFinal,
      handleHistorySelect,
      handleToolSelect,
      clearToolMode,
      setInput,
      setShowSlashPopup,
      setSlashSelectedIndex,
      setShowFilePopup,
      setFileSelectedIndex,
      setShowPathPopup,
      setPathSelectedIndex,
      setPathQuery,
      setShowHistorySearch,
      setHistorySearchQuery,
      setHistorySelectedIndex,
      setOriginalInput,
      setShowToolPopup,
      setToolSelectedIndex,
      setActiveTool,
    ],
  );
}
