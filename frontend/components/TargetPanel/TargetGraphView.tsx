import { AlertTriangle, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { organizations as orgsApi } from "@/lib/api";
import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { buildTopologyModel } from "./topology/buildTopologyModel";
import { TopologyCanvas } from "./topology/TopologyCanvas";
import { TopologyControls } from "./topology/TopologyControls";
import { TopologyInspector } from "./topology/TopologyInspector";
import type { TopologyMode, TopologyVisibility } from "./topology/types";

const DEFAULT_VISIBILITY: TopologyVisibility = {
  organization: true,
  target: true,
  service: true,
  evidence: true,
};

export function TargetGraphView({ targets }: { targets: Target[] }) {
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [orgLoading, setOrgLoading] = useState(true);
  const [orgError, setOrgError] = useState<string | null>(null);
  const [mode, setMode] = useState<TopologyMode>("ownership");
  const [visibility, setVisibility] = useState<TopologyVisibility>(DEFAULT_VISIBILITY);
  const [query, setQuery] = useState("");
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [fitSignal, setFitSignal] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setOrgLoading(true);
    setOrgError(null);
    orgsApi
      .listOrganizations(getProjectPath())
      .then((list) => {
        if (!cancelled) setOrganizations(list);
      })
      .catch((error) => {
        if (!cancelled) {
          setOrganizations([]);
          setOrgError(String(error));
        }
      })
      .finally(() => {
        if (!cancelled) setOrgLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const model = useMemo(
    () => buildTopologyModel(organizations, targets, { mode, visibility, query }),
    [organizations, targets, mode, visibility, query]
  );

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
      />

      <div className="relative flex min-w-0 flex-1">
        {orgLoading && (
          <div className="absolute left-4 top-4 z-20 inline-flex items-center gap-2 rounded-md border border-border/35 bg-card/90 px-3 py-2 text-[11px] text-muted-foreground shadow-sm">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            Loading organizations
          </div>
        )}
        {orgError && (
          <div className="absolute left-4 top-4 z-20 inline-flex max-w-md items-center gap-2 rounded-md border border-amber-400/25 bg-amber-400/10 px-3 py-2 text-[11px] text-amber-200 shadow-sm">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            {orgError}
          </div>
        )}
        <TopologyCanvas
          model={model}
          selectedNodeId={selectedNode?.id ?? null}
          fitSignal={fitSignal}
          onSelectNode={setSelectedNodeId}
        />
      </div>

      <TopologyInspector node={selectedNode} />
    </div>
  );
}
