import { FileText, GitCommit, GripVertical, Package, RefreshCw, ScrollText, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Markdown } from "@/components/Markdown/Markdown";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useThrottledResize } from "@/hooks/useThrottledResize";
import { onEvent } from "@/lib/events";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import {
  type Artifact,
  getAppliedPatches,
  getCurrentSession,
  getPendingArtifacts,
  getSessionLog,
  getSessionState,
  getStagedPatches,
  previewArtifact,
  type StagedPatch,
} from "@/lib/sidecar";
import { cn } from "@/lib/utils";
import { ArtifactsView } from "./ArtifactViews";
import { PatchesView } from "./PatchViews";

interface ContextPanelProps {
  /** Session ID to show context for (uses current session if not provided) */
  sessionId?: string;
  /** Whether the panel is open */
  open: boolean;
  /** Callback when panel should close */
  onOpenChange: (open: boolean) => void;
}

type TabId = "state" | "log" | "patches" | "artifacts";

const MIN_WIDTH = 300;
const MAX_WIDTH = 900;
const DEFAULT_WIDTH = 450;

/**
 * Side panel showing the current session's markdown state and log.
 * Displays the state.md (LLM-managed session context) and log.md (event history).
 * Renders inline as part of the flex layout (not a modal overlay).
 */
export function ContextPanel({ sessionId, open, onOpenChange }: ContextPanelProps) {
  const [activeTab, setActiveTab] = useState<TabId>("state");
  const [stateContent, setStateContent] = useState<string>("");
  const [logContent, setLogContent] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resolvedSessionId, setResolvedSessionId] = useState<string | null>(null);

  // Patches state
  const [stagedPatches, setStagedPatches] = useState<StagedPatch[]>([]);
  const [appliedPatches, setAppliedPatches] = useState<StagedPatch[]>([]);
  const [selectedPatchId, setSelectedPatchId] = useState<number | null>(null);

  // Artifacts state
  const [pendingArtifacts, setPendingArtifacts] = useState<Artifact[]>([]);
  const [selectedArtifact, setSelectedArtifact] = useState<string | null>(null);
  const [artifactPreview, setArtifactPreview] = useState<string | null>(null);

  // Resize state with RAF-based throttling
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  const { startResizing } = useThrottledResize({
    minWidth: MIN_WIDTH,
    maxWidth: MAX_WIDTH,
    onWidthChange: setWidth,
    calculateWidth: (e) => window.innerWidth - e.clientX,
  });

  // Fetch content for the current (or specified) session
  const fetchContent = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      // Resolve session ID
      let sid: string | undefined = sessionId;
      if (!sid) {
        sid = (await getCurrentSession()) ?? undefined;
      }

      if (!sid) {
        setError(null);
        setStateContent(
          "No active capture session.\n\nSend a message to the AI to start context capture."
        );
        setLogContent(
          "No active capture session.\n\nSend a message to the AI to start context capture."
        );
        setStagedPatches([]);
        setAppliedPatches([]);
        setPendingArtifacts([]);
        setResolvedSessionId(null);
        return;
      }

      setResolvedSessionId(sid);

      // Fetch all data in parallel
      const [state, log, staged, applied, artifacts] = await Promise.all([
        getSessionState(sid).catch(() => ""),
        getSessionLog(sid).catch(() => ""),
        getStagedPatches(sid).catch(() => []),
        getAppliedPatches(sid).catch(() => []),
        getPendingArtifacts(sid).catch(() => []),
      ]);

      setStateContent(state || "(empty)");
      setLogContent(log || "(empty)");
      setStagedPatches(staged);
      setAppliedPatches(applied);
      setPendingArtifacts(artifacts);
    } catch (e) {
      // Tauri errors may be strings, not Error objects
      const message =
        e instanceof Error
          ? e.message
          : typeof e === "string"
            ? e
            : "Failed to fetch session content";
      setError(message);
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  // Fetch content when panel opens
  useEffect(() => {
    if (!open) return;
    fetchContent();
  }, [open, fetchContent]);

  // Subscribe to sidecar events for auto-refresh
  useEffect(() => {
    if (!open) return;

    const unlisten = onEvent("sidecar-event", (payload) => {
      const eventType = payload.event_type;
      if (
        eventType === "session_started" ||
        eventType === "session_ended" ||
        eventType === "patch_created" ||
        eventType === "patch_applied" ||
        eventType === "patch_discarded" ||
        eventType === "artifact_created" ||
        eventType === "artifact_applied" ||
        eventType === "artifact_discarded"
      ) {
        fetchContent();
      }
    });

    return () => {
      runTauriUnlistenFromPromise(unlisten);
    };
  }, [open, fetchContent]);

  // Handle artifact preview loading
  useEffect(() => {
    if (!selectedArtifact || !resolvedSessionId) {
      setArtifactPreview(null);
      return;
    }

    setArtifactPreview(null);
    previewArtifact(resolvedSessionId, selectedArtifact)
      .then(setArtifactPreview)
      .catch(() => setArtifactPreview("Failed to load preview"));
  }, [selectedArtifact, resolvedSessionId]);

  // Get all patches combined with status
  const allPatches = [
    ...stagedPatches.map((p) => ({ ...p, status: "staged" as const })),
    ...appliedPatches.map((p) => ({ ...p, status: "applied" as const })),
  ].sort((a, b) => a.meta.id - b.meta.id);

  // Get selected patch
  const selectedPatch = allPatches.find((p) => p.meta.id === selectedPatchId) ?? null;

  // Get selected artifact data
  const selectedArtifactData =
    pendingArtifacts.find((a) => a.filename === selectedArtifact) ?? null;

  if (!open) return null;

  return (
    <div
      className="bg-card border-l border-border flex flex-col relative"
      style={{ width: `${width}px`, minWidth: `${MIN_WIDTH}px`, maxWidth: `${MAX_WIDTH}px` }}
    >
      {/* Resize handle */}
      <div
        className="absolute top-0 left-0 w-1 h-full cursor-col-resize hover:bg-[var(--ansi-blue)] transition-colors z-10 group"
        onMouseDown={startResizing}
      >
        <div className="absolute top-1/2 left-0 -translate-y-1/2 opacity-0 group-hover:opacity-100 transition-opacity">
          <GripVertical className="w-3 h-3 text-muted-foreground" />
        </div>
      </div>

      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border">
        <div className="flex items-center gap-2 min-w-0">
          <FileText className="w-4 h-4 text-muted-foreground shrink-0" />
          <h2 className="text-sm font-medium truncate">Session Context</h2>
          {resolvedSessionId && (
            <span className="text-xs text-muted-foreground font-mono shrink-0">
              {resolvedSessionId.slice(0, 8)}...
            </span>
          )}
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={fetchContent}
            disabled={loading}
          >
            <RefreshCw className={cn("w-3.5 h-3.5", loading && "animate-spin")} />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6"
            onClick={() => onOpenChange(false)}
          >
            <X className="w-3.5 h-3.5" />
          </Button>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-border">
        <button
          type="button"
          onClick={() => setActiveTab("state")}
          className={cn(
            "flex-1 px-3 py-1.5 text-xs font-medium transition-colors",
            activeTab === "state"
              ? "text-foreground border-b-2 border-[var(--ansi-blue)]"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          <FileText className="w-3.5 h-3.5 inline mr-1" />
          State
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("log")}
          className={cn(
            "flex-1 px-3 py-1.5 text-xs font-medium transition-colors",
            activeTab === "log"
              ? "text-foreground border-b-2 border-[var(--ansi-blue)]"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          <ScrollText className="w-3.5 h-3.5 inline mr-1" />
          Log
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("patches")}
          className={cn(
            "flex-1 px-3 py-1.5 text-xs font-medium transition-colors",
            activeTab === "patches"
              ? "text-foreground border-b-2 border-[var(--ansi-blue)]"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          <GitCommit className="w-3.5 h-3.5 inline mr-1" />
          Patches
          {allPatches.length > 0 && (
            <span className="ml-1 text-[10px] bg-muted px-1 rounded">{allPatches.length}</span>
          )}
        </button>
        <button
          type="button"
          onClick={() => setActiveTab("artifacts")}
          className={cn(
            "flex-1 px-3 py-1.5 text-xs font-medium transition-colors",
            activeTab === "artifacts"
              ? "text-foreground border-b-2 border-[var(--ansi-blue)]"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          <Package className="w-3.5 h-3.5 inline mr-1" />
          Artifacts
          {pendingArtifacts.length > 0 && (
            <span className="ml-1 text-[10px] bg-muted px-1 rounded">
              {pendingArtifacts.length}
            </span>
          )}
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 min-h-0 overflow-hidden flex flex-col">
        {error ? (
          <div className="text-[var(--ansi-red)] text-xs p-3">{error}</div>
        ) : loading ? (
          <div className="text-muted-foreground text-xs animate-pulse p-3">Loading...</div>
        ) : activeTab === "state" ? (
          <ScrollArea className="flex-1 min-h-0">
            <div className="p-3 text-xs [&_h1]:text-base [&_h2]:text-sm [&_h3]:text-xs [&_p]:text-xs [&_li]:text-xs [&_code]:text-[10px] [&_pre]:text-[10px]">
              <Markdown content={stateContent} />
            </div>
          </ScrollArea>
        ) : activeTab === "log" ? (
          <ScrollArea className="flex-1 min-h-0">
            <div className="p-3 text-xs [&_h1]:text-base [&_h2]:text-sm [&_h3]:text-xs [&_p]:text-xs [&_li]:text-xs [&_code]:text-[10px] [&_pre]:text-[10px]">
              <Markdown content={logContent} />
            </div>
          </ScrollArea>
        ) : activeTab === "patches" ? (
          <PatchesView
            patches={allPatches}
            selectedPatchId={selectedPatchId}
            selectedPatch={selectedPatch}
            onSelectPatch={setSelectedPatchId}
          />
        ) : (
          <ArtifactsView
            artifacts={pendingArtifacts}
            selectedArtifact={selectedArtifact}
            selectedArtifactData={selectedArtifactData}
            artifactPreview={artifactPreview}
            onSelectArtifact={setSelectedArtifact}
          />
        )}
      </div>

      {/* Footer */}
      <div className="px-3 py-1.5 border-t border-border text-[10px] text-muted-foreground">
        {activeTab === "state"
          ? "LLM-managed session state (state.md)"
          : activeTab === "log"
            ? "Append-only event log (log.md)"
            : activeTab === "patches"
              ? "Git patches from this session (staged & applied)"
              : "Generated documentation artifacts"}
      </div>
    </div>
  );
}
