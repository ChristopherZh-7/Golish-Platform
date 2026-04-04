import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { SendHorizontal } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FileCommandPopup } from "@/components/FileCommandPopup";
import { HistorySearchPopup } from "@/components/HistorySearchPopup";
import { InlineTaskPlan } from "@/components/InlineTaskPlan";
import { PathCompletionPopup } from "@/components/PathCompletionPopup";
import { filterCommands, SlashCommandPopup } from "@/components/SlashCommandPopup";
import { ToolSearchPopup } from "@/components/ToolSearchPopup/ToolSearchPopup";
import { useCommandHistory } from "@/hooks/useCommandHistory";
import { useFileCommands } from "@/hooks/useFileCommands";
import { type HistoryMatch, useHistorySearch } from "@/hooks/useHistorySearch";
import { usePathCompletion } from "@/hooks/usePathCompletion";
import { type SlashCommand, useSlashCommands } from "@/hooks/useSlashCommands";
import {
  getVisionCapabilities,
  type ImagePart,
  sendPromptSession,
  sendPromptWithAttachments,
  type VisionCapabilities,
} from "@/lib/ai";
import { logger } from "@/lib/logger";
import { notify } from "@/lib/notify";
import { useTranslation } from "react-i18next";
import { type CaretSettings, DEFAULT_CARET_SETTINGS, getSettings } from "@/lib/settings";
import {
  classifyInput,
  type FileInfo,
  type PathCompletion,
  ptyWrite,
  readFileAsBase64 as readFileAsBase64FromPath,
  readPrompt,
  readSkillBody,
} from "@/lib/tauri";
import { useToolSearch } from "@/hooks/useToolSearch";
import type { ToolConfig } from "@/lib/pentest/types";

interface ToolParam {
  label: string;
  flag: string;
  type: string;
  required?: boolean;
  placeholder?: string;
  default?: string | number | boolean;
  options?: { value: string; label: string }[];
  description?: string;
}
import { cn } from "@/lib/utils";
import { useAgentMessages, usePendingCommand, useSessionAiConfig, useStore } from "@/store";
import { useUnifiedInputState } from "@/store/selectors/unified-input";
import { selectDisplaySettings } from "@/store/slices";
import { BlockCaret } from "./BlockCaret";
import { ContextBar } from "./ContextBar";
import { ImageAttachment, readFileAsBase64 } from "./ImageAttachment";
import { InputStatusRow } from "./InputStatusRow";

const clearTerminal = (sessionId: string) => {
  const store = useStore.getState();
  store.clearBlocks(sessionId);
  store.clearTimeline(sessionId);
};

interface UnifiedInputProps {
  sessionId: string;
}

// Extract word at cursor for tab completion
function extractWordAtCursor(
  input: string,
  cursorPos: number
): { word: string; startIndex: number } {
  const beforeCursor = input.slice(0, cursorPos);
  const match = beforeCursor.match(/[^\s|;&]+$/);
  if (!match) return { word: "", startIndex: cursorPos };
  return {
    word: match[0],
    startIndex: cursorPos - match[0].length,
  };
}

// Check if cursor is on the first line of textarea content
function isCursorOnFirstLine(text: string, cursorPos: number): boolean {
  const textBeforeCursor = text.substring(0, cursorPos);
  return !textBeforeCursor.includes("\n");
}

// Check if cursor is on the last line of textarea content
function isCursorOnLastLine(text: string, cursorPos: number): boolean {
  const textAfterCursor = text.substring(cursorPos);
  return !textAfterCursor.includes("\n");
}

// Static style constant for ghost text hint (top position is always 0)
const ghostTextBaseStyle = { top: 0 } as const;

// Memoized component for ghost text hint to avoid inline style object recreation
const GhostTextHint = memo(function GhostTextHint({
  text,
  inputLength,
}: {
  text: string;
  inputLength: number;
}) {
  // Memoize style with dynamic left position
  const style = useMemo(
    () => ({
      ...ghostTextBaseStyle,
      // Position at end of current input text using ch unit for monospace character width
      left: `${inputLength}ch`,
    }),
    [inputLength]
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
  const { t } = useTranslation();
  const workingDirectory = useStore((state) => state.sessions[sessionId]?.workingDirectory);
  const display = useStore(selectDisplaySettings);
  const [input, setInput] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
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
  const [imageAttachments, setImageAttachments] = useState<ImagePart[]>([]);
  const [visionCapabilities, setVisionCapabilities] = useState<VisionCapabilities | null>(null);
  const [showToolPopup, setShowToolPopup] = useState(false);
  const [toolSelectedIndex, setToolSelectedIndex] = useState(0);
  const [activeTool, setActiveTool] = useState<ToolConfig | null>(null);
  const [toolParams, setToolParams] = useState<ToolParam[]>([]);
  const [isDragOver, setIsDragOver] = useState(false);
  const [dragError, setDragError] = useState<string | null>(null);
  const [isFocused, setIsFocused] = useState(false);
  const [caretSettings, setCaretSettings] = useState<CaretSettings>(DEFAULT_CARET_SETTINGS);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const dropZoneRef = useRef<HTMLDivElement>(null);
  const inputContainerRef = useRef<HTMLDivElement>(null);
  const paneContainerRef = useRef<HTMLElement | null>(null);
  // Cached bounding rect for drop zone hit testing to avoid getBoundingClientRect() on every drag-over
  const dropZoneRectRef = useRef<DOMRect | null>(null);

  // Combined selector for optimized state access (reduces ~12 subscriptions to 1)
  const { inputMode, isAgentResponding, isCompacting, isSessionDead, streamingBlocksLength } =
    useUnifiedInputState(sessionId);
  const pendingCommand = usePendingCommand(sessionId);
  const isProcessRunning = !!pendingCommand;
  const hideAiItems = display.hideAiSettingsInShellMode && inputMode === "terminal";

  const showStatusRow =
    display.showStatusBar ||
    display.showInputModeToggle ||
    (!hideAiItems &&
      (display.showStatusBadge ||
        display.showAgentModeSelector ||
        display.showContextUsage ||
        display.showMcpBadge));

  // AI config for tracking provider changes (used to refresh vision capabilities)
  const aiConfig = useSessionAiConfig(sessionId);

  // Load caret settings and listen for changes
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

  // Store actions (stable references, don't cause re-renders)
  const setInputMode = useStore((state) => state.setInputMode);
  const setLastSentCommand = useStore((state) => state.setLastSentCommand);
  const addAgentMessage = useStore((state) => state.addAgentMessage);
  const agentMessages = useAgentMessages(sessionId);

  // Command history for up/down navigation
  const {
    history,
    add: addToHistory,
    navigateUp,
    navigateDown,
    reset: resetHistory,
  } = useCommandHistory({
    entryType: inputMode === "terminal" ? "cmd" : "prompt", // auto mode uses prompt history
  });

  // History search (Ctrl+R)
  const { matches: historyMatches } = useHistorySearch({
    history,
    query: historySearchQuery,
  });

  // Slash commands (prompts and skills)
  const { commands } = useSlashCommands(workingDirectory);
  // Split slash input into command name (for filtering) - args are extracted in handleKeyDown
  const slashInput = input.startsWith("/") ? input.slice(1) : "";
  const slashSpaceIndex = slashInput.indexOf(" ");
  const slashCommandName =
    slashSpaceIndex === -1 ? slashInput : slashInput.slice(0, slashSpaceIndex);
  const filteredSlashCommands = useMemo(
    () => filterCommands(commands, slashCommandName),
    [commands, slashCommandName]
  );

  // File commands (@ trigger)
  // Detect @ at end of input (e.g., "Look at @But" -> query is "But")
  const atMatch = input.match(/@([^\s@]*)$/);
  const fileQuery = atMatch?.[1] ?? "";
  const { files } = useFileCommands(workingDirectory, fileQuery);

  // Tool search - active when typing in terminal mode without slash/at triggers
  const isToolSearchMode = inputMode === "terminal" && /^\/t\s/i.test(input);
  const toolSearchQuery = isToolSearchMode ? input.replace(/^\/t\s+/i, "") : "";
  const toolSearchEnabled = isToolSearchMode && toolSearchQuery.length > 0 && !activeTool;
  const { matches: toolMatches } = useToolSearch(toolSearchQuery, toolSearchEnabled);

  // Path completions (Tab in terminal mode)
  const { completions: pathCompletions, totalCount: pathTotalCount } = usePathCompletion({
    sessionId,
    partialPath: pathQuery,
    enabled: showPathPopup && inputMode === "terminal",
  });

  // Ghost text shows the remainder of the top completion as a hint
  const ghostText = useMemo(() => {
    if (!showPathPopup || pathCompletions.length === 0 || !pathQuery) {
      return "";
    }

    // Use the selected completion, or the top one if nothing selected
    const completion = pathCompletions[pathSelectedIndex] || pathCompletions[0];

    // Ghost shows the part that would be added after current input
    // Extract what the completion would add beyond the current path query
    const nameLower = completion.name.toLowerCase();
    const queryLower = pathQuery.toLowerCase();

    // If the name starts with the query (fuzzy match may not be exact prefix), show the rest
    if (nameLower.startsWith(queryLower)) {
      return completion.name.slice(pathQuery.length);
    }

    // For fuzzy matches, don't show ghost (would be confusing)
    return "";
  }, [showPathPopup, pathCompletions, pathQuery, pathSelectedIndex]);

  // Agent is busy when submitting, streaming content, actively responding, or compacting
  const isAgentBusy =
    inputMode !== "terminal" &&
    (isSubmitting || streamingBlocksLength > 0 || isAgentResponding || isCompacting);

  // Input is disabled when agent is busy OR session is dead
  const isInputDisabled = isAgentBusy || isSessionDead;

  // Ref to store current state values for stable callbacks
  // This pattern allows callbacks to always access current values without being recreated
  // Updated directly in render (not in useEffect) because:
  // 1. Ref assignments are synchronous and don't trigger re-renders
  // 2. The value is available immediately for callbacks called during render
  // 3. No wasted useEffect overhead
  //
  // OPTIMIZATION: We update individual properties rather than creating a new object
  // every render. This avoids allocating a new 20+ field object on each render.
  const stateRef = useRef({
    input: "",
    inputMode: "terminal" as "terminal" | "agent" | "auto",
    isAgentBusy: false,
    isSubmitting: false,
    imageAttachments: [] as ImagePart[],
    visionCapabilities: null as VisionCapabilities | null,
    showSlashPopup: false,
    filteredSlashCommands: [] as SlashCommand[],
    slashSelectedIndex: 0,
    showFilePopup: false,
    files: [] as FileInfo[],
    fileSelectedIndex: 0,
    showPathPopup: false,
    pathCompletions: [] as PathCompletion[],
    pathSelectedIndex: 0,
    showHistorySearch: false,
    historySearchQuery: "",
    historyMatches: [] as HistoryMatch[],
    historySelectedIndex: 0,
    originalInput: "",
    commands: [] as SlashCommand[],
    showToolPopup: false,
    toolMatches: [] as ToolConfig[],
    toolSelectedIndex: 0,
    activeTool: null as ToolConfig | null,
  });

  // Update ref properties directly in render (no object allocation)
  // This is more efficient than creating a new object with 20+ fields every render
  const ref = stateRef.current;
  ref.input = input;
  ref.inputMode = inputMode;
  ref.isAgentBusy = isAgentBusy;
  ref.isSubmitting = isSubmitting;
  ref.imageAttachments = imageAttachments;
  ref.visionCapabilities = visionCapabilities;
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

  // Supported image MIME types for drag-and-drop and paste
  const SUPPORTED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/jpg", "image/gif", "image/webp"];

  // Process image files into ImagePart format
  const processImageFiles = useCallback(
    async (files: FileList | File[]): Promise<ImagePart[]> => {
      const newAttachments: ImagePart[] = [];
      const fileArray = Array.from(files);

      for (const file of fileArray) {
        // Check if it's a supported image type
        if (!SUPPORTED_IMAGE_TYPES.includes(file.type)) {
          notify.warning(`Unsupported file type: ${file.type}`);
          continue;
        }

        // Check file size if we have vision capabilities
        if (visionCapabilities && file.size > visionCapabilities.max_image_size_bytes) {
          const maxMB = (visionCapabilities.max_image_size_bytes / 1024 / 1024).toFixed(0);
          const fileMB = (file.size / 1024 / 1024).toFixed(1);
          notify.warning(`Image too large: ${fileMB}MB (max ${maxMB}MB)`);
          continue;
        }

        try {
          const base64 = await readFileAsBase64(file);
          newAttachments.push({
            type: "image",
            data: base64,
            media_type: file.type,
            filename: file.name,
          });
        } catch {
          notify.error("Failed to read file");
        }
      }

      return newAttachments;
    },
    [visionCapabilities]
  );

  // Cache for last known textarea height to avoid unnecessary reflows
  const lastTextareaHeightRef = useRef<number>(0);

  // Auto-resize textarea using requestAnimationFrame to batch DOM reads/writes
  // and avoid layout thrashing (write -> read -> write pattern)
  const adjustTextareaHeight = useCallback(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;

    // Use rAF to batch the layout operation
    requestAnimationFrame(() => {
      // Read phase: reset to auto and measure
      textarea.style.height = "auto";
      const scrollHeight = textarea.scrollHeight;
      const newHeight = Math.min(scrollHeight, 200);

      // Only write if height actually changed
      if (newHeight !== lastTextareaHeightRef.current) {
        lastTextareaHeightRef.current = newHeight;
        textarea.style.height = `${newHeight}px`;
      } else {
        // Restore the height since we set it to "auto" for measurement
        textarea.style.height = `${newHeight}px`;
      }
    });
  }, []);

  // Reset isSubmitting when AI response completes
  const prevMessagesLengthRef = useRef(agentMessages.length);
  useEffect(() => {
    // If a new message was added and we were submitting, check if it's from assistant/system
    if (agentMessages.length > prevMessagesLengthRef.current && isSubmitting) {
      const lastMessage = agentMessages[agentMessages.length - 1];
      // Reset if assistant or system (error) responded
      if (lastMessage && (lastMessage.role === "assistant" || lastMessage.role === "system")) {
        setIsSubmitting(false);
      }
    }
    prevMessagesLengthRef.current = agentMessages.length;
  }, [agentMessages, isSubmitting]);

  // Reset submission state when switching sessions to prevent input lock across tabs
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally only reset on sessionId change
  useEffect(() => {
    setIsSubmitting(false);
    // Reset ref to 0 so the message length check works correctly for the new session
    prevMessagesLengthRef.current = 0;
    // Clear attachments when switching sessions
    setImageAttachments([]);
  }, [sessionId]);

  // Fetch vision capabilities when in agent mode or when provider changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: aiConfig.provider triggers refetch when user switches providers
  useEffect(() => {
    if (inputMode !== "terminal") {
      getVisionCapabilities(sessionId)
        .then(setVisionCapabilities)
        .catch((err) => {
          logger.debug("Failed to get vision capabilities:", err);
          setVisionCapabilities(null);
        });
    }
  }, [sessionId, inputMode, aiConfig?.provider]);

  // Auto-focus input when session or mode changes.
  // Defer to the next frame so it isn't immediately overridden by focus management
  // (e.g., Radix Tabs focusing the clicked tab trigger).
  useEffect(() => {
    void sessionId;
    void inputMode;
    if (isProcessRunning) return;
    const handle = requestAnimationFrame(() => {
      textareaRef.current?.focus();
    });

    return () => cancelAnimationFrame(handle);
  }, [sessionId, inputMode, isProcessRunning]);

  // Blur textarea when a command is running (input bar is hidden,
  // focus should go to the interactive LiveTerminalBlock).
  // Re-focus when the command finishes.
  useEffect(() => {
    if (isProcessRunning) {
      textareaRef.current?.blur();
    } else {
      requestAnimationFrame(() => textareaRef.current?.focus());
    }
  }, [isProcessRunning]);

  // Adjust height when input changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: input triggers re-measurement of textarea scrollHeight
  useEffect(() => {
    adjustTextareaHeight();
  }, [input, adjustTextareaHeight]);

  // Find and cache the parent pane container element for drag-drop zone detection
  // Use requestAnimationFrame to ensure DOM is ready after render
  useEffect(() => {
    const findPaneContainer = () => {
      const paneContainer = document.querySelector<HTMLElement>(
        `[data-pane-drop-zone="${sessionId}"]`
      );
      paneContainerRef.current = paneContainer;
    };

    // Try immediately, then defer to next frame as fallback
    findPaneContainer();
    const handle = requestAnimationFrame(findPaneContainer);

    return () => cancelAnimationFrame(handle);
  }, [sessionId]);

  // Toggle input mode (cycle: terminal → agent → auto → terminal)
  const toggleInputMode = useCallback(() => {
    const next = inputMode === "terminal" ? "agent" : inputMode === "agent" ? "auto" : "terminal";
    setInputMode(sessionId, next);
  }, [sessionId, inputMode, setInputMode]);

  // Update the cached drop zone bounding rect (called on drag-enter and periodically)
  const updateDropZoneRect = useCallback(() => {
    let dropZone = paneContainerRef.current;
    if (!dropZone) {
      dropZone = document.querySelector<HTMLElement>(`[data-pane-drop-zone="${sessionId}"]`);
      if (dropZone) {
        paneContainerRef.current = dropZone;
      }
    }
    if (!dropZone) {
      dropZone = dropZoneRef.current;
    }
    if (dropZone) {
      dropZoneRectRef.current = dropZone.getBoundingClientRect();
    }
  }, [sessionId]);

  // Check if a position is within the pane container (entire pane is a drop zone)
  // Uses cached bounding rect to avoid layout thrashing on every drag-over event
  const isPositionOverDropZone = useCallback((x: number, y: number): boolean => {
    const rect = dropZoneRectRef.current;
    if (!rect) return false;
    return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
  }, []);

  // Process file paths from Tauri drag-drop into ImagePart format
  const processFilePaths = useCallback(async (filePaths: string[]): Promise<ImagePart[]> => {
    const newAttachments: ImagePart[] = [];

    for (const filePath of filePaths) {
      // Check if it's an image by extension
      const ext = filePath.toLowerCase().split(".").pop();
      const imageExtensions = ["png", "jpg", "jpeg", "gif", "webp"];
      if (!ext || !imageExtensions.includes(ext)) {
        notify.warning(`Skipping non-image file: ${filePath}`);
        continue;
      }

      // Get MIME type from extension
      const mimeTypes: Record<string, string> = {
        png: "image/png",
        jpg: "image/jpeg",
        jpeg: "image/jpeg",
        gif: "image/gif",
        webp: "image/webp",
      };
      const mediaType = mimeTypes[ext] || "image/png";

      try {
        // Use Tauri command to read file as base64 data URL
        const base64 = await readFileAsBase64FromPath(filePath);

        const filename = filePath.split("/").pop() || filePath.split("\\").pop() || "image";
        newAttachments.push({
          type: "image",
          data: base64,
          media_type: mediaType,
          filename,
        });
      } catch {
        notify.error(`Failed to read image: ${filePath}`);
      }
    }

    return newAttachments;
  }, []);

  // Tauri drag-drop event listeners
  // Track last known drag position for drop zone detection
  const lastDragPositionRef = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    // Skip in browser mode (no Tauri)
    if (typeof window !== "undefined" && window.__MOCK_BROWSER_MODE__) {
      return;
    }

    const unlisteners: UnlistenFn[] = [];

    const setupListeners = async () => {
      // Listen for drag enter - cache the drop zone rect for hit testing
      const unlistenEnter = await listen("tauri://drag-enter", () => {
        // Cache the bounding rect once on drag-enter to avoid
        // calling getBoundingClientRect() on every drag-over event
        updateDropZoneRect();
      });
      unlisteners.push(unlistenEnter);

      // Listen for drag over - update visual state based on position
      const unlistenOver = await listen<{ position: { x: number; y: number } }>(
        "tauri://drag-over",
        (event) => {
          const { x, y } = event.payload.position;
          lastDragPositionRef.current = { x, y };

          if (inputMode !== "terminal" && isPositionOverDropZone(x, y)) {
            setIsDragOver(true);
            setDragError(null);
          } else {
            setIsDragOver(false);
          }
        }
      );
      unlisteners.push(unlistenOver);

      // Listen for drag leave
      const unlistenLeave = await listen("tauri://drag-leave", () => {
        setIsDragOver(false);
        setDragError(null);
        lastDragPositionRef.current = null;
        dropZoneRectRef.current = null; // Clear cached rect
      });
      unlisteners.push(unlistenLeave);

      // Listen for drop
      const unlistenDrop = await listen<{ paths: string[]; position: { x: number; y: number } }>(
        "tauri://drag-drop",
        async (event) => {
          setIsDragOver(false);
          setDragError(null);
          lastDragPositionRef.current = null;

          // Use the drop event's position to check if over drop zone
          const { x, y } = event.payload.position;
          const isOverDropZone = isPositionOverDropZone(x, y);

          // Only process if in agent/auto mode and over drop zone
          if (inputMode === "terminal" || !isOverDropZone) {
            return;
          }

          const filePaths = event.payload.paths;
          if (filePaths.length === 0) return;

          // Check if any paths are images
          const imageExtensions = ["png", "jpg", "jpeg", "gif", "webp"];
          const hasImages = filePaths.some((path) => {
            const ext = path.toLowerCase().split(".").pop();
            return ext && imageExtensions.includes(ext);
          });

          if (!hasImages) {
            notify.warning("Only image files are supported");
            return;
          }

          const newAttachments = await processFilePaths(filePaths);
          if (newAttachments.length > 0) {
            setImageAttachments((prev) => [...prev, ...newAttachments]);
          }
        }
      );
      unlisteners.push(unlistenDrop);
    };

    setupListeners();

    return () => {
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [inputMode, isPositionOverDropZone, processFilePaths, updateDropZoneRect]);

  // Clipboard paste handler for image attachment
  const handlePaste = useCallback(
    async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      // Only handle in agent/auto mode
      if (inputMode === "terminal") return;

      const clipboardItems = e.clipboardData.items;
      const imageItems: File[] = [];

      for (const item of clipboardItems) {
        if (item.kind === "file" && item.type.startsWith("image/")) {
          const file = item.getAsFile();
          if (file) {
            imageItems.push(file);
          }
        }
      }

      // If no images, let default paste behavior handle text
      if (imageItems.length === 0) return;

      // Prevent default only if we have images to process
      e.preventDefault();

      const newAttachments = await processImageFiles(imageItems);
      if (newAttachments.length > 0) {
        setImageAttachments((prev) => [...prev, ...newAttachments]);
      }
    },
    [inputMode, processImageFiles]
  );

  // Stable handleSubmit callback - reads current values from stateRef
  // This prevents recreation when streaming state (isAgentResponding, streamingBlocks) changes
  const handleSubmit = useCallback(async () => {
    // Read current values from ref
    const { input, inputMode, isAgentBusy, imageAttachments, visionCapabilities } =
      stateRef.current;

    // Allow submit if: (1) has text input, OR (2) agent/auto mode with image attachments
    const hasContent =
      input.trim() ||
      ((inputMode === "agent" || inputMode === "auto") && imageAttachments.length > 0);
    if (!hasContent || isAgentBusy) {
      return;
    }

    const value = input.trim();
    setInput("");
    resetHistory();

    // Determine effective mode: for "auto", classify the input
    let effectiveMode: "terminal" | "agent";
    if (inputMode === "auto") {
      try {
        const result = await classifyInput(value);
        effectiveMode = result.route;
      } catch {
        effectiveMode = "terminal"; // fallback
      }
    } else {
      effectiveMode = inputMode as "terminal" | "agent";
    }

    if (effectiveMode === "terminal") {
      // /t prefix is tool search — don't send to terminal
      if (/^\/t(\s|$)/i.test(value)) {
        return;
      }

      // Handle clear command - clear timeline and command blocks
      if (value === "clear") {
        clearTerminal(sessionId);
        // Don't send to PTY - just clear the UI
        return;
      }

      // If in tool mode, construct the full command with cd and runtime prefix
      const ctx = toolContextRef.current;
      const currentActiveTool = stateRef.current.activeTool;
      let fullCmd = value;
      let historyEntry = value;
      if (ctx.baseCmd && currentActiveTool) {
        const args = value.trim();
        const toolCmd = args ? `${ctx.baseCmd} ${args}` : ctx.baseCmd;
        fullCmd = ctx.cdPrefix ? `${ctx.cdPrefix}${toolCmd}` : toolCmd;
        historyEntry = toolCmd;
        // Save to tool history for badge restoration on recall
        toolHistoryRef.current.set(historyEntry, {
          tool: currentActiveTool,
          cdPrefix: ctx.cdPrefix,
          baseCmd: ctx.baseCmd,
        });
        clearToolMode();
      }

      // Add the user-facing command to history (not just raw args)
      addToHistory(historyEntry);

      // Store command before sending
      setLastSentCommand(sessionId, historyEntry);

      // Send command + newline to PTY
      await ptyWrite(sessionId, `${fullCmd}\n`);
    } else {
      // Agent mode: send to AI

      // Validate images if attached but provider doesn't support vision
      if (imageAttachments.length > 0 && !visionCapabilities?.supports_vision) {
        notify.error(
          "Current model doesn't support images. Remove images or switch to a vision-capable model (Claude 3+, GPT-4+, Gemini)."
        );
        return;
      }

      setIsSubmitting(true);

      // Add to history
      addToHistory(value);

      // Capture attachments before clearing (we need them for the message and backend)
      const attachmentsToSend = imageAttachments.length > 0 ? [...imageAttachments] : [];

      // Add user message to store (with attachments if any)
      addAgentMessage(sessionId, {
        id: crypto.randomUUID(),
        sessionId,
        role: "user",
        content: value,
        timestamp: new Date().toISOString(),
        attachments: attachmentsToSend.length > 0 ? attachmentsToSend : undefined,
      });

      // Clear attachments immediately after adding to message (triggers pop animation)
      if (attachmentsToSend.length > 0) {
        setImageAttachments([]);
      }

      // Send to AI backend - response will come via useAiEvents hook
      try {
        if (attachmentsToSend.length > 0) {
          // Build payload with text and images
          const payload = {
            parts: [
              ...(value ? [{ type: "text" as const, text: value }] : []),
              ...attachmentsToSend,
            ],
          };
          await sendPromptWithAttachments(sessionId, payload);
        } else {
          await sendPromptSession(sessionId, value);
        }
        // Response will be handled by useAiEvents when AI completes
        // Don't set isSubmitting to false here - wait for completed/error event
      } catch (error) {
        notify.error(`Agent error: ${error}`);
        setIsSubmitting(false);
      }
    }
  }, [sessionId, addAgentMessage, addToHistory, resetHistory, setLastSentCommand]);

  // Handle slash command selection (prompts and skills)
  const handleSlashSelect = useCallback(
    async (command: SlashCommand, args?: string) => {
      // Built-in /t command: enter tool search mode
      if (command.type === "builtin" && command.name === "t") {
        setShowSlashPopup(false);
        setInput("/t ");
        requestAnimationFrame(() => textareaRef.current?.focus());
        return;
      }

      setShowSlashPopup(false);
      setInput("");

      // Switch to agent mode if in terminal mode
      if (inputMode === "terminal") {
        setInputMode(sessionId, "agent");
      }

      // Read and send the command content
      try {
        // Skills use readSkillBody (just the instructions), prompts use readPrompt (full file)
        const content =
          command.type === "skill"
            ? await readSkillBody(command.path)
            : await readPrompt(command.path);
        // Append args to content if provided
        const fullContent = args ? `${content}\n\n${args}` : content;
        setIsSubmitting(true);

        // Add user message to store (show the slash command name with args)
        addAgentMessage(sessionId, {
          id: crypto.randomUUID(),
          sessionId,
          role: "user",
          content: args ? `/${command.name} ${args}` : `/${command.name}`,
          timestamp: new Date().toISOString(),
        });

        // Send the actual content (with args) to AI
        await sendPromptSession(sessionId, fullContent);
      } catch (error) {
        notify.error(`Failed to run ${command.type}: ${error}`);
        setIsSubmitting(false);
      }
    },
    [sessionId, inputMode, setInputMode, addAgentMessage]
  );

  // Handle file selection from @ popup
  const handleFileSelect = useCallback(
    (file: FileInfo) => {
      setShowFilePopup(false);
      // Replace @query with the selected file's relative path
      const newInput = input.replace(/@[^\s@]*$/, file.relative_path);
      setInput(newInput);
      setFileSelectedIndex(0);
    },
    [input]
  );

  // Handle path completion selection (Tab in terminal mode)
  // For directories: complete and show contents (keep popup open)
  // For files: complete and close popup
  // NOTE: Reads from stateRef.current.input to avoid stale closure issues when called from handleKeyDown
  const handlePathSelect = useCallback(
    (completion: PathCompletion) => {
      const currentInput = stateRef.current.input;
      const cursorPos = textareaRef.current?.selectionStart ?? currentInput.length;
      const { startIndex } = extractWordAtCursor(currentInput, cursorPos);

      const newInput =
        currentInput.slice(0, startIndex) + completion.insert_text + currentInput.slice(cursorPos);

      setInput(newInput);
      setPathSelectedIndex(0);

      if (completion.entry_type === "directory") {
        // Keep popup open and update query to show directory contents
        setPathQuery(completion.insert_text);
        // Popup stays open to show directory contents
      } else {
        // Close popup for files
        setShowPathPopup(false);
      }
    },
    [] // No dependencies - reads from stateRef
  );

  // Note: Previously had auto-complete when there's only one unique match (bash/zsh behavior).
  // Removed to allow users to keep typing and filtering without the popup auto-closing.
  // Users can press Tab or Enter to explicitly select the completion.

  // Handle path completion final selection (Enter) - closes popup without continuing
  // NOTE: Reads from stateRef.current.input to avoid stale closure issues when called from handleKeyDown
  const handlePathSelectFinal = useCallback(
    (completion: PathCompletion) => {
      const currentInput = stateRef.current.input;
      const cursorPos = textareaRef.current?.selectionStart ?? currentInput.length;
      const { startIndex } = extractWordAtCursor(currentInput, cursorPos);

      const newInput =
        currentInput.slice(0, startIndex) + completion.insert_text + currentInput.slice(cursorPos);

      setInput(newInput);
      setShowPathPopup(false);
      setPathSelectedIndex(0);
      // Don't continue for directories - just close the popup
    },
    [] // No dependencies - reads from stateRef
  );

  // Handle history search selection
  const handleHistorySelect = useCallback((match: HistoryMatch) => {
    setInput(match.command);
    setShowHistorySearch(false);
    setHistorySearchQuery("");
    setHistorySelectedIndex(0);
    textareaRef.current?.focus();
  }, []);

  // Stores the tool context for transparent directory change on submit
  const toolContextRef = useRef<{ cdPrefix: string; baseCmd: string }>({ cdPrefix: "", baseCmd: "" });

  const clearToolMode = useCallback(() => {
    setActiveTool(null);
    setToolParams([]);
    toolContextRef.current = { cdPrefix: "", baseCmd: "" };
  }, []);
  // Maps history commands to their tool context for badge restoration on recall
  const toolHistoryRef = useRef<Map<string, { tool: ToolConfig; cdPrefix: string; baseCmd: string }>>(new Map());

  // Handle tool selection from tool search popup — enters "tool launch mode"
  const handleToolSelect = useCallback(
    async (tool: ToolConfig) => {
      setShowToolPopup(false);
      setToolSelectedIndex(0);

      if (!tool.installed) {
        setInput("");
        notify.warning(t("install.notInstalledWarning", { name: tool.name }));
        return;
      }

      // Build command context
      const { getConfig } = await import("@/lib/pentest/api");
      let toolsDir = "";
      try {
        const cfg = await getConfig();
        toolsDir = cfg.tools_dir;
      } catch { /* use empty */ }

      let prefix = "";
      console.log("[ToolMode] runtime:", tool.runtime, "runtimeVersion:", tool.runtimeVersion);
      if (tool.runtime === "python") {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          console.log("[ToolMode] Resolving python path for version:", tool.runtimeVersion);
          const pythonPath = await invoke<string>("pentest_resolve_python_path", {
            version: tool.runtimeVersion || "",
          });
          console.log("[ToolMode] Resolved python path:", pythonPath);
          prefix = `"${pythonPath}"`;
        } catch (e) {
          console.warn("[ToolMode] Failed to resolve python path:", e);
          prefix = "python3";
        }
      } else if (tool.runtime === "java") {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const javaPath = await invoke<string>("pentest_resolve_java_path", {
            version: tool.runtimeVersion || "",
          });
          console.log("[ToolMode] Resolved java path:", javaPath);
          prefix = `"${javaPath}" -jar`;
        } catch (e) {
          console.warn("[ToolMode] Failed to resolve java path, using default:", e);
          prefix = "java -jar";
        }
      } else if (tool.runtime === "node") {
        prefix = "node";
      }
      const isHomebrew = tool.install?.method === "homebrew" || tool.runtime === "native" || tool.runtime === "system";
      const exeFile = tool.executable.split("/").pop() || tool.executable;

      if (isHomebrew) {
        const cmdName = tool.install?.source || exeFile.replace(/\.[^.]+$/, "");
        toolContextRef.current = { cdPrefix: "", baseCmd: cmdName };
      } else {
        const runCmd = prefix ? `${prefix} ${exeFile}` : `./${exeFile}`;
        if (toolsDir) {
          const toolSubDir = tool.executable.includes("/")
            ? tool.executable.split("/").slice(0, -1).join("/")
            : tool.name.toLowerCase();
          toolContextRef.current = {
            cdPrefix: `cd "${toolsDir}/${toolSubDir}" && `,
            baseCmd: runCmd,
          };
        } else {
          toolContextRef.current = { cdPrefix: "", baseCmd: runCmd };
        }
      }

      // Load tool params from raw config — try name-based filename first, then id-based
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        let rawJson = "";
        try {
          rawJson = await invoke<string>("pentest_read_tool_config", {
            category: tool.category,
            subcategory: tool.subcategory,
            toolId: tool.name.toLowerCase(),
          });
        } catch {
          rawJson = await invoke<string>("pentest_read_tool_config", {
            category: tool.category,
            subcategory: tool.subcategory,
            toolId: tool.id,
          });
        }
        const parsed = JSON.parse(rawJson);
        setToolParams(parsed?.tool?.params || []);
      } catch {
        setToolParams([]);
      }

      // Enter tool mode — clear input for parameter entry
      setActiveTool(tool);
      setInput("");

      requestAnimationFrame(() => {
        textareaRef.current?.focus();
      });
    },
    []
  );

  // Stable handleKeyDown callback - reads current values from stateRef
  // This prevents recreation on every keystroke (when input changes)
  const handleKeyDown = useCallback(
    async (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // Read current values from ref
      const {
        input,
        inputMode,
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

      // Force-clear stale pendingCommand state on Escape
      if (e.key === "Escape" && isProcessRunning && !stateRef.current.activeTool && !showToolPopup && !showHistorySearch) {
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
        // Escape or Ctrl+G - cancel search and restore original input
        if (e.key === "Escape" || (e.ctrlKey && e.key === "g")) {
          e.preventDefault();
          setShowHistorySearch(false);
          setInput(originalInput);
          setHistorySearchQuery("");
          setHistorySelectedIndex(0);
          return;
        }

        // Enter - select current match and close
        if (e.key === "Enter" && !e.shiftKey && historyMatches.length > 0) {
          e.preventDefault();
          handleHistorySelect(historyMatches[historySelectedIndex]);
          return;
        }

        // Ctrl+R - cycle to next match
        if (e.ctrlKey && e.key === "r") {
          e.preventDefault();
          if (historyMatches.length > 0) {
            setHistorySelectedIndex((prev) => (prev < historyMatches.length - 1 ? prev + 1 : 0));
          }
          return;
        }

        // Arrow down - navigate to next match
        if (e.key === "ArrowDown") {
          e.preventDefault();
          if (historyMatches.length > 0) {
            setHistorySelectedIndex((prev) => (prev < historyMatches.length - 1 ? prev + 1 : prev));
          }
          return;
        }

        // Arrow up - navigate to previous match
        if (e.key === "ArrowUp") {
          e.preventDefault();
          if (historyMatches.length > 0) {
            setHistorySelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          }
          return;
        }

        // Backspace - remove character from search query or exit if empty
        if (e.key === "Backspace") {
          e.preventDefault();
          if (historySearchQuery.length > 0) {
            setHistorySearchQuery((prev) => prev.slice(0, -1));
            setHistorySelectedIndex(0);
          } else {
            // Exit search mode if query is empty
            setShowHistorySearch(false);
            setInput(originalInput);
            setHistorySearchQuery("");
            setHistorySelectedIndex(0);
          }
          return;
        }

        // Any printable character - add to search query
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          e.preventDefault();
          setHistorySearchQuery((prev) => prev + e.key);
          setHistorySelectedIndex(0);
          return;
        }

        // Block all other keys when in search mode
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

      // Cmd+I to toggle input mode - handle first to ensure it works in all modes
      // Check both lowercase 'i' and the key code for reliability across platforms
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && (e.key === "i" || e.key === "I")) {
        e.preventDefault();
        e.stopPropagation();
        toggleInputMode();
        return;
      }

      // Path completion keyboard navigation (terminal mode)
      if (showPathPopup && pathCompletions.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowPathPopup(false);
          return;
        }
        // Arrow keys and vim-style navigation: Ctrl+N/J = down, Ctrl+P/K = up
        // Navigation wraps around at boundaries
        if (e.key === "ArrowDown" || (e.ctrlKey && (e.key === "n" || e.key === "j"))) {
          e.preventDefault();
          e.stopPropagation();
          setPathSelectedIndex((prev) => (prev + 1) % pathCompletions.length);
          return;
        }
        if (e.key === "ArrowUp" || (e.ctrlKey && (e.key === "p" || e.key === "k"))) {
          e.preventDefault();
          e.stopPropagation();
          setPathSelectedIndex(
            (prev) => (prev - 1 + pathCompletions.length) % pathCompletions.length
          );
          return;
        }
        // Tab - select and continue into directories
        if (e.key === "Tab" && !e.shiftKey) {
          e.preventDefault();
          handlePathSelect(pathCompletions[pathSelectedIndex]);
          return;
        }
        // Enter - select and close popup (final selection)
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          handlePathSelectFinal(pathCompletions[pathSelectedIndex]);
          return;
        }
      }

      // When slash popup is open, handle navigation
      if (showSlashPopup && filteredSlashCommands.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowSlashPopup(false);
          return;
        }

        // Arrow down - move selection down
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSlashSelectedIndex((prev) =>
            prev < filteredSlashCommands.length - 1 ? prev + 1 : prev
          );
          return;
        }

        // Arrow up - move selection up
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSlashSelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          return;
        }

        // Tab - complete the selected option into the input field with space for args
        if (e.key === "Tab") {
          e.preventDefault();
          const selectedPrompt = filteredSlashCommands[slashSelectedIndex];
          if (selectedPrompt) {
            setInput(`/${selectedPrompt.name} `);
            setShowSlashPopup(false);
          }
          return;
        }

        // Enter - execute the selected option
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          const selectedPrompt = filteredSlashCommands[slashSelectedIndex];
          if (selectedPrompt) {
            handleSlashSelect(selectedPrompt);
          }
          return;
        }
      }

      // When file popup is open, handle navigation
      if (showFilePopup && files.length > 0) {
        if (e.key === "Escape") {
          e.preventDefault();
          setShowFilePopup(false);
          return;
        }

        // Arrow down - move selection down
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setFileSelectedIndex((prev) => (prev < files.length - 1 ? prev + 1 : prev));
          return;
        }

        // Arrow up - move selection up
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setFileSelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          return;
        }

        // Tab - complete the selected file
        if (e.key === "Tab") {
          e.preventDefault();
          const selectedFile = files[fileSelectedIndex];
          if (selectedFile) {
            handleFileSelect(selectedFile);
          }
          return;
        }

        // Enter - insert the selected file
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          const selectedFile = files[fileSelectedIndex];
          if (selectedFile) {
            handleFileSelect(selectedFile);
          }
          return;
        }
      }

      // Cmd+Shift+T to toggle input mode
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        toggleInputMode();
        return;
      }


      // Handle Enter for slash commands with args (popup closed due to exact match + space)
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

      // Handle Enter - execute/send (Shift+Enter for newline)
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        await handleSubmit();
        return;
      }

      // History navigation - shared between terminal and agent modes
      // Only activate history if cursor is on the first/last line of input
      if (e.key === "ArrowUp") {
        const cursorPos = textareaRef.current?.selectionStart ?? 0;
        if (isCursorOnFirstLine(input, cursorPos)) {
          e.preventDefault();
          const cmd = navigateUp();
          if (cmd !== null) {
            // Check if this command was a tool command — restore badge if so
            const toolCtx = toolHistoryRef.current.get(cmd);
            if (toolCtx) {
              setActiveTool(toolCtx.tool);
              toolContextRef.current = { cdPrefix: toolCtx.cdPrefix, baseCmd: toolCtx.baseCmd };
              // Show only the arguments portion (strip the base command prefix)
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
        // Otherwise, let default behavior move cursor up
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
            toolContextRef.current = { cdPrefix: toolCtx.cdPrefix, baseCmd: toolCtx.baseCmd };
            const argsOnly = recalled.startsWith(toolCtx.baseCmd)
              ? recalled.slice(toolCtx.baseCmd.length).trimStart()
              : recalled;
            setInput(argsOnly);
          } else {
            clearToolMode();
            setInput(recalled);
          }
        }
        // Otherwise, let default behavior move cursor down
        return;
      }

      // Terminal-specific shortcuts
      if (inputMode === "terminal") {
        // Handle Tab - show path completion popup
        if (e.key === "Tab") {
          e.preventDefault();

          // If popup already open, select current item
          if (showPathPopup && pathCompletions.length > 0) {
            handlePathSelect(pathCompletions[pathSelectedIndex]);
            return;
          }

          // Extract word at cursor and show popup
          const cursorPos = textareaRef.current?.selectionStart ?? input.length;
          const { word } = extractWordAtCursor(input, cursorPos);
          setPathQuery(word);
          setShowPathPopup(true);
          setPathSelectedIndex(0);
          return;
        }

        // Handle Ctrl+C - send interrupt
        if (e.ctrlKey && e.key === "c") {
          e.preventDefault();
          await ptyWrite(sessionId, "\x03");
          setInput("");
          clearToolMode();
          // Fallback: clear pendingCommand if shell integration events don't fire
          setTimeout(() => {
            const store = useStore.getState();
            if (store.pendingCommand[sessionId]) {
              store.handlePromptStart(sessionId);
            }
          }, 500);
          return;
        }

        // Handle Ctrl+D - send EOF
        if (e.ctrlKey && e.key === "d") {
          e.preventDefault();
          await ptyWrite(sessionId, "\x04");
          return;
        }

        // Handle Ctrl+L - clear timeline and command blocks
        if (e.ctrlKey && e.key === "l") {
          e.preventDefault();
          clearTerminal(sessionId);
          return;
        }
      }
    },
    [
      sessionId,
      handleSubmit,
      navigateUp,
      navigateDown,
      toggleInputMode,
      handleSlashSelect,
      handleFileSelect,
      handlePathSelect,
      handlePathSelectFinal,
      handleHistorySelect,
      handleToolSelect,
      clearToolMode,
    ]
  );

  // Render pane-level drop zone overlay using portal
  const paneDropOverlay =
    isDragOver && paneContainerRef.current
      ? createPortal(
          <div className="absolute inset-0 flex items-center justify-center z-50 pointer-events-none bg-background/60 backdrop-blur-[1px] border-2 border-dashed border-accent rounded-lg m-1">
            <div
              className={cn(
                "px-6 py-3 rounded-lg text-sm font-medium shadow-lg",
                dragError
                  ? "bg-destructive/90 text-destructive-foreground"
                  : "bg-accent text-accent-foreground"
              )}
            >
              {dragError || "Drop images here"}
            </div>
          </div>,
          paneContainerRef.current
        )
      : null;

  return (
    <>
      {paneDropOverlay}
      <div className="border-t border-[var(--border-subtle)]">
        {/* Inline Task Plan - only shown when a plan exists */}
        <InlineTaskPlan sessionId={sessionId} />

        {/* Path badge row - minimal Warp-style */}
        <ContextBar sessionId={sessionId} isAgentBusy={isAgentBusy} />

        {/* Tool params hint panel — shown when in tool mode */}
        {activeTool && toolParams.length > 0 && (
          <div className="px-3 py-1.5 border-b border-[var(--border-subtle)] flex flex-wrap gap-1">
            {toolParams.map((p) => {
              const escapedFlag = p.flag.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
              const alreadyUsed = new RegExp(`(^|\\s)${escapedFlag}(\\s|$)`).test(input);
              return (
                <button
                  key={p.flag}
                  type="button"
                  className={cn(
                    "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] transition-colors cursor-pointer",
                    alreadyUsed
                      ? "bg-accent/20 text-accent border border-accent/30 hover:bg-destructive/20 hover:text-destructive hover:border-destructive/30"
                      : p.required
                        ? "bg-accent/15 text-accent border border-accent/30 hover:bg-accent/25"
                        : "bg-muted/40 text-muted-foreground hover:bg-muted/60",
                  )}
                  onClick={() => {
                    const ta = textareaRef.current;
                    if (!ta) return;

                    if (alreadyUsed) {
                      // Remove: strip the flag and its value from the input
                      let newInput = input;
                      if (p.type === "boolean") {
                        newInput = newInput.replace(new RegExp(`\\s*${p.flag.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`), "");
                      } else {
                        newInput = newInput.replace(new RegExp(`\\s*${p.flag.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s+\\S*`), "");
                      }
                      setInput(newInput.trim());
                      requestAnimationFrame(() => { ta.focus(); });
                      return;
                    }

                    // Insert just the flag (+ space for non-boolean types)
                    const insert = p.type === "boolean" ? p.flag : `${p.flag} `;
                    const pos = ta.selectionStart ?? input.length;
                    const before = input.slice(0, pos);
                    const after = input.slice(pos);
                    const spaceBefore = before.length > 0 && !before.endsWith(" ") ? " " : "";
                    const spaceAfter = after.length > 0 && !after.startsWith(" ") ? " " : "";
                    const newInput = `${before}${spaceBefore}${insert}${spaceAfter}${after}`;
                    setInput(newInput);
                    requestAnimationFrame(() => {
                      const newPos = (before + spaceBefore + insert).length;
                      ta.focus();
                      ta.setSelectionRange(newPos, newPos);
                    });
                  }}
                  title={p.description || p.label}
                >
                  <span className="font-mono font-medium">{p.flag}</span>
                  <span className="opacity-70">{p.label}</span>
                  {p.required && !alreadyUsed && <span className="text-accent">*</span>}
                  {alreadyUsed && <span className="opacity-50">✓</span>}
                </button>
              );
            })}
          </div>
        )}

        {/* Input row with container */}
        <div className="px-3 py-2 border-y border-[var(--border-subtle)]">
          <div
            ref={dropZoneRef}
            className={cn(
              "relative flex items-center gap-2 rounded-md bg-background px-2 py-1.5",
              "transition-all duration-150",
              // Drag-over states
              isDragOver && !dragError && ["bg-accent/10"],
              isDragOver && dragError && ["bg-destructive/10"]
            )}
          >
            {/* Tool search mode badge (/t) */}
            {isToolSearchMode && !activeTool && (
              <div className="flex items-center gap-1 h-[26px] px-2 rounded-md bg-orange-500/15 border border-orange-500/30 shrink-0 self-center">
                <span className="text-[12px] font-medium text-orange-400 leading-none">{t("toolSearch.title")}</span>
                <button
                  type="button"
                  className="w-3.5 h-3.5 flex items-center justify-center rounded-full hover:bg-orange-500/20 text-orange-400/60 hover:text-orange-400 transition-colors"
                  onClick={() => setInput("")}
                >
                  <span className="text-[10px] leading-none">×</span>
                </button>
              </div>
            )}

            {/* Tool launch mode badge */}
            {activeTool && (
              <div className="flex items-center gap-1.5 h-[26px] px-2 rounded-md bg-accent/15 border border-accent/30 shrink-0 self-center">
                <div className="w-4 h-4 rounded bg-accent/20 flex items-center justify-center">
                  <span className="text-[9px] font-bold text-accent">
                    {activeTool.runtime === "python" ? "Py" : activeTool.runtime === "java" ? "Jv" : activeTool.runtime === "node" ? "Js" : "⌘"}
                  </span>
                </div>
                <span className="text-[13px] font-medium text-accent leading-none">{activeTool.name}</span>
                <button
                  type="button"
                  className="w-3.5 h-3.5 flex items-center justify-center rounded-full hover:bg-accent/20 text-accent/60 hover:text-accent transition-colors"
                  onClick={() => {
                    clearToolMode();
                    setInput("");
                  }}
                >
                  ×
                </button>
              </div>
            )}
            <div ref={inputContainerRef} className="relative flex-1 min-w-0">
              <textarea
                ref={textareaRef}
                data-testid="unified-input"
                data-mode={inputMode}
                value={showHistorySearch ? "" : isToolSearchMode ? toolSearchQuery : input}
                onChange={(e) => {
                  let value = e.target.value;

                  // When badge is visible, user types query only — prepend /t prefix
                  if (isToolSearchMode && !value.toLowerCase().startsWith("/t")) {
                    value = `/t ${value}`;
                  }

                  setInput(value);
                  resetHistory();

                  // Update path query when typing with popup open (for live filtering)
                  if (showPathPopup && inputMode === "terminal") {
                    // Use the new cursor position (end of input after typing)
                    const newCursorPos = e.target.selectionStart ?? value.length;
                    const { word } = extractWordAtCursor(value, newCursorPos);
                    if (word) {
                      // Update query for live filtering
                      setPathQuery(word);
                      setPathSelectedIndex(0);
                    } else {
                      // Close popup if word becomes empty (e.g., typed a space)
                      setShowPathPopup(false);
                    }
                  }

                  // /t prefix — tool search mode
                  const isToolInput = /^\/t\s/i.test(value);
                  if (isToolInput && inputMode === "terminal" && !activeTool) {
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

                    // Show slash popup when "/" is typed at the start
                    if (value.startsWith("/") && value.length >= 1) {
                      const afterSlash = value.slice(1);
                      const spaceIdx = afterSlash.indexOf(" ");
                      const commandPart =
                        spaceIdx === -1 ? afterSlash : afterSlash.slice(0, spaceIdx);
                      const exactMatch = commands.some((c) => c.name === commandPart);

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

                    // Show file popup when "@" is typed (agent mode only)
                    if (inputMode !== "terminal" && /@[^\s@]*$/.test(value)) {
                      setShowFilePopup(true);
                      setFileSelectedIndex(0);
                    } else {
                      setShowFilePopup(false);
                    }
                  }
                }}
                onKeyDown={handleKeyDown}
                onPaste={handlePaste}
                disabled={isSessionDead}
                placeholder={
                  showHistorySearch
                    ? ""
                    : isSessionDead
                      ? "Session limit exceeded. Please start a new session."
                      : isCompacting
                        ? "Compacting conversation..."
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
                  "placeholder:text-muted-foreground"
                )}
                style={isBlockCaret ? { caretColor: "transparent" } : undefined}
                onFocus={() => setIsFocused(true)}
                onBlur={() => setIsFocused(false)}
                spellCheck={false}
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
              />
              {/* Custom block caret overlay */}
              <BlockCaret
                textareaRef={textareaRef}
                text={input}
                settings={caretSettings}
                visible={isFocused && isBlockCaret && !isSessionDead}
              />
              {/* Ghost completion hint - shown inline after current input */}
              {ghostText && inputMode === "terminal" && !showHistorySearch && (
                <GhostTextHint text={ghostText} inputLength={input.length} />
              )}
              {/* Popup siblings - rendered conditionally, positioned absolute */}
              <HistorySearchPopup
                open={showHistorySearch}
                onOpenChange={setShowHistorySearch}
                matches={historyMatches}
                selectedIndex={historySelectedIndex}
                searchQuery={historySearchQuery}
                onSelect={handleHistorySelect}
                containerRef={inputContainerRef}
              />
              <PathCompletionPopup
                open={showPathPopup}
                onOpenChange={setShowPathPopup}
                completions={pathCompletions}
                totalCount={pathTotalCount}
                selectedIndex={pathSelectedIndex}
                onSelect={handlePathSelect}
                containerRef={inputContainerRef}
              />
              <SlashCommandPopup
                open={showSlashPopup}
                onOpenChange={setShowSlashPopup}
                commands={filteredSlashCommands}
                selectedIndex={slashSelectedIndex}
                onSelect={handleSlashSelect}
                containerRef={inputContainerRef}
              />
              <FileCommandPopup
                open={showFilePopup}
                onOpenChange={setShowFilePopup}
                files={files}
                selectedIndex={fileSelectedIndex}
                onSelect={handleFileSelect}
                containerRef={inputContainerRef}
              />
              <ToolSearchPopup
                open={showToolPopup && toolMatches.length > 0}
                onOpenChange={setShowToolPopup}
                tools={toolMatches}
                selectedIndex={toolSelectedIndex}
                onSelect={handleToolSelect}
                containerRef={inputContainerRef}
              />
            </div>

            {/* Image attachment (only shown in agent mode when vision is supported) */}
            {inputMode !== "terminal" && (
              <ImageAttachment
                attachments={imageAttachments}
                onAttachmentsChange={setImageAttachments}
                capabilities={visionCapabilities}
                disabled={isInputDisabled}
              />
            )}

            {/* Send button */}
            <button
              type="button"
              data-testid="send-button"
              onClick={handleSubmit}
              disabled={(!input.trim() && imageAttachments.length === 0) || isInputDisabled}
              className={cn(
                "h-7 w-7 flex items-center justify-center self-center rounded-md shrink-0",
                "transition-all duration-150",
                (input.trim() || imageAttachments.length > 0) && !isInputDisabled
                  ? "text-foreground hover:text-foreground/70"
                  : "text-muted-foreground/40 cursor-not-allowed"
              )}
            >
              <SendHorizontal className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* Status row - hidden for cleaner layout, functionality moved to AI Chat Panel */}
      </div>
    </>
  );
}
