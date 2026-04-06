import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";
import {
  FolderOpen,
  Network,
  Plus,
  RefreshCw,
  Save,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import cytoscape from "cytoscape";

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

export function TopologyView() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<cytoscape.Core | null>(null);
  const [savedMaps, setSavedMaps] = useState<string[]>([]);
  const [currentName, setCurrentName] = useState("");
  const [showPaste, setShowPaste] = useState(false);
  const [pasteContent, setPasteContent] = useState("");
  const [selectedNode, setSelectedNode] = useState<TopoNode | null>(null);

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

  const initCytoscape = useCallback(
    (data: TopologyData) => {
      if (!containerRef.current) return;

      if (cyRef.current) {
        cyRef.current.destroy();
      }

      const elements: cytoscape.ElementDefinition[] = [];

      for (const node of data.nodes) {
        elements.push({
          data: {
            id: node.id,
            label: node.label,
            nodeType: node.node_type,
            ip: node.ip,
            ports: node.ports,
            os: node.os,
            status: node.status,
            portCount: node.ports.length,
          },
        });
      }

      for (const edge of data.edges) {
        elements.push({
          data: {
            source: edge.source,
            target: edge.target,
            label: edge.label || "",
          },
        });
      }

      const cy = cytoscape({
        container: containerRef.current,
        elements,
        style: [
          {
            selector: "node[nodeType='network']",
            style: {
              "background-color": "#3b82f6",
              label: "data(label)",
              color: "#94a3b8",
              "font-size": "10px",
              "text-valign": "bottom",
              "text-margin-y": 8,
              shape: "diamond",
              width: 40,
              height: 40,
              "border-width": 2,
              "border-color": "#60a5fa",
            },
          },
          {
            selector: "node[nodeType='host']",
            style: {
              "background-color": "#10b981",
              label: "data(label)",
              color: "#d1d5db",
              "font-size": "9px",
              "text-valign": "bottom",
              "text-margin-y": 6,
              shape: "round-rectangle",
              width: "mapData(portCount, 0, 20, 30, 60)",
              height: "mapData(portCount, 0, 20, 30, 60)",
              "border-width": 2,
              "border-color": "#34d399",
            },
          },
          {
            selector: "node[status='down']",
            style: {
              "background-color": "#6b7280",
              "border-color": "#9ca3af",
              opacity: 0.6,
            },
          },
          {
            selector: "edge",
            style: {
              width: 1.5,
              "line-color": "#475569",
              "target-arrow-color": "#475569",
              "target-arrow-shape": "triangle",
              "curve-style": "bezier",
              opacity: 0.7,
            },
          },
          {
            selector: ":selected",
            style: {
              "border-color": "#f59e0b",
              "border-width": 3,
            },
          },
        ],
        layout: {
          name: "cose",
          animate: true,
          animationDuration: 500,
          nodeRepulsion: () => 8000,
          idealEdgeLength: () => 120,
          gravity: 0.3,
          padding: 40,
        },
        minZoom: 0.3,
        maxZoom: 3,
      });

      cy.on("tap", "node", (evt) => {
        const nodeData = evt.target.data();
        if (nodeData.nodeType === "host") {
          setSelectedNode({
            id: nodeData.id,
            label: nodeData.label,
            node_type: nodeData.nodeType,
            ip: nodeData.ip,
            ports: nodeData.ports || [],
            os: nodeData.os,
            status: nodeData.status,
          });
        }
      });

      cy.on("tap", (evt) => {
        if (evt.target === cy) {
          setSelectedNode(null);
        }
      });

      cyRef.current = cy;
    },
    []
  );

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
        <div className="flex-1" />
        <button
          className="flex items-center gap-1 px-2 py-1 text-xs rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => setShowPaste(true)}
        >
          <Upload className="w-3.5 h-3.5" />
          {t("topology.import", "Import Scan")}
        </button>
        {cyRef.current && (
          <button
            className="flex items-center gap-1 px-2 py-1 text-xs rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => cyRef.current?.layout({ name: "cose", animate: true, animationDuration: 500 } as cytoscape.LayoutOptions).run()}
          >
            <RefreshCw className="w-3.5 h-3.5" />
            {t("topology.relayout", "Re-layout")}
          </button>
        )}
      </div>

      <div className="flex flex-1 min-h-0 overflow-hidden">
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
            {savedMaps.map((name) => (
              <div
                key={name}
                className="group flex items-center gap-1 px-2 py-1.5 rounded-md hover:bg-muted/50 cursor-pointer text-xs"
                onClick={() => handleLoad(name)}
              >
                <FolderOpen className="w-3 h-3 text-muted-foreground flex-shrink-0" />
                <span className="truncate flex-1">{name}</span>
                <button
                  className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-opacity"
                  onClick={(e) => {
                    e.stopPropagation();
                    handleDelete(name);
                  }}
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            ))}
          </div>

          {/* Save controls */}
          {cyRef.current && (
            <div className="px-2 py-2 border-t border-border/30">
              <div className="flex gap-1">
                <input
                  className="flex-1 text-xs px-2 py-1 rounded bg-background border border-border/50 focus:border-accent outline-none"
                  placeholder={t("topology.mapName", "Map name...")}
                  value={currentName}
                  onChange={(e) => setCurrentName(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && handleSave()}
                />
                <button
                  className="p-1 rounded hover:bg-accent/20 text-muted-foreground hover:text-accent transition-colors"
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
            style={{ background: "var(--background)" }}
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

          {/* Node details panel */}
          {selectedNode && (
            <div className="absolute top-2 right-2 w-64 bg-card/95 backdrop-blur-sm rounded-lg border border-border/50 shadow-lg overflow-hidden">
              <div className="flex items-center gap-2 px-3 py-2 border-b border-border/30">
                <span className="text-xs font-medium truncate flex-1">
                  {selectedNode.label}
                </span>
                <button
                  className="p-0.5 rounded hover:bg-muted/50"
                  onClick={() => setSelectedNode(null)}
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
              <div className="px-3 py-2 space-y-1.5 max-h-60 overflow-y-auto">
                {selectedNode.ip && (
                  <div className="text-[10px]">
                    <span className="text-muted-foreground">IP: </span>
                    <span className="font-mono">{selectedNode.ip}</span>
                  </div>
                )}
                {selectedNode.os && (
                  <div className="text-[10px]">
                    <span className="text-muted-foreground">OS: </span>
                    <span>{selectedNode.os}</span>
                  </div>
                )}
                {selectedNode.ports.length > 0 && (
                  <div>
                    <div className="text-[10px] text-muted-foreground mb-1">
                      Ports ({selectedNode.ports.length}):
                    </div>
                    {selectedNode.ports.map((p) => (
                      <div
                        key={`${p.port}-${p.protocol}`}
                        className="flex items-center gap-2 text-[10px] font-mono py-0.5"
                      >
                        <span
                          className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                            p.state === "open"
                              ? "bg-green-400"
                              : p.state === "filtered"
                              ? "bg-yellow-400"
                              : "bg-red-400"
                          }`}
                        />
                        <span className="w-12">{p.port}/{p.protocol}</span>
                        <span className="text-muted-foreground">{p.service}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
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
    </div>
  );
}
