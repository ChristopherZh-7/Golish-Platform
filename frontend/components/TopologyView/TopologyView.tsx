import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";
import {
  Bug,
  ChevronRight,
  Clock,
  Diff,
  Globe,
  Minus,
  Network,
  Plus,
  RefreshCw,
  Save,
  Server,
  Trash2,
  Upload,
  X,
  Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import cytoscape from "cytoscape";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";

const SCAN_COMMAND_PATTERNS = /^(nmap|masscan|rustscan)\s/;

const FINDING_SEV_BADGE: Record<string, string> = {
  critical: "bg-red-500/15 text-red-400 border-red-500/25",
  high: "bg-orange-500/15 text-orange-400 border-orange-500/25",
  medium: "bg-yellow-500/15 text-yellow-400 border-yellow-500/25",
  low: "bg-blue-500/15 text-blue-400 border-blue-500/25",
  info: "bg-slate-500/15 text-slate-400 border-slate-500/25",
};

const FINDING_SEV_LABEL: Record<string, string> = {
  critical: "Crit",
  high: "High",
  medium: "Med",
  low: "Low",
  info: "Info",
};

/** Fixed coordinates + preset layout — no force-directed motion or spin. */
const TOPOLOGY_PRESET_LAYOUT: cytoscape.LayoutOptions = {
  name: "preset",
  fit: true,
  padding: 48,
  animate: false,
};

function computeStaticNodePositions(data: TopologyData): Map<string, { x: number; y: number }> {
  const m = new Map<string, { x: number; y: number }>();
  if (data.nodes.length === 0) return m;
  const gapX = 140;
  const gapY = 80;
  const nets = data.nodes.filter((n) => n.node_type === "network");
  const hosts = data.nodes.filter((n) => n.node_type !== "network");

  if (nets.length === 0) {
    const rows = Math.min(8, Math.ceil(Math.sqrt(data.nodes.length)));
    data.nodes.forEach((n, i) => {
      const col = Math.floor(i / rows);
      const row = i % rows;
      const countThisCol = Math.min(rows, data.nodes.length - col * rows);
      const colH = Math.max(0, countThisCol - 1) * gapY;
      m.set(n.id, { x: col * gapX, y: row * gapY - colH / 2 });
    });
    return m;
  }

  const netsH = Math.max(0, nets.length - 1) * gapY;
  nets.forEach((n, i) => {
    m.set(n.id, { x: 0, y: i * gapY - netsH / 2 });
  });

  const rows = Math.min(8, Math.max(1, hosts.length));
  hosts.forEach((n, i) => {
    const col = Math.floor(i / rows) + 1;
    const row = i % rows;
    const countThisCol = Math.min(rows, hosts.length - (col - 1) * rows);
    const colH = Math.max(0, countThisCol - 1) * gapY;
    m.set(n.id, { x: col * gapX, y: row * gapY - colH / 2 });
  });
  return m;
}

function applyStaticPositionsToCy(cy: cytoscape.Core) {
  const nodes: TopoNode[] = [];
  cy.nodes().forEach((ele) => {
    const d = ele.data();
    nodes.push({
      id: d.id,
      label: d.label,
      node_type: d.nodeType,
      ip: d.ip ?? null,
      ports: d.ports ?? [],
      os: d.os ?? null,
      status: d.status,
    });
  });
  const pos = computeStaticNodePositions({
    nodes,
    edges: [],
    scan_info: null,
  });
  cy.nodes().forEach((n) => {
    const p = pos.get(n.id());
    if (p) n.position(p);
  });
}

function stripAnsi(str: string): string {
  // eslint-disable-next-line no-control-regex
  return str.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "").replace(/\x1b\].*?(\x07|\x1b\\)/g, "");
}

interface TopoPort {
  port: number;
  protocol: string;
  state: string;
  service: string;
}

interface TopoNode {
  id: string;
  label: string;
  node_type: string;
  ip: string | null;
  ports: TopoPort[];
  os: string | null;
  status: string;
}

interface TopoEdge {
  source: string;
  target: string;
  label: string | null;
}

interface TopologyData {
  nodes: TopoNode[];
  edges: TopoEdge[];
  scan_info: string | null;
}

function MapItem({ name, displayName, onLoad, onDelete }: {
  name: string; displayName: string;
  onLoad: (n: string) => void; onDelete: (n: string) => void;
}) {
  return (
    <div
      className="group flex items-center gap-1.5 px-2 py-1 rounded-md hover:bg-muted/50 cursor-pointer text-xs"
      onClick={() => onLoad(name)}
    >
      <Clock className="w-3 h-3 text-muted-foreground/60 flex-shrink-0" />
      <span className="truncate flex-1 text-muted-foreground">{displayName}</span>
      <button
        className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-opacity"
        onClick={(e) => { e.stopPropagation(); onDelete(name); }}
      >
        <Trash2 className="w-2.5 h-2.5" />
      </button>
    </div>
  );
}

function TargetGroup({ target, items, onLoad, onDelete }: {
  target: string; items: string[];
  onLoad: (n: string) => void; onDelete: (n: string) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div
        className="flex items-center gap-1 px-2 py-1.5 rounded-md hover:bg-muted/50 cursor-pointer text-xs font-medium"
        onClick={() => setOpen(!open)}
      >
        <ChevronRight className={cn("w-3 h-3 text-muted-foreground transition-transform", open && "rotate-90")} />
        <Globe className="w-3 h-3 text-accent/70 flex-shrink-0" />
        <span className="truncate flex-1">{target}</span>
        <span className="text-[10px] text-muted-foreground/50">{items.length}</span>
      </div>
      {open && (
        <div className="pl-3">
          {items.map((name) => {
            const ts = name.slice(name.indexOf("/") + 1);
            const parts = ts.split("T");
            const date = parts[0] || ts;
            const time = (parts[1] || "").replace(/-/g, ":");
            const display = time ? `${date} ${time}` : date;
            return <MapItem key={name} name={name} displayName={display} onLoad={onLoad} onDelete={onDelete} />;
          })}
        </div>
      )}
    </div>
  );
}

export function TopologyView() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<cytoscape.Core | null>(null);
  const [topoData, setTopoData] = useState<TopologyData | null>(null);
  const [savedMaps, setSavedMaps] = useState<string[]>([]);
  const [currentName, setCurrentName] = useState("");
  const [showPaste, setShowPaste] = useState(false);
  const [pasteContent, setPasteContent] = useState("");
  const [selectedNode, setSelectedNode] = useState<TopoNode | null>(null);
  const [nodeFindings, setNodeFindings] = useState<{ id: string; title: string; severity: string; status: string }[]>([]);

  const [showDiff, setShowDiff] = useState(false);
  const [diffA, setDiffA] = useState("");
  const [diffB, setDiffB] = useState("");
  const [diffResult, setDiffResult] = useState<{
    new_hosts: string[]; removed_hosts: string[]; new_ports: string[]; removed_ports: string[]; changed_services: string[];
  } | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);

  useEffect(() => {
    if (!selectedNode) { setNodeFindings([]); return; }
    const host = selectedNode.ip || selectedNode.label;
    invoke<{ id: string; title: string; severity: string; status: string }[]>("findings_for_host", {
      host,
      projectPath: getProjectPath(),
    }).then(setNodeFindings).catch(() => setNodeFindings([]));
  }, [selectedNode]);

  const handleDiff = useCallback(async () => {
    if (!diffA || !diffB || diffA === diffB) return;
    setDiffLoading(true);
    try {
      const result = await invoke<typeof diffResult>("topo_diff", {
        nameA: diffA,
        nameB: diffB,
        projectPath: getProjectPath(),
      });
      setDiffResult(result);
    } catch {
      setDiffResult(null);
    }
    setDiffLoading(false);
  }, [diffA, diffB]);

  const loadSavedList = useCallback(async () => {
    try {
      const list = await invoke<string[]>("topo_list", { projectPath: getProjectPath() });
      setSavedMaps(list);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    loadSavedList();
  }, [loadSavedList, currentProjectPath]);

  const pendingDataRef = useRef<TopologyData | null>(null);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryCountRef = useRef(0);
  const MAX_RETRIES = 20;

  useEffect(() => {
    return () => { if (retryTimerRef.current) clearTimeout(retryTimerRef.current); };
  }, []);

  const renderCytoscape = useCallback((data: TopologyData) => {
    const container = containerRef.current;
    if (!container) return;
    const parent = container.parentElement;
    if (!parent) return;

    const parentRect = parent.getBoundingClientRect();
    if (parentRect.width <= 0 || parentRect.height <= 0) {
      if (retryCountRef.current < MAX_RETRIES) {
        retryCountRef.current++;
        pendingDataRef.current = data;
        if (retryTimerRef.current) clearTimeout(retryTimerRef.current);
        retryTimerRef.current = setTimeout(() => renderCytoscape(data), 200);
      }
      return;
    }
    retryCountRef.current = 0;

    if (cyRef.current) cyRef.current.destroy();

    container.style.position = "absolute";
    container.style.top = "0";
    container.style.left = "0";
    container.style.width = `${parentRect.width}px`;
    container.style.height = `${parentRect.height}px`;
    container.style.overflow = "hidden";
    container.style.opacity = "0";

    const positions = computeStaticNodePositions(data);
    const elements: cytoscape.ElementDefinition[] = [];
    for (const node of data.nodes) {
      const p = positions.get(node.id) ?? { x: 0, y: 0 };
      const displayLabel =
        node.node_type === "network"
          ? `${node.ip || node.label} (${t("topology.localMachine", "本机")})`
          : node.label;
      elements.push({
        data: {
          id: node.id, label: displayLabel, nodeType: node.node_type,
          ip: node.ip, ports: node.ports, os: node.os,
          status: node.status, portCount: node.ports.length,
        },
        position: p,
      });
    }
    for (const edge of data.edges) {
      elements.push({ data: { source: edge.source, target: edge.target, label: edge.label || "" } });
    }

    const svgUri = (svg: string) =>
      `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;

    const computerSvg = svgUri(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect x="5" y="4" width="30" height="22" rx="2.5" fill="#1e293b" stroke="#a78bfa" stroke-width="1.2"/><rect x="7" y="6" width="26" height="18" rx="1.5" fill="#0f172a"/><line x1="14" y1="26" x2="14" y2="30" stroke="#475569" stroke-width="1.2"/><line x1="26" y1="26" x2="26" y2="30" stroke="#475569" stroke-width="1.2"/><rect x="10" y="30" width="20" height="2.5" rx="1.25" fill="#1e293b" stroke="#a78bfa" stroke-width="0.8"/><circle cx="20" cy="15" r="3" fill="none" stroke="#a78bfa" stroke-width="0.8" opacity="0.6"/><circle cx="20" cy="15" r="1" fill="#a78bfa" opacity="0.8"/></svg>`
    );

    const serverSvg = svgUri(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#38bdf8" stroke-width="1.2"/><ellipse cx="20" cy="20" rx="14" ry="5" fill="none" stroke="#38bdf8" stroke-width="0.6" opacity="0.5"/><ellipse cx="20" cy="20" rx="5" ry="14" fill="none" stroke="#38bdf8" stroke-width="0.6" opacity="0.5"/><line x1="6" y1="20" x2="34" y2="20" stroke="#38bdf8" stroke-width="0.5" opacity="0.3"/><line x1="20" y1="6" x2="20" y2="34" stroke="#38bdf8" stroke-width="0.5" opacity="0.3"/><circle cx="20" cy="20" r="2" fill="#38bdf8" opacity="0.7"/></svg>`
    );

    const cy = cytoscape({
      container,
      elements,
      style: [
        {
          selector: "node",
          style: {
            label: "data(label)",
            color: "#94a3b8",
            "font-size": "10px",
            "font-weight": "500",
            "text-valign": "bottom",
            "text-halign": "center",
            "text-margin-y": 6,
            "min-zoomed-font-size": 8,
            "overlay-padding": "4px",
            "background-color": "transparent",
            "background-fit": "contain",
            "background-clip": "none",
            "background-width": "100%",
            "background-height": "100%",
            "border-width": 0,
            shape: "rectangle",
          },
        },
        {
          selector: "node[nodeType='network']",
          style: {
            "background-image": computerSvg,
            width: 36,
            height: 36,
            "text-max-width": "120px",
          },
        },
        {
          selector: "node[nodeType='host']",
          style: {
            "background-image": serverSvg,
            width: 32,
            height: 32,
            shape: "ellipse",
          },
        },
        {
          selector: "node[status='down']",
          style: { opacity: 0.35, color: "#475569" },
        },
        {
          selector: "edge",
          style: {
            width: 1,
            "line-color": "#334155",
            "target-arrow-color": "#475569",
            "target-arrow-shape": "triangle",
            "arrow-scale": 0.6,
            "curve-style": "straight",
            opacity: 0.5,
          },
        },
        {
          selector: "node:active",
          style: { "overlay-color": "#6366f1", "overlay-opacity": 0.12 },
        },
        {
          selector: ":selected",
          style: { "overlay-color": "#f59e0b", "overlay-opacity": 0.2 },
        },
      ],
      layout: TOPOLOGY_PRESET_LAYOUT,
      minZoom: 0.3, maxZoom: 5,
    });

    requestAnimationFrame(() => {
      cy.resize();
      cy.fit(undefined, 60);
      const fitZ = cy.zoom();
      cy.zoom({
        level: fitZ * 0.55,
        renderedPosition: { x: container.clientWidth / 2, y: container.clientHeight / 2 },
      });
      container.style.transition = "opacity 150ms ease-in";
      container.style.opacity = "1";
    });

    cy.on("tap", "node", (evt) => {
      const nodeData = evt.target.data();
      if (nodeData.nodeType === "host") {
        setSelectedNode({
          id: nodeData.id, label: nodeData.label, node_type: nodeData.nodeType,
          ip: nodeData.ip, ports: nodeData.ports || [], os: nodeData.os, status: nodeData.status,
        });
      }
    });
    cy.on("tap", (evt) => { if (evt.target === cy) setSelectedNode(null); });
    cyRef.current = cy;
    pendingDataRef.current = null;
    setTopoData(data);
  }, []);

  useEffect(() => {
    const el = containerRef.current?.parentElement;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
          const container = containerRef.current;
          if (container && container.style.width) {
            container.style.width = `${entry.contentRect.width}px`;
            container.style.height = `${entry.contentRect.height}px`;
          }
          if (cyRef.current) {
            cyRef.current.resize();
            cyRef.current.fit(undefined, 40);
          } else if (pendingDataRef.current) {
            renderCytoscape(pendingDataRef.current);
          }
        }
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [renderCytoscape]);

  const initCytoscape = useCallback((data: TopologyData) => {
    renderCytoscape(data);
  }, [renderCytoscape]);

  const processedBlocksRef = useRef(new Set<string>());
  const [autoParseCount, setAutoParseCount] = useState(0);

  const timelines = useStore((s) => s.timelines);
  useEffect(() => {
    const allBlocks = Object.values(timelines).flat();
    for (const block of allBlocks) {
      if (block.type !== "command") continue;
      if (processedBlocksRef.current.has(block.id)) continue;
      const cmd = block.data.command?.trim();
      if (!cmd || !SCAN_COMMAND_PATTERNS.test(cmd)) continue;
      const output = block.data.output;
      if (!output || output.length < 50) continue;

      processedBlocksRef.current.add(block.id);
      const cleaned = stripAnsi(output);
      invoke<TopologyData>("topo_parse", { rawOutput: cleaned })
        .then((data) => {
          if (data.nodes.length > 0) {
            renderCytoscape(data);
            setAutoParseCount((c) => c + 1);
            const parts = cmd?.split(/\s+/) || [];
            const target = parts.filter((p) => p && !p.startsWith("-")).pop() || "scan";
            const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
            const autoName = `${target}/${ts}`;
            invoke("topo_save", {
              name: autoName,
              data,
              projectPath: getProjectPath(),
            }).then(() => loadSavedList()).catch(() => {});
          }
        })
        .catch((err) => {
        });
    }
  }, [timelines, loadSavedList, renderCytoscape]);

  const handleParsePaste = useCallback(async () => {
    if (!pasteContent.trim()) return;
    try {
      const data = await invoke<TopologyData>("topo_parse", {
        rawOutput: pasteContent,
        projectPath: getProjectPath(),
      });
      initCytoscape(data);
      setShowPaste(false);
      setPasteContent("");
    } catch (e) {
      console.error("Parse failed:", e);
    }
  }, [pasteContent, initCytoscape]);

  const handleSave = useCallback(async () => {
    if (!currentName.trim() || !cyRef.current) return;
    const cy = cyRef.current;
    const nodes: TopoNode[] = [];
    const edges: TopoEdge[] = [];

    cy.nodes().forEach((n) => {
      const d = n.data();
      nodes.push({
        id: d.id,
        label: d.label,
        node_type: d.nodeType,
        ip: d.ip,
        ports: d.ports || [],
        os: d.os,
        status: d.status,
      });
    });

    cy.edges().forEach((e) => {
      const d = e.data();
      edges.push({ source: d.source, target: d.target, label: d.label || null });
    });

    try {
      await invoke("topo_save", {
        name: currentName,
        data: { nodes, edges, scan_info: null },
        projectPath: getProjectPath(),
      });
      await loadSavedList();
      invoke("audit_log", { action: "topology_saved", category: "topology", details: currentName, projectPath: getProjectPath() }).catch(() => {});
    } catch (e) {
      console.error("Save failed:", e);
    }
  }, [currentName, loadSavedList]);

  const handleLoad = useCallback(
    async (name: string) => {
      try {
        const data = await invoke<TopologyData>("topo_load", { name, projectPath: getProjectPath() });
        setCurrentName(name);
        initCytoscape(data);
      } catch (e) {
        console.error("Load failed:", e);
      }
    },
    [initCytoscape]
  );

  const handleDelete = useCallback(
    async (name: string) => {
      try {
        await invoke("topo_delete", { name, projectPath: getProjectPath() });
        await loadSavedList();
      } catch (e) {
        console.error("Delete failed:", e);
      }
    },
    [loadSavedList]
  );

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/30 flex-shrink-0">
        <Network className="w-4 h-4 text-accent" />
        <span className="text-sm font-medium">{t("topology.title", "Network Topology")}</span>
        {autoParseCount > 0 && (
          <span className="flex items-center gap-1 text-[10px] text-accent/70 bg-accent/10 px-1.5 py-0.5 rounded">
            <Zap className="w-2.5 h-2.5" />
            Auto-parsed {autoParseCount} scan{autoParseCount > 1 ? "s" : ""}
          </span>
        )}
        <div className="flex-1" />
        <button
          className="flex items-center gap-1 px-2 py-1 text-xs rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => setShowPaste(true)}
        >
          <Upload className="w-3.5 h-3.5" />
          {t("topology.import", "Import Scan")}
        </button>
        {savedMaps.length >= 2 && (
          <button
            className="flex items-center gap-1 px-2 py-1 text-xs rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => { setShowDiff(true); setDiffResult(null); }}
          >
            <Diff className="w-3.5 h-3.5" />
            Compare
          </button>
        )}
        {cyRef.current && (
          <button
            className="flex items-center gap-1 px-2 py-1 text-xs rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => {
              const cy = cyRef.current;
              if (!cy) return;
              applyStaticPositionsToCy(cy);
              cy.layout(TOPOLOGY_PRESET_LAYOUT).run();
              cy.one("layoutstop", () => {
                cy.resize();
                cy.fit(undefined, 60);
                const fitZ = cy.zoom();
                cy.zoom({
                  level: fitZ * 0.55,
                  renderedPosition: { x: cy.width() / 2, y: cy.height() / 2 },
                });
              });
            }}
          >
            <RefreshCw className="w-3.5 h-3.5" />
            {t("topology.relayout", "Re-layout")}
          </button>
        )}
      </div>

      <div className="relative flex-1 min-h-0 overflow-hidden">
        <div className="absolute inset-0 flex">
        {/* Sidebar - saved maps */}
        <div className="w-48 border-r border-border/30 flex flex-col flex-shrink-0">
          <div className="px-3 py-2 text-[10px] uppercase tracking-wider text-muted-foreground font-medium">
            {t("topology.savedMaps", "Saved Maps")}
          </div>
          <div className="flex-1 overflow-y-auto px-1">
            {savedMaps.length === 0 && (
              <div className="px-2 py-4 text-xs text-muted-foreground/60 text-center">
                {t("topology.noMaps", "No saved maps")}
              </div>
            )}
            {(() => {
              const groups: Record<string, string[]> = {};
              const ungrouped: string[] = [];
              for (const name of savedMaps) {
                const slashIdx = name.indexOf("/");
                if (slashIdx > 0) {
                  const target = name.slice(0, slashIdx);
                  (groups[target] ||= []).push(name);
                } else {
                  ungrouped.push(name);
                }
              }
              return (
                <>
                  {Object.entries(groups).map(([target, items]) => (
                    <TargetGroup
                      key={target}
                      target={target}
                      items={items}
                      onLoad={handleLoad}
                      onDelete={handleDelete}
                    />
                  ))}
                  {ungrouped.map((name) => (
                    <MapItem key={name} name={name} displayName={name} onLoad={handleLoad} onDelete={handleDelete} />
                  ))}
                </>
              );
            })()}
          </div>

          {/* Save controls */}
          {topoData && (
            <div className="px-2 py-2 border-t border-border/30 flex-shrink-0">
              <div className="flex gap-1">
                <input
                  className="flex-1 min-w-0 text-xs px-2 py-1 rounded bg-background border border-border/50 focus:border-accent outline-none"
                  placeholder={t("topology.mapName", "Map name...")}
                  value={currentName}
                  onChange={(e) => setCurrentName(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && handleSave()}
                />
                <button
                  className="p-1 rounded hover:bg-accent/20 text-muted-foreground hover:text-accent transition-colors flex-shrink-0"
                  onClick={handleSave}
                  disabled={!currentName.trim()}
                >
                  <Save className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
          )}
        </div>

        {/* Main canvas */}
        <div className="flex-1 min-w-0 relative">
          <div
            ref={containerRef}
            className="absolute inset-0"
            style={{
              background: "var(--background)",
              backgroundImage:
                "radial-gradient(circle, hsl(var(--muted-foreground) / 0.08) 1px, transparent 1px)",
              backgroundSize: "24px 24px",
            }}
          />

          {/* Empty state */}
          {!cyRef.current && !showPaste && (
            <div className="absolute inset-0 flex items-center justify-center">
              <div className="flex flex-col items-center gap-3 text-muted-foreground/60">
                <Network className="w-12 h-12" />
                <p className="text-sm">{t("topology.empty", "No topology data")}</p>
                <button
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-accent/10 text-accent hover:bg-accent/20 text-xs font-medium transition-colors"
                  onClick={() => setShowPaste(true)}
                >
                  <Plus className="w-3.5 h-3.5" />
                  {t("topology.importScan", "Import nmap/masscan output")}
                </button>
              </div>
            </div>
          )}

          {/* Node details — right inspector */}
          {selectedNode && (
            <aside
              className={cn(
                "absolute top-3 right-3 z-10 flex max-h-[min(28rem,calc(100%-1.5rem))] w-[min(19rem,calc(100%-1.5rem))] flex-col overflow-hidden rounded-xl border border-border/60",
                "bg-popover/95 shadow-xl shadow-black/10 backdrop-blur-md",
                "ring-1 ring-black/[0.04] dark:ring-white/[0.06]",
              )}
            >
              <div className="flex items-start gap-2.5 border-b border-border/50 bg-muted/25 px-3 py-2.5">
                <div
                  className={cn(
                    "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg",
                    selectedNode.status === "down"
                      ? "bg-muted text-muted-foreground"
                      : "bg-accent/15 text-accent",
                  )}
                >
                  <Server className="h-4 w-4" strokeWidth={1.75} />
                </div>
                <div className="min-w-0 flex-1 pt-0.5">
                  <h3 className="truncate text-sm font-semibold leading-snug tracking-tight text-foreground">
                    {selectedNode.label}
                  </h3>
                  <div className="mt-1 flex flex-wrap items-center gap-1.5">
                    {selectedNode.status === "down" ? (
                      <span className="rounded-md border border-border/50 bg-muted/50 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                        {t("topology.hostDown", "Unreachable")}
                      </span>
                    ) : (
                      <span className="rounded-md border border-emerald-500/20 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                        {t("topology.hostUp", "Live")}
                      </span>
                    )}
                  </div>
                </div>
                <button
                  type="button"
                  className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted/80 hover:text-foreground"
                  onClick={() => setSelectedNode(null)}
                  aria-label={t("common.close", "Close")}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
                {(selectedNode.ip || selectedNode.os) && (
                  <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-xs">
                    {selectedNode.ip && (
                      <>
                        <dt className="text-muted-foreground">{t("topology.ip", "IP")}</dt>
                        <dd className="font-mono text-foreground tabular-nums">{selectedNode.ip}</dd>
                      </>
                    )}
                    {selectedNode.os && (
                      <>
                        <dt className="text-muted-foreground">{t("topology.os", "OS")}</dt>
                        <dd className="text-foreground">{selectedNode.os}</dd>
                      </>
                    )}
                  </dl>
                )}

                {selectedNode.ports.length > 0 && (
                  <div className={cn((selectedNode.ip || selectedNode.os) && "mt-4")}>
                    <div className="mb-2 flex items-center justify-between gap-2">
                      <span className="text-xs font-medium text-foreground">
                        {t("topology.ports", "Ports")}
                      </span>
                      <span className="rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                        {selectedNode.ports.length}
                      </span>
                    </div>
                    <ul className="space-y-1">
                      {selectedNode.ports.map((p) => {
                        const dot =
                          p.state === "open"
                            ? "bg-emerald-400 shadow-[0_0_0_2px_hsl(var(--background))] ring-1 ring-emerald-500/40"
                            : p.state === "filtered"
                              ? "bg-amber-400 shadow-[0_0_0_2px_hsl(var(--background))] ring-1 ring-amber-500/40"
                              : "bg-rose-400 shadow-[0_0_0_2px_hsl(var(--background))] ring-1 ring-rose-500/40";
                        return (
                          <li
                            key={`${p.port}-${p.protocol}`}
                            className="flex items-center gap-2 rounded-lg border border-border/40 bg-muted/15 px-2 py-1.5 text-xs"
                          >
                            <span className={cn("h-2 w-2 shrink-0 rounded-full", dot)} />
                            <span className="w-[4.25rem] shrink-0 font-mono tabular-nums text-foreground">
                              {p.port}/{p.protocol}
                            </span>
                            <span className="min-w-0 truncate text-muted-foreground">{p.service || "—"}</span>
                          </li>
                        );
                      })}
                    </ul>
                  </div>
                )}

                {nodeFindings.length > 0 && (
                  <div
                    className={cn(
                      "border-t border-border/40 pt-3",
                      (selectedNode.ip || selectedNode.os || selectedNode.ports.length > 0) && "mt-4",
                    )}
                  >
                    <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-foreground">
                      <Bug className="h-3.5 w-3.5 text-muted-foreground" strokeWidth={1.75} />
                      {t("topology.findings", "Findings")}
                      <span className="ml-auto rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                        {nodeFindings.length}
                      </span>
                    </div>
                    <ul className="space-y-1">
                      {nodeFindings.map((f) => (
                        <li
                          key={f.id}
                          className="flex items-start gap-2 rounded-lg border border-border/40 bg-muted/10 px-2 py-1.5"
                        >
                          <span
                            className={cn(
                              "mt-0.5 shrink-0 rounded border px-1 py-px text-[9px] font-semibold uppercase leading-none",
                              FINDING_SEV_BADGE[f.severity] || FINDING_SEV_BADGE.info,
                            )}
                          >
                            {FINDING_SEV_LABEL[f.severity] || FINDING_SEV_LABEL.info}
                          </span>
                          <span className="min-w-0 flex-1 text-[11px] leading-snug text-foreground">
                            {f.title}
                          </span>
                          <span className="shrink-0 text-[10px] capitalize text-muted-foreground/70">
                            {f.status}
                          </span>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}

                <div
                  className={cn(
                    "border-t border-border/40 pt-3",
                    (selectedNode.ip ||
                      selectedNode.os ||
                      selectedNode.ports.length > 0 ||
                      nodeFindings.length > 0) &&
                      "mt-4",
                  )}
                >
                  <p className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/80">
                    {t("topology.notes", "Notes")}
                  </p>
                  <QuickNotes entityType="topology" entityId={selectedNode.ip || selectedNode.label} compact />
                </div>
              </div>
            </aside>
          )}
        </div>
        </div>
      </div>

      {/* Paste dialog */}
      {showPaste && (
        <div className="absolute inset-0 z-30 bg-background/80 backdrop-blur-sm flex items-center justify-center">
          <div className="w-[600px] max-h-[80vh] bg-card rounded-xl border border-border/50 shadow-2xl flex flex-col">
            <div className="flex items-center gap-2 px-4 py-3 border-b border-border/30">
              <Upload className="w-4 h-4 text-accent" />
              <span className="text-sm font-medium">
                {t("topology.pasteTitle", "Paste nmap/masscan Output")}
              </span>
              <div className="flex-1" />
              <button
                className="p-1 rounded hover:bg-muted/50"
                onClick={() => {
                  setShowPaste(false);
                  setPasteContent("");
                }}
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="flex-1 p-4 min-h-0">
              <textarea
                className="w-full h-64 text-xs font-mono p-3 rounded-md bg-background border border-border/50 focus:border-accent outline-none resize-none"
                placeholder={`Paste nmap or masscan output here...\n\nExample:\nStarting Nmap 7.94 ...\nNmap scan report for 192.168.1.1\nPORT   STATE SERVICE\n22/tcp open  ssh\n80/tcp open  http\n443/tcp open https`}
                value={pasteContent}
                onChange={(e) => setPasteContent(e.target.value)}
                autoFocus
              />
            </div>
            <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-border/30">
              <button
                className="px-3 py-1.5 text-xs rounded-md hover:bg-muted/50 transition-colors"
                onClick={() => {
                  setShowPaste(false);
                  setPasteContent("");
                }}
              >
                {t("common.cancel", "Cancel")}
              </button>
              <button
                className="px-3 py-1.5 text-xs rounded-md bg-accent text-accent-foreground hover:bg-accent/90 transition-colors font-medium"
                onClick={handleParsePaste}
                disabled={!pasteContent.trim()}
              >
                {t("topology.parseAndVisualize", "Parse & Visualize")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Diff dialog */}
      {showDiff && (
        <div className="absolute inset-0 z-30 bg-background/80 backdrop-blur-sm flex items-center justify-center">
          <div className="w-[500px] max-h-[80vh] bg-card rounded-xl border border-border/50 shadow-2xl flex flex-col">
            <div className="flex items-center gap-2 px-4 py-3 border-b border-border/30">
              <Diff className="w-4 h-4 text-accent" />
              <span className="text-sm font-medium">Compare Scans</span>
              <div className="flex-1" />
              <button className="p-1 rounded hover:bg-muted/50" onClick={() => setShowDiff(false)}>
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="p-4 space-y-3">
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="text-[10px] text-muted-foreground/50 mb-1 block">Baseline (A)</label>
                  <select value={diffA} onChange={(e) => setDiffA(e.target.value)}
                    className="w-full text-xs px-2 py-1.5 rounded bg-background border border-border/50 text-foreground outline-none">
                    <option value="">Select...</option>
                    {savedMaps.map((m) => <option key={m} value={m}>{m}</option>)}
                  </select>
                </div>
                <div>
                  <label className="text-[10px] text-muted-foreground/50 mb-1 block">Current (B)</label>
                  <select value={diffB} onChange={(e) => setDiffB(e.target.value)}
                    className="w-full text-xs px-2 py-1.5 rounded bg-background border border-border/50 text-foreground outline-none">
                    <option value="">Select...</option>
                    {savedMaps.map((m) => <option key={m} value={m}>{m}</option>)}
                  </select>
                </div>
              </div>
              <button
                onClick={handleDiff}
                disabled={!diffA || !diffB || diffA === diffB || diffLoading}
                className={cn(
                  "w-full py-1.5 text-xs rounded-md font-medium transition-colors",
                  diffA && diffB && diffA !== diffB
                    ? "bg-accent text-accent-foreground hover:bg-accent/90"
                    : "bg-muted/20 text-muted-foreground/30 cursor-not-allowed",
                )}
              >
                {diffLoading ? "Comparing..." : "Compare"}
              </button>

              {diffResult && (
                <div className="space-y-2 max-h-[300px] overflow-y-auto border-t border-border/30 pt-3">
                  {diffResult.new_hosts.length === 0 && diffResult.removed_hosts.length === 0 &&
                    diffResult.new_ports.length === 0 && diffResult.removed_ports.length === 0 &&
                    diffResult.changed_services.length === 0 ? (
                    <p className="text-[11px] text-muted-foreground/50 text-center py-2">No differences found</p>
                  ) : (
                    <>
                      {diffResult.new_hosts.length > 0 && (
                        <div>
                          <div className="text-[10px] text-green-400 font-medium mb-1 flex items-center gap-1">
                            <Plus className="w-3 h-3" /> New Hosts ({diffResult.new_hosts.length})
                          </div>
                          {diffResult.new_hosts.map((h) => (
                            <div key={h} className="text-[10px] font-mono text-green-400/70 pl-4">{h}</div>
                          ))}
                        </div>
                      )}
                      {diffResult.removed_hosts.length > 0 && (
                        <div>
                          <div className="text-[10px] text-red-400 font-medium mb-1 flex items-center gap-1">
                            <Minus className="w-3 h-3" /> Removed Hosts ({diffResult.removed_hosts.length})
                          </div>
                          {diffResult.removed_hosts.map((h) => (
                            <div key={h} className="text-[10px] font-mono text-red-400/70 pl-4">{h}</div>
                          ))}
                        </div>
                      )}
                      {diffResult.new_ports.length > 0 && (
                        <div>
                          <div className="text-[10px] text-green-400 font-medium mb-1 flex items-center gap-1">
                            <Plus className="w-3 h-3" /> New Ports ({diffResult.new_ports.length})
                          </div>
                          {diffResult.new_ports.map((p) => (
                            <div key={p} className="text-[10px] font-mono text-green-400/70 pl-4">{p}</div>
                          ))}
                        </div>
                      )}
                      {diffResult.removed_ports.length > 0 && (
                        <div>
                          <div className="text-[10px] text-red-400 font-medium mb-1 flex items-center gap-1">
                            <Minus className="w-3 h-3" /> Closed Ports ({diffResult.removed_ports.length})
                          </div>
                          {diffResult.removed_ports.map((p) => (
                            <div key={p} className="text-[10px] font-mono text-red-400/70 pl-4">{p}</div>
                          ))}
                        </div>
                      )}
                      {diffResult.changed_services.length > 0 && (
                        <div>
                          <div className="text-[10px] text-yellow-400 font-medium mb-1 flex items-center gap-1">
                            <Diff className="w-3 h-3" /> Service Changes ({diffResult.changed_services.length})
                          </div>
                          {diffResult.changed_services.map((s) => (
                            <div key={s} className="text-[10px] font-mono text-yellow-400/70 pl-4">{s}</div>
                          ))}
                        </div>
                      )}
                    </>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
