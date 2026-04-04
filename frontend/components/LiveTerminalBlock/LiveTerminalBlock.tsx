import { useCallback, useEffect, useRef } from "react";
import { liveTerminalManager } from "@/lib/terminal";
import { ptyWrite } from "@/lib/tauri";
import "@xterm/xterm/css/xterm.css";
import "@/styles/xterm-overrides.css";

interface LiveTerminalBlockProps {
  sessionId: string;
  /** The command being executed (captured from OSC 133;C) */
  command: string | null;
  /** When true, terminal accepts keyboard input forwarded to the PTY */
  interactive?: boolean;
}

export const CODE_STYLE = {
  fontSize: "12px",
  lineHeight: 1.4,
  fontFamily: "JetBrains Mono, Menlo, Monaco, Consolas, monospace",
} as const;

export function LiveTerminalBlock({ sessionId, command, interactive }: LiveTerminalBlockProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  const handleData = useCallback(
    (data: string) => {
      ptyWrite(sessionId, data).catch(() => {});
    },
    [sessionId]
  );

  useEffect(() => {
    if (!containerRef.current) {
      return;
    }

    const container = containerRef.current;

    liveTerminalManager.getOrCreate(sessionId);
    liveTerminalManager.attachToContainer(sessionId, container);

    const resizeObserver = new ResizeObserver(() => {
      liveTerminalManager.fit(sessionId);
    });
    resizeObserver.observe(container);

    return () => {
      resizeObserver.disconnect();
      liveTerminalManager.detach(sessionId);
    };
  }, [sessionId]);

  // Toggle interactive mode based on prop
  useEffect(() => {
    if (interactive) {
      liveTerminalManager.enableInput(sessionId, handleData);
    } else {
      liveTerminalManager.disableInput(sessionId);
    }
    return () => {
      liveTerminalManager.disableInput(sessionId);
    };
  }, [sessionId, interactive, handleData]);

  // Auto-focus when interactive
  useEffect(() => {
    if (interactive) {
      liveTerminalManager.focus(sessionId);
    }
  }, [sessionId, interactive]);

  return (
    <div className="w-full flex-1 flex flex-col min-h-0">
      {/* Command header */}
      {command && (
        <div className="flex items-center gap-2 px-5 py-3 w-full shrink-0">
          <code className="flex-1 truncate text-[var(--ansi-white)]" style={CODE_STYLE}>
            <span className="text-[var(--ansi-green)]">$ </span>
            {command}
          </code>
          <span className="w-2 h-2 bg-[#7aa2f7] rounded-full animate-pulse flex-shrink-0" />
        </div>
      )}

      {/* Terminal container - grows to fill available space when interactive */}
      <div className={`px-5 pb-4 w-full ${interactive ? "flex-1 min-h-0" : ""}`}>
        <div
          ref={containerRef}
          className={`w-full overflow-hidden [&_.xterm-viewport]:!overflow-y-auto ${interactive ? "h-full" : "h-96"}`}
        />
      </div>
    </div>
  );
}
