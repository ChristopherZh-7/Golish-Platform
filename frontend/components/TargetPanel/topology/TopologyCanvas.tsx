import { Building2, Database, Globe2, Server, ShieldAlert } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import { cn } from "@/lib/utils";
import type { TopologyEdge, TopologyModel, TopologyNode } from "./types";

export function TopologyCanvas({
  model,
  selectedNodeId,
  fitSignal,
  onSelectNode,
}: {
  model: TopologyModel;
  selectedNodeId: string | null;
  fitSignal: number;
  onSelectNode: (nodeId: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectedNode = useMemo(
    () => model.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [model.nodes, selectedNodeId]
  );

  useEffect(() => {
    if (!selectedNode || !scrollRef.current) return;
    const element = scrollRef.current.querySelector(
      `[data-node-id="${cssEscape(selectedNode.id)}"]`
    );
    element?.scrollIntoView({ block: "center", inline: "center", behavior: "smooth" });
  }, [selectedNode, fitSignal]);

  if (model.nodes.length === 0) {
    return (
      <div className="flex h-full min-w-0 flex-1 items-center justify-center bg-background/20">
        <div className="text-center text-muted-foreground">
          <Globe2 className="mx-auto h-10 w-10 opacity-50" />
          <div className="mt-3 text-sm">No topology nodes</div>
          <div className="mt-1 text-xs opacity-70">Create an organization or import targets.</div>
        </div>
      </div>
    );
  }

  return (
    <div className="relative min-w-0 flex-1 overflow-hidden bg-background/20">
      <div className="flex h-[58px] items-center justify-between border-b border-border/25 bg-card/25 px-4">
        <div>
          <div className="text-[16px] font-semibold text-foreground">Topology</div>
          <div className="mt-0.5 text-[11px] text-muted-foreground">
            Organization to target to service, with evidence as the contract
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          {["Root", "Sub org", "Target", "Service", "Evidence"].map((label) => (
            <span
              key={label}
              className="rounded-md border border-border/30 bg-background/25 px-2 py-1 text-[10px] font-medium text-muted-foreground"
            >
              {label}
            </span>
          ))}
        </div>
      </div>

      <div
        ref={scrollRef}
        className="absolute inset-x-0 bottom-0 top-[58px] overflow-auto"
        style={{
          backgroundImage:
            "linear-gradient(hsl(var(--border) / 0.17) 1px, transparent 1px), linear-gradient(90deg, hsl(var(--border) / 0.17) 1px, transparent 1px)",
          backgroundSize: "24px 24px",
        }}
      >
        <div
          className="relative"
          style={{
            width: model.bounds.width,
            height: model.bounds.height,
            minWidth: "100%",
          }}
        >
          <ColumnGuide x={72} label="ROOT ORG" height={model.bounds.height} />
          <ColumnGuide x={268} label="SUB ORG" height={model.bounds.height} />
          <ColumnGuide x={486} label="TARGET" height={model.bounds.height} />
          <ColumnGuide x={704} label="SERVICE" height={model.bounds.height} />
          <ColumnGuide x={884} label="EVIDENCE" height={model.bounds.height} />

          <svg
            className="pointer-events-none absolute inset-0"
            width={model.bounds.width}
            height={model.bounds.height}
            viewBox={`0 0 ${model.bounds.width} ${model.bounds.height}`}
          >
            <title>Topology edges</title>
            {model.edges.map((edge) => (
              <TopologyEdgePath key={edge.id} edge={edge} model={model} />
            ))}
          </svg>

          {model.nodes.map((node) => (
            <TopologyNodeBlock
              key={node.id}
              node={node}
              selected={node.id === selectedNodeId}
              onSelect={() => onSelectNode(node.id)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function ColumnGuide({ x, label, height }: { x: number; label: string; height: number }) {
  return (
    <>
      <div
        className="absolute top-8 bottom-8 w-px bg-border/20"
        style={{ left: x + 8, height: height - 80 }}
      />
      <div
        className="absolute top-4 -translate-x-1/2 text-[10px] font-bold text-muted-foreground/65"
        style={{ left: x + 68 }}
      >
        {label}
      </div>
    </>
  );
}

function TopologyEdgePath({ edge, model }: { edge: TopologyEdge; model: TopologyModel }) {
  const source = model.nodes.find((node) => node.id === edge.source);
  const target = model.nodes.find((node) => node.id === edge.target);
  if (!source || !target) return null;

  const sx = source.x + source.width;
  const sy = source.y + source.height / 2;
  const tx = target.x;
  const ty = target.y + target.height / 2;
  const mid = Math.max(36, (tx - sx) / 2);
  const path = `M ${sx} ${sy} C ${sx + mid} ${sy}, ${tx - mid} ${ty}, ${tx} ${ty}`;
  const stroke =
    edge.kind === "produced"
      ? "hsl(42 88% 68% / 0.78)"
      : edge.kind === "exposes"
        ? "hsl(216 100% 74% / 0.76)"
        : edge.kind === "contains"
          ? "hsl(174 70% 67% / 0.84)"
          : "hsl(188 86% 62% / 0.82)";
  const strokeWidth = edge.kind === "owns" ? 2.1 : 1.5;

  return (
    <path d={path} fill="none" stroke={stroke} strokeWidth={strokeWidth} strokeLinecap="round" />
  );
}

function TopologyNodeBlock({
  node,
  selected,
  onSelect,
}: {
  node: TopologyNode;
  selected: boolean;
  onSelect: () => void;
}) {
  const Icon =
    node.kind === "organization"
      ? Building2
      : node.kind === "target"
        ? Globe2
        : node.kind === "service"
          ? Server
          : Database;
  const tone =
    node.kind === "organization"
      ? "text-cyan-300"
      : node.kind === "target"
        ? "text-blue-300"
        : node.kind === "service"
          ? "text-emerald-300"
          : "text-amber-300";

  return (
    <button
      type="button"
      data-node-id={node.id}
      className={cn(
        "absolute rounded-lg border px-3 py-2 text-left shadow-sm transition-colors",
        selected
          ? "border-cyan-300/70 bg-cyan-300/10 shadow-cyan-950/30"
          : "border-border/35 bg-card/85 hover:border-border/60 hover:bg-muted/20",
        node.scope === "out" && "opacity-55"
      )}
      style={{
        left: node.x,
        top: node.y,
        width: node.width,
        height: node.height,
      }}
      onClick={onSelect}
    >
      <div className="flex min-w-0 items-center gap-2">
        <Icon className={cn("h-3.5 w-3.5 shrink-0", tone)} />
        <span className="min-w-0 truncate text-[11px] font-semibold text-foreground">
          {node.label}
        </span>
        {node.kind === "evidence" && node.metrics?.findings ? (
          <ShieldAlert className="ml-auto h-3 w-3 shrink-0 text-amber-300" />
        ) : null}
      </div>
      <div className="mt-1 truncate text-[10px] text-muted-foreground">{node.subtitle}</div>
      <NodeMetric node={node} />
    </button>
  );
}

function NodeMetric({ node }: { node: TopologyNode }) {
  if (node.kind === "organization") {
    return (
      <div className="mt-1 text-[9px] text-muted-foreground/75">
        {node.metrics?.inScopeTargets ?? 0}/{node.metrics?.targets ?? 0} in scope
      </div>
    );
  }
  if (node.kind === "target") {
    return (
      <div className="mt-1 text-[9px] text-muted-foreground/75">
        {node.metrics?.ports ?? 0} ports · {node.metrics?.evidence ?? 0} evidence
      </div>
    );
  }
  return null;
}

function cssEscape(value: string) {
  if (typeof CSS !== "undefined" && CSS.escape) return CSS.escape(value);
  return value.replace(/"/g, '\\"');
}
