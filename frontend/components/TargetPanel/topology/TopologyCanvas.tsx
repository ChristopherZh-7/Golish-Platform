import {
  Building2,
  Crosshair,
  Database,
  Globe2,
  Minus,
  Plus,
  Server,
  ShieldAlert,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import type { TopologyEdge, TopologyModel, TopologyNode } from "./types";

const MIN_ZOOM = 0.3;
const MAX_ZOOM = 2.5;

function clampZoom(value: number) {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, value));
}

export function TopologyCanvas({
  model,
  selectedNodeId,
  focusNodeId,
  relatedNodeIds,
  fitSignal,
  onSelectNode,
  onFocusNode,
  onClearFocus,
}: {
  model: TopologyModel;
  selectedNodeId: string | null;
  focusNodeId: string | null;
  relatedNodeIds: Set<string> | null;
  fitSignal: number;
  onSelectNode: (nodeId: string) => void;
  onFocusNode: (nodeId: string) => void;
  onClearFocus: () => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const selectedNode = useMemo(
    () => model.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [model.nodes, selectedNodeId]
  );
  const focusedNode = useMemo(
    () => (focusNodeId ? (model.nodes.find((node) => node.id === focusNodeId) ?? null) : null),
    [model.nodes, focusNodeId]
  );

  const [zoom, setZoom] = useState(1);
  const zoomRef = useRef(1);
  const anchorRef = useRef<{
    contentX: number;
    contentY: number;
    viewX: number;
    viewY: number;
  } | null>(null);

  useLayoutEffect(() => {
    zoomRef.current = zoom;
    const container = scrollRef.current;
    const anchor = anchorRef.current;
    if (!container || !anchor) return;
    container.scrollLeft = anchor.contentX * zoom - anchor.viewX;
    container.scrollTop = anchor.contentY * zoom - anchor.viewY;
    anchorRef.current = null;
  }, [zoom]);

  // Native non-passive wheel listener so we can preventDefault. Trackpad pinch
  // arrives as a wheel event with ctrlKey set; Ctrl/Cmd + wheel works for mice.
  // Plain scroll passes through so the canvas keeps scrolling normally.
  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const rect = container.getBoundingClientRect();
      const viewX = event.clientX - rect.left;
      const viewY = event.clientY - rect.top;
      const current = zoomRef.current;
      const contentX = (container.scrollLeft + viewX) / current;
      const contentY = (container.scrollTop + viewY) / current;
      anchorRef.current = { contentX, contentY, viewX, viewY };
      setZoom(clampZoom(current * Math.exp(-event.deltaY * 0.0015)));
    };
    container.addEventListener("wheel", onWheel, { passive: false });
    return () => container.removeEventListener("wheel", onWheel);
  }, []);

  const zoomByFactor = (factor: number) => {
    const container = scrollRef.current;
    if (container) {
      const viewX = container.clientWidth / 2;
      const viewY = container.clientHeight / 2;
      const current = zoomRef.current;
      anchorRef.current = {
        contentX: (container.scrollLeft + viewX) / current,
        contentY: (container.scrollTop + viewY) / current,
        viewX,
        viewY,
      };
    }
    setZoom((prev) => clampZoom(prev * factor));
  };
  const resetZoom = () => {
    anchorRef.current = null;
    setZoom(1);
  };

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
          {["Root", "Sub org", "Target", "Service", "Surface"].map((label) => (
            <span
              key={label}
              className="rounded-md border border-border/30 bg-background/25 px-2 py-1 text-[10px] font-medium text-muted-foreground"
            >
              {label}
            </span>
          ))}
        </div>
      </div>

      {focusedNode && (
        <div className="absolute left-3 top-[66px] z-20 inline-flex items-center gap-2 rounded-md border border-cyan-300/40 bg-card/95 px-2.5 py-1.5 text-[11px] shadow-sm">
          <Crosshair className="h-3 w-3 shrink-0 text-cyan-300" />
          <span className="text-muted-foreground">Focused</span>
          <span className="max-w-[180px] truncate font-semibold text-foreground">
            {focusedNode.label}
          </span>
          <button
            type="button"
            className="ml-1 inline-flex items-center gap-1 rounded border border-border/40 px-1.5 py-0.5 text-[10px] text-muted-foreground transition-colors hover:border-border/70 hover:text-foreground"
            onClick={onClearFocus}
          >
            <X className="h-3 w-3" />
            Exit
          </button>
        </div>
      )}

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
          style={{
            width: model.bounds.width * zoom,
            height: model.bounds.height * zoom,
            minWidth: "100%",
          }}
        >
          <div
            className="relative"
            style={{
              width: model.bounds.width,
              height: model.bounds.height,
              transform: `scale(${zoom})`,
              transformOrigin: "0 0",
            }}
          >
            <ColumnGuide x={72} label="ROOT ORG" height={model.bounds.height} />
            <ColumnGuide x={268} label="SUB ORG" height={model.bounds.height} />
            <ColumnGuide x={486} label="TARGET" height={model.bounds.height} />
            <ColumnGuide x={704} label="SERVICE" height={model.bounds.height} />
            <ColumnGuide x={884} label="SURFACE" height={model.bounds.height} />

            <svg
              className="pointer-events-none absolute inset-0"
              width={model.bounds.width}
              height={model.bounds.height}
              viewBox={`0 0 ${model.bounds.width} ${model.bounds.height}`}
            >
              <title>Topology edges</title>
              {model.edges.map((edge) => (
                <TopologyEdgePath
                  key={edge.id}
                  edge={edge}
                  model={model}
                  dimmed={isEdgeDimmed(edge, relatedNodeIds)}
                />
              ))}
            </svg>

            {model.nodes.map((node) => (
              <TopologyNodeBlock
                key={node.id}
                node={node}
                selected={node.id === selectedNodeId}
                focused={node.id === focusNodeId}
                dimmed={relatedNodeIds != null && !relatedNodeIds.has(node.id)}
                onSelect={() => onSelectNode(node.id)}
                onFocus={() => onFocusNode(node.id)}
              />
            ))}
          </div>
        </div>
      </div>

      <div className="absolute bottom-3 right-3 z-20 flex items-center gap-0.5 rounded-md border border-border/35 bg-card/95 p-1 shadow-sm">
        <button
          type="button"
          title="Zoom out"
          className="inline-flex h-6 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted/30 hover:text-foreground"
          onClick={() => zoomByFactor(1 / 1.2)}
        >
          <Minus className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          title="Reset zoom"
          className="min-w-[42px] rounded px-1 text-center text-[10px] font-semibold text-muted-foreground transition-colors hover:bg-muted/30 hover:text-foreground"
          onClick={resetZoom}
        >
          {Math.round(zoom * 100)}%
        </button>
        <button
          type="button"
          title="Zoom in"
          className="inline-flex h-6 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted/30 hover:text-foreground"
          onClick={() => zoomByFactor(1.2)}
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
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

function isEdgeDimmed(edge: TopologyEdge, relatedNodeIds: Set<string> | null) {
  if (!relatedNodeIds) return false;
  return !(relatedNodeIds.has(edge.source) && relatedNodeIds.has(edge.target));
}

function TopologyEdgePath({
  edge,
  model,
  dimmed,
}: {
  edge: TopologyEdge;
  model: TopologyModel;
  dimmed: boolean;
}) {
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
    <path
      d={path}
      fill="none"
      stroke={stroke}
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      opacity={dimmed ? 0.12 : 1}
    />
  );
}

function TopologyNodeBlock({
  node,
  selected,
  focused,
  dimmed,
  onSelect,
  onFocus,
}: {
  node: TopologyNode;
  selected: boolean;
  focused: boolean;
  dimmed: boolean;
  onSelect: () => void;
  onFocus: () => void;
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
      title="Double-click to isolate this unit"
      className={cn(
        "absolute rounded-lg border px-3 py-2 text-left shadow-sm transition-all",
        selected
          ? "border-cyan-300/70 bg-cyan-300/10 shadow-cyan-950/30"
          : "border-border/35 bg-card/85 hover:border-border/60 hover:bg-muted/20",
        focused && "ring-2 ring-cyan-300/70 ring-offset-1 ring-offset-background",
        node.scope === "out" && !dimmed && "opacity-55",
        dimmed && "opacity-20 hover:opacity-70"
      )}
      style={{
        left: node.x,
        top: node.y,
        width: node.width,
        height: node.height,
      }}
      onClick={onSelect}
      onDoubleClick={onFocus}
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
        {node.metrics?.ports ?? 0} ports · {node.metrics?.endpoints ?? 0} API ·{" "}
        {node.metrics?.params ?? 0} params
      </div>
    );
  }
  if (
    node.kind === "evidence" &&
    (node.metrics?.endpoints || node.metrics?.paths || node.metrics?.js)
  ) {
    return (
      <div className="mt-1 text-[9px] text-muted-foreground/75">
        {node.metrics?.paths ?? 0} paths · {node.metrics?.js ?? 0} JS
      </div>
    );
  }
  return null;
}

function cssEscape(value: string) {
  if (typeof CSS !== "undefined" && CSS.escape) return CSS.escape(value);
  return value.replace(/"/g, '\\"');
}
