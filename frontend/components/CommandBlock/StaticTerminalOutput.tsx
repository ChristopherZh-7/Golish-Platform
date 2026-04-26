import { FitAddon } from "@xterm/addon-fit";
import type { ILink, Terminal as TerminalType } from "@xterm/xterm";
import { Terminal } from "@xterm/xterm";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FilePathPopup } from "@/components/FilePathPopup";
import { useFileEditorSidebar } from "@/hooks/useFileEditorSidebar";
import { logger } from "@/lib/logger";
import { type DetectedPath, detectFilePaths } from "@/lib/pathDetection";
import { type ResolvedPath, resolvePath } from "@/lib/pathResolution";
import { ThemeManager } from "@/lib/theme";
import "@xterm/xterm/css/xterm.css";

// ANSI escape codes for styling detected file paths
// Using cyan (36) for accent color and underline (4)
const LINK_START = "\x1b[4;36m"; // underline + cyan
const LINK_END = "\x1b[24;39m"; // no underline + default color

/**
 * Highlights detected file paths in terminal output with ANSI styling.
 * Processes each line to find file paths and wraps them with color/underline codes.
 */
function highlightFilePaths(text: string): string {
  const lines = text.split("\n");
  const highlightedLines = lines.map((line) => {
    const detected = detectFilePaths(line);
    if (detected.length === 0) return line;

    // Build the line with highlighted paths
    // Process in reverse order to preserve indices
    let result = line;
    for (let i = detected.length - 1; i >= 0; i--) {
      const path = detected[i];
      result =
        result.slice(0, path.start) +
        LINK_START +
        result.slice(path.start, path.end) +
        LINK_END +
        result.slice(path.end);
    }
    return result;
  });
  return highlightedLines.join("\n");
}

interface StaticTerminalOutputProps {
  /** ANSI-formatted output to display */
  output: string;
  /** Session ID for file editor */
  sessionId?: string;
  /** Working directory for path resolution */
  workingDirectory?: string;
}

/**
 * Renders terminal output using xterm.js in read-only mode.
 * This ensures visual consistency with LiveTerminalBlock.
 */
export function StaticTerminalOutput({
  output,
  sessionId,
  workingDirectory,
}: StaticTerminalOutputProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<TerminalType | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  const [popupOpen, setPopupOpen] = useState(false);
  const [popupPosition, setPopupPosition] = useState<{ x: number; y: number } | null>(null);
  const [popupPaths, setPopupPaths] = useState<ResolvedPath[]>([]);
  const [popupLoading, setPopupLoading] = useState(false);
  const pendingDetectedRef = useRef<DetectedPath | null>(null);

  const { openFile } = useFileEditorSidebar(workingDirectory);

  const lineCount = output.split("\n").length;
  const isLargeOutput = lineCount > 100;
  const MAX_TERMINAL_HEIGHT = 600;

  // Estimated min-height prevents layout shift when the xterm.js terminal
  // hasn't rendered yet (fixes output "jumping up" on fast commands).
  const estimatedMinHeight = useMemo(() => {
    const lineHeightPx = 17;
    return Math.min(lineCount * lineHeightPx, MAX_TERMINAL_HEIGHT);
  }, [lineCount]);

  // Guard flag: suppress ResizeObserver's fitAddon.fit() during content write
  // to prevent resize loops that cause the "progressive deletion" visual bug.
  const isWritingRef = useRef(false);

  // Effect to create terminal (runs once on mount)
  useEffect(() => {
    if (!containerRef.current) return;

    if (!terminalRef.current) {
      const terminal = new Terminal({
        cursorBlink: false,
        cursorInactiveStyle: "none",
        disableStdin: true,
        fontSize: 12,
        fontFamily: "SF Mono, Menlo, Monaco, JetBrains Mono, Consolas, monospace",
        fontWeight: "normal",
        fontWeightBold: "bold",
        lineHeight: 1.4,
        scrollback: 10000,
        convertEol: true,
        allowProposedApi: true,
      });

      const fitAddon = new FitAddon();
      terminal.loadAddon(fitAddon);
      fitAddonRef.current = fitAddon;

      ThemeManager.applyToTerminal(terminal);

      terminal.options.fontSize = 12;
      terminal.options.lineHeight = 1.4;
      terminal.options.fontWeight = "normal";
      terminal.options.letterSpacing = 0;
      terminal.options.theme = {
        ...terminal.options.theme,
        background: "rgba(0,0,0,0)",
      };

      terminal.open(containerRef.current);
      terminalRef.current = terminal;

      // Fit columns to container width
      try {
        fitAddon.fit();
      } catch {
        /* ignore */
      }
    }

    // Re-fit when container resizes, but skip during content writes
    const container = containerRef.current;
    const observer = new ResizeObserver(() => {
      if (isWritingRef.current) return;
      if (fitAddonRef.current && terminalRef.current) {
        try {
          fitAddonRef.current.fit();
        } catch {
          /* ignore */
        }
      }
    });
    observer.observe(container);

    return () => {
      observer.disconnect();
      if (terminalRef.current) {
        terminalRef.current.dispose();
        terminalRef.current = null;
        fitAddonRef.current = null;
      }
    };
  }, []);

  // Effect to register link provider when sessionId/workingDirectory available
  useEffect(() => {
    if (!sessionId || !workingDirectory || !terminalRef.current) return;

    const terminal = terminalRef.current;
    const wdRef = workingDirectory; // Capture for closure

    const disposable = terminal.registerLinkProvider({
      provideLinks: (bufferLineNumber, callback) => {
        const buffer = terminal.buffer.active;
        const line = buffer.getLine(bufferLineNumber - 1);
        if (!line) {
          callback(undefined);
          return;
        }

        const lineText = line.translateToString(true);
        const detected = detectFilePaths(lineText);

        if (detected.length === 0) {
          callback(undefined);
          return;
        }

        const links: ILink[] = detected.map((pathInfo) => ({
          range: {
            start: { x: pathInfo.start + 1, y: bufferLineNumber },
            end: { x: pathInfo.end, y: bufferLineNumber },
          },
          text: pathInfo.raw,
          activate: async (event: MouseEvent) => {
            // Store the detected path for resolution
            pendingDetectedRef.current = pathInfo;

            setPopupLoading(true);
            setPopupPosition({ x: event.clientX, y: event.clientY });
            setPopupOpen(true);

            try {
              const resolved = await resolvePath(pathInfo, wdRef);
              setPopupPaths(resolved);
            } catch (error) {
              logger.error("Failed to resolve path:", error);
              setPopupPaths([]);
            } finally {
              setPopupLoading(false);
            }
          },
        }));

        callback(links);
      },
    });

    return () => {
      disposable.dispose();
    };
  }, [sessionId, workingDirectory]);

  // Pre-process output to highlight file paths when links are enabled
  const processedOutput = useMemo(() => {
    if (!sessionId || !workingDirectory || !output) return output;
    return highlightFilePaths(output);
  }, [output, sessionId, workingDirectory]);

  // Effect to write content.
  // Hide terminal during async write to prevent the "progressive deletion"
  // visual glitch where xterm progressively pushes old lines out of view.
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !processedOutput) return;

    isWritingRef.current = true;

    // Hide content while writing to avoid visual flicker
    const container = containerRef.current;
    if (container) {
      container.style.opacity = "0";
    }

    const cols = terminal.cols || 80;
    const lines = processedOutput.split("\n");
    let totalRows = 0;
    for (const line of lines) {
      const visibleLen = line.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "").length;
      totalRows += Math.max(1, Math.ceil(visibleLen / cols));
    }
    totalRows = Math.max(totalRows, 1);

    const maxVisibleRows = Math.floor(MAX_TERMINAL_HEIGHT / 17);
    const displayRows = isLargeOutput ? Math.min(totalRows, maxVisibleRows) : totalRows;

    if (terminal.rows !== displayRows || terminal.cols !== cols) {
      terminal.resize(cols, displayRows);
    }

    terminal.clear();
    terminal.write(processedOutput, () => {
      const actualRows = Math.max(terminal.buffer.active.length, 1);
      const nextRows = isLargeOutput ? Math.min(actualRows, maxVisibleRows) : actualRows;
      if (terminal.rows !== nextRows || terminal.cols !== cols) {
        terminal.resize(cols, nextRows);
      }
      terminal.scrollToTop();

      isWritingRef.current = false;
      if (container) {
        container.style.opacity = "1";
      }
    });
  }, [processedOutput, isLargeOutput]);

  const handleOpenFile = useCallback(
    (absolutePath: string, _line?: number, _column?: number) => {
      // TODO: Support line navigation when CodeMirror supports it
      openFile(absolutePath);
      setPopupOpen(false);
    },
    [openFile]
  );

  return (
    <>
      <div
        ref={containerRef}
        style={{
          minHeight: estimatedMinHeight,
          maxHeight: isLargeOutput ? MAX_TERMINAL_HEIGHT : undefined,
        }}
        className={
          isLargeOutput
            ? "overflow-hidden"
            : "overflow-hidden [&_.xterm-viewport]:!overflow-hidden [&_.xterm-screen]:!h-auto"
        }
      />
      {popupPosition && (
        <FilePathPopup
          open={popupOpen}
          onOpenChange={setPopupOpen}
          paths={popupPaths}
          loading={popupLoading}
          onOpenFile={handleOpenFile}
          position={popupPosition}
        />
      )}
    </>
  );
}
