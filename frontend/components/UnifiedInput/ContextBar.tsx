import { ArrowDown, ArrowUp, Folder, GitBranch, Package } from "lucide-react";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { useUnifiedInputState } from "@/store/selectors/unified-input";
import { selectDisplaySettings } from "@/store/slices";

interface ContextBarProps {
  sessionId: string;
  isAgentBusy: boolean;
}

export function ContextBar({ sessionId, isAgentBusy }: ContextBarProps) {
  const workingDirectory = useStore((state) => state.sessions[sessionId]?.workingDirectory);
  const openGitPanel = useStore((state) => state.openGitPanel);
  const { virtualEnv, gitBranch, gitStatus } = useUnifiedInputState(sessionId);
  const display = useStore(selectDisplaySettings);

  // Abbreviate path like fish shell: ~/C/p/my-project
  const displayPath = (() => {
    if (!workingDirectory) return "~";
    const withTilde = workingDirectory.replace(/^\/Users\/[^/]+/, "~");
    const parts = withTilde.split("/");
    if (parts.length <= 2) return withTilde;
    const first = parts[0];
    const last = parts[parts.length - 1];
    const middle = parts.slice(1, -1).map((p) => p[0] || p);
    return [first, ...middle, last].join("/");
  })();

  const parentOn = display.showTerminalContext;
  const pathVisible = parentOn && display.showWorkingDirectory;
  const gitVisible = parentOn && display.showGitBranch;
  const rowVisible = pathVisible || gitVisible;

  if (!rowVisible) return null;

  return (
    <div>
      <div
        className={cn(
          "flex items-center gap-2 px-3 py-1",
          isAgentBusy && "agent-loading-shimmer"
        )}
      >
        {/* Path badge - Warp style */}
        {pathVisible && (
          <div
            className="h-[18px] px-1.5 gap-1 text-[11px] rounded bg-muted/40 inline-flex items-center shrink-0"
            title={workingDirectory || "~"}
          >
            <Folder className="w-3 h-3 text-[#e0af68] shrink-0" />
            <span className="text-muted-foreground">{displayPath}</span>
          </div>
        )}

        {/* Git badge hidden - functionality available via Git panel */}

        {/* Virtual env badge — always visible, not gated by display settings */}
        {virtualEnv && (
          <div className="h-5 px-1.5 gap-1 text-xs rounded bg-[#9ece6a]/10 text-[#9ece6a] flex items-center border border-[#9ece6a]/20 shrink-0">
            <Package className="size-icon-context-bar" />
            <span>{virtualEnv}</span>
          </div>
        )}
      </div>
    </div>
  );
}
