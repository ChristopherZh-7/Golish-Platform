import { AlertTriangle, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { organizations as orgsApi } from "@/lib/api";
import type { Organization } from "@/lib/api/organizations";
import { onCustomEvent, onEvent } from "@/lib/events";
import { listDirectoryEntries } from "@/lib/pentest/api";
import type { Target } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { apiEndpointsList, jsAnalysisList } from "@/lib/security-analysis";
import { countEndpointParams } from "./surface/endpointParams";
import {
  applyTopologyFocus,
  buildTopologyModel,
  collectLineageIds,
} from "./topology/buildTopologyModel";
import { TopologyCanvas } from "./topology/TopologyCanvas";
import { TopologyControls } from "./topology/TopologyControls";
import { TopologyInspector } from "./topology/TopologyInspector";
import type {
  TargetTopologySurfaceSummary,
  TopologyMode,
  TopologyVisibility,
} from "./topology/types";

const DEFAULT_VISIBILITY: TopologyVisibility = {
  organization: true,
  target: true,
  service: true,
  evidence: true,
};
const SURFACE_REFRESH_TOOLS = new Set([
  "browser_collect_js_api",
  "js_extract_apis",
  "route_probe_paths",
  "pentest_run",
  "output_parse_and_store",
  "discover_apis",
]);
const SURFACE_SUMMARY_CONCURRENCY = 8;

async function loadTargetTopologySurfaceSummary(
  targetId: string
): Promise<TargetTopologySurfaceSummary> {
  const [endpoints, jsResults, directoryEntries] = await Promise.all([
    apiEndpointsList(targetId),
    jsAnalysisList(targetId),
    listDirectoryEntries({ targetId }),
  ]);
  const pathKeys = new Set<string>();
  for (const entry of directoryEntries) {
    if (entry.url) pathKeys.add(entry.url);
  }
  for (const endpoint of endpoints) {
    const path = endpoint.url || endpoint.path;
    if (path) pathKeys.add(path);
  }
  return {
    endpoints: endpoints.length,
    params: countEndpointParams(endpoints),
    paths: pathKeys.size,
    js: jsResults.length,
  };
}

async function loadTopologySurfaceSummaries(
  targets: Target[]
): Promise<Map<string, TargetTopologySurfaceSummary>> {
  const ids = [...new Set(targets.map((target) => target.id).filter(Boolean))];
  const out = new Map<string, TargetTopologySurfaceSummary>();
  let index = 0;
  const worker = async () => {
    while (index < ids.length) {
      const id = ids[index];
      index += 1;
      try {
        out.set(id, await loadTargetTopologySurfaceSummary(id));
      } catch {
        out.set(id, { endpoints: 0, params: 0, paths: 0, js: 0 });
      }
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(SURFACE_SUMMARY_CONCURRENCY, ids.length) }, () => worker())
  );
  return out;
}

export function TargetGraphView({ targets }: { targets: Target[] }) {
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [surfaceByTargetId, setSurfaceByTargetId] = useState(
    new Map<string, TargetTopologySurfaceSummary>()
  );
  const [orgLoading, setOrgLoading] = useState(true);
  const [surfaceLoading, setSurfaceLoading] = useState(false);
  const [orgError, setOrgError] = useState<string | null>(null);
  const [surfaceError, setSurfaceError] = useState<string | null>(null);
  const [mode, setMode] = useState<TopologyMode>("ownership");
  const [visibility, setVisibility] = useState<TopologyVisibility>(DEFAULT_VISIBILITY);
  const [query, setQuery] = useState("");
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [focusNodeId, setFocusNodeId] = useState<string | null>(null);
  const [fitSignal, setFitSignal] = useState(0);

  const loadOrgs = useCallback(async () => {
    setOrgLoading(true);
    setOrgError(null);
    try {
      setOrganizations(await orgsApi.listOrganizations(getProjectPath()));
    } catch (error) {
      setOrganizations([]);
      setOrgError(String(error));
    } finally {
      setOrgLoading(false);
    }
  }, []);

  const loadSurfaceSummaries = useCallback(async () => {
    if (targets.length === 0) {
      setSurfaceByTargetId(new Map());
      setSurfaceError(null);
      return;
    }
    setSurfaceLoading(true);
    setSurfaceError(null);
    try {
      setSurfaceByTargetId(await loadTopologySurfaceSummaries(targets));
    } catch (error) {
      setSurfaceByTargetId(new Map());
      setSurfaceError(String(error));
    } finally {
      setSurfaceLoading(false);
    }
  }, [targets]);

  useEffect(() => {
    loadOrgs();
  }, [loadOrgs]);

  useEffect(() => {
    void loadSurfaceSummaries();
  }, [loadSurfaceSummaries]);

  // Refresh the topology when the AI writes orgs (scoping:
  // manage_organizations / recon_discover_subsidiaries) or on the umbrella
  // `targets-changed` signal (also fired on switching into the Target view).
  // Without this the graph loaded orgs once on mount and went stale.
  useEffect(() => {
    const ORG_WRITE_TOOLS = new Set(["manage_organizations", "recon_discover_subsidiaries"]);
    const unlistenAi = onEvent("ai-event", (payload) => {
      const p = payload as { type?: string; tool_name?: string };
      if (p.type === "tool_result" && p.tool_name && ORG_WRITE_TOOLS.has(p.tool_name)) {
        loadOrgs();
      }
      if (p.type === "tool_result" && p.tool_name && SURFACE_REFRESH_TOOLS.has(p.tool_name)) {
        void loadSurfaceSummaries();
      }
    });
    const unlistenChanged = onCustomEvent("targets-changed", () => {
      loadOrgs();
      void loadSurfaceSummaries();
    });
    return () => {
      runTauriUnlistenFromPromise(unlistenAi);
      runTauriUnlistenFromPromise(unlistenChanged);
    };
  }, [loadOrgs, loadSurfaceSummaries]);

  const fullModel = useMemo(
    () =>
      buildTopologyModel(organizations, targets, {
        mode,
        visibility,
        query,
        surfaceByTargetId,
      }),
    [organizations, targets, mode, visibility, query, surfaceByTargetId]
  );

  const model = useMemo(() => applyTopologyFocus(fullModel, focusNodeId), [fullModel, focusNodeId]);

  const selectedNode = useMemo(
    () => model.nodes.find((node) => node.id === selectedNodeId) ?? model.nodes[0] ?? null,
    [model.nodes, selectedNodeId]
  );

  useEffect(() => {
    if (!selectedNode && selectedNodeId) {
      setSelectedNodeId(null);
      return;
    }
    if (!selectedNodeId && selectedNode) {
      setSelectedNodeId(selectedNode.id);
    }
  }, [selectedNode, selectedNodeId]);

  useEffect(() => {
    if (focusNodeId && !fullModel.nodes.some((node) => node.id === focusNodeId)) {
      setFocusNodeId(null);
    }
  }, [fullModel, focusNodeId]);

  const focusedNode = useMemo(
    () => (focusNodeId ? (fullModel.nodes.find((node) => node.id === focusNodeId) ?? null) : null),
    [fullModel.nodes, focusNodeId]
  );

  const relatedNodeIds = useMemo(() => {
    if (focusNodeId || !selectedNode) return null;
    return collectLineageIds(fullModel.edges, selectedNode.id);
  }, [focusNodeId, selectedNode, fullModel.edges]);

  const focusSelected = () => {
    if (selectedNode) setFocusNodeId(selectedNode.id);
  };
  const toggleFocusNode = (nodeId: string) => {
    setFocusNodeId((prev) => (prev === nodeId ? null : nodeId));
  };
  const clearFocus = () => setFocusNodeId(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
      if (event.key === "Escape" && focusNodeId) {
        setFocusNodeId(null);
      } else if ((event.key === "f" || event.key === "F") && selectedNode) {
        event.preventDefault();
        setFocusNodeId(selectedNode.id);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [focusNodeId, selectedNode]);

  const selectedLabel = selectedNode?.label ?? "No selection";

  return (
    <div className="flex h-full w-full overflow-hidden bg-background">
      <TopologyControls
        mode={mode}
        visibility={visibility}
        query={query}
        stats={model.stats}
        selectedLabel={selectedLabel}
        onModeChange={setMode}
        onVisibilityChange={(kind) => setVisibility((prev) => ({ ...prev, [kind]: !prev[kind] }))}
        onQueryChange={setQuery}
        onFitSelected={() => setFitSignal((value) => value + 1)}
        focusActive={focusNodeId != null}
        focusLabel={focusedNode?.label ?? null}
        canIsolate={selectedNode != null}
        onIsolateSelected={focusSelected}
        onClearFocus={clearFocus}
      />

      <div className="relative flex min-w-0 flex-1">
        {orgLoading && (
          <div className="absolute left-4 top-4 z-20 inline-flex items-center gap-2 rounded-md border border-border/35 bg-card/90 px-3 py-2 text-[11px] text-muted-foreground shadow-sm">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            Loading organizations
          </div>
        )}
        {surfaceLoading && !orgLoading && (
          <div className="absolute left-4 top-4 z-20 inline-flex items-center gap-2 rounded-md border border-border/35 bg-card/90 px-3 py-2 text-[11px] text-muted-foreground shadow-sm">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            Loading surface evidence
          </div>
        )}
        {orgError && (
          <div className="absolute left-4 top-4 z-20 inline-flex max-w-md items-center gap-2 rounded-md border border-amber-400/25 bg-amber-400/10 px-3 py-2 text-[11px] text-amber-200 shadow-sm">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            {orgError}
          </div>
        )}
        {surfaceError && (
          <div className="absolute left-4 top-16 z-20 inline-flex max-w-md items-center gap-2 rounded-md border border-amber-400/25 bg-amber-400/10 px-3 py-2 text-[11px] text-amber-200 shadow-sm">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            {surfaceError}
          </div>
        )}
        <TopologyCanvas
          model={model}
          selectedNodeId={selectedNode?.id ?? null}
          focusNodeId={focusNodeId}
          relatedNodeIds={relatedNodeIds}
          fitSignal={fitSignal}
          onSelectNode={setSelectedNodeId}
          onFocusNode={toggleFocusNode}
          onClearFocus={clearFocus}
        />
      </div>

      <TopologyInspector node={selectedNode} />
    </div>
  );
}
