import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import cytoscape from "cytoscape";
import {
  Bug,
  ChevronRight,
  Crosshair,
  Globe,
  Hash,
  Network,
  Search,
  Server,
  Shield,
  Wifi,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";

interface PortInfo {
  port: number;
  protocol?: string;
  service?: string;
  state?: string;
  http_title?: string;
  http_status?: number;
  webserver?: string;
  technologies?: string[];
}

interface Target {
  id: string;
  name: string;
  type: "domain" | "ip" | "cidr" | "url" | "wildcard";
  value: string;
  tags: string[];
  notes: string;
  scope: "in" | "out";
  status: string;
  source: string;
  parent_id: string | null;
  ports: PortInfo[];
  technologies: string[];
  real_ip: string;
  cdn_waf: string;
  http_title: string;
  http_status: number | null;
  webserver: string;
  os_info: string;
  content_type: string;
  created_at: number;
  updated_at: number;
}

const MULTI_PART_TLDS = new Set([
  "co.uk", "co.jp", "co.kr", "co.nz", "co.za", "co.in", "co.id",
  "com.au", "com.br", "com.cn", "com.hk", "com.mx", "com.sg", "com.tw",
  "net.au", "net.cn", "org.au", "org.uk", "org.cn",
  "ac.uk", "gov.uk", "gov.cn", "edu.cn", "edu.au",
]);

function getRootDomain(value: string): string {
  let host: string;
  try {
    const u = new URL(value.includes("://") ? value : `https://${value}`);
    host = u.hostname;
  } catch {
    host = value.replace(/\/.*$/, "");
  }
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.includes(":")) return host;
  const parts = host.split(".");
  if (parts.length <= 2) return host;
  const last2 = parts.slice(-2).join(".");
  if (MULTI_PART_TLDS.has(last2) && parts.length > 2) {
    return parts.slice(-3).join(".");
  }
  return last2;
}

const FINDING_SEV_BADGE: Record<string, string> = {
  critical: "bg-red-500/15 text-red-400 border-red-500/25",
  high: "bg-orange-500/15 text-orange-400 border-orange-500/25",
  medium: "bg-yellow-500/15 text-yellow-400 border-yellow-500/25",
  low: "bg-blue-500/15 text-blue-400 border-blue-500/25",
  info: "bg-slate-500/15 text-slate-400 border-slate-500/25",
};

const FINDING_SEV_LABEL: Record<string, string> = {
  critical: "Crit", high: "High", medium: "Med", low: "Low", info: "Info",
};

function buildGraphElements(targets: Target[]): cytoscape.ElementDefinition[] {
  const elements: cytoscape.ElementDefinition[] = [];
  const rootTargets = targets.filter((t) => !t.parent_id);
  const childMap = new Map<string, Target[]>();
  for (const t of targets) {
    if (t.parent_id) {
      const arr = childMap.get(t.parent_id) || [];
      arr.push(t);
      childMap.set(t.parent_id, arr);
    }
  }

  const domainGroups = new Map<string, Target[]>();
  for (const t of rootTargets) {
    const domain = getRootDomain(t.value);
    const arr = domainGroups.get(domain) || [];
    arr.push(t);
    domainGroups.set(domain, arr);
  }

  const GAP_X = 200;
  const GAP_Y = 100;
  const CHILD_GAP_X = 160;

  let groupIdx = 0;
  for (const [domain, groupTargets] of domainGroups) {
    const needsGroupNode = groupTargets.length > 1;
    const groupX = groupIdx * (GAP_X + CHILD_GAP_X);

    if (needsGroupNode) {
      elements.push({
        data: {
          id: `group:${domain}`,
          label: domain,
          nodeType: "domain-group",
          targetId: null,
          scope: groupTargets.some((t) => t.scope === "in") ? "in" : "out",
          portCount: 0,
        },
        position: { x: groupX, y: 0 },
      });
    }

    groupTargets.forEach((target, tIdx) => {
      const y = needsGroupNode ? (tIdx - (groupTargets.length - 1) / 2) * GAP_Y : 0;
      const x = needsGroupNode ? groupX + CHILD_GAP_X : groupX;

      elements.push({
        data: {
          id: `target:${target.id}`,
          label: target.value,
          nodeType: target.type,
          targetId: target.id,
          scope: target.scope,
          portCount: target.ports?.length ?? 0,
          status: target.status,
          httpStatus: target.http_status,
        },
        position: { x, y },
      });

      if (needsGroupNode) {
        elements.push({
          data: {
            source: `group:${domain}`,
            target: `target:${target.id}`,
          },
        });
      }

      const children = childMap.get(target.id) || [];
      children.forEach((child, cIdx) => {
        const cy = y + (cIdx - (children.length - 1) / 2) * (GAP_Y * 0.7);
        const cx = x + CHILD_GAP_X;

        elements.push({
          data: {
            id: `target:${child.id}`,
            label: child.value,
            nodeType: child.type,
            targetId: child.id,
            scope: child.scope,
            portCount: child.ports?.length ?? 0,
            status: child.status,
            httpStatus: child.http_status,
          },
          position: { x: cx, y: cy },
        });

        elements.push({
          data: {
            source: `target:${target.id}`,
            target: `target:${child.id}`,
          },
        });
      });
    });

    groupIdx++;
  }

  return elements;
}

const TYPE_ICON_SVG: Record<string, string> = {
  "domain-group": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#60a5fa" stroke-width="1.2"/><ellipse cx="20" cy="20" rx="14" ry="5" fill="none" stroke="#60a5fa" stroke-width="0.6" opacity="0.5"/><ellipse cx="20" cy="20" rx="5" ry="14" fill="none" stroke="#60a5fa" stroke-width="0.6" opacity="0.5"/><circle cx="20" cy="20" r="2" fill="#60a5fa" opacity="0.7"/></svg>`,
  domain: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#38bdf8" stroke-width="1.2"/><ellipse cx="20" cy="20" rx="14" ry="5" fill="none" stroke="#38bdf8" stroke-width="0.6" opacity="0.5"/><ellipse cx="20" cy="20" rx="5" ry="14" fill="none" stroke="#38bdf8" stroke-width="0.6" opacity="0.5"/><line x1="6" y1="20" x2="34" y2="20" stroke="#38bdf8" stroke-width="0.5" opacity="0.3"/><circle cx="20" cy="20" r="2" fill="#38bdf8" opacity="0.7"/></svg>`,
  ip: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect x="6" y="8" width="28" height="24" rx="3" fill="#0f172a" stroke="#4ade80" stroke-width="1.2"/><rect x="10" y="12" width="20" height="4" rx="1" fill="#1e293b" stroke="#4ade80" stroke-width="0.5" opacity="0.6"/><circle cx="28" cy="14" r="1.5" fill="#4ade80" opacity="0.8"/><rect x="10" y="20" width="20" height="4" rx="1" fill="#1e293b" stroke="#4ade80" stroke-width="0.5" opacity="0.6"/><circle cx="28" cy="22" r="1.5" fill="#4ade80" opacity="0.8"/></svg>`,
  cidr: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="none" stroke="#facc15" stroke-width="1.2" stroke-dasharray="4 2"/><circle cx="20" cy="20" r="8" fill="#0f172a" stroke="#facc15" stroke-width="0.8"/><circle cx="20" cy="20" r="2" fill="#facc15" opacity="0.7"/></svg>`,
  url: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#c084fc" stroke-width="1.2"/><path d="M14 20 L20 14 L26 20" fill="none" stroke="#c084fc" stroke-width="1.2" stroke-linecap="round"/><line x1="20" y1="14" x2="20" y2="28" stroke="#c084fc" stroke-width="1.2" stroke-linecap="round"/></svg>`,
  wildcard: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#fb923c" stroke-width="1.2"/><text x="20" y="26" text-anchor="middle" fill="#fb923c" font-size="18" font-weight="bold">*</text></svg>`,
};

function svgUri(svg: string) {
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

export function TargetGraphView({ targets }: { targets: Target[] }) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<cytoscape.Core | null>(null);
  const retryRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [selectedTarget, setSelectedTarget] = useState<Target | null>(null);
  const [nodeFindings, setNodeFindings] = useState<{ id: string; title: string; severity: string; status: string }[]>([]);

  useEffect(() => {
    if (!selectedTarget) { setNodeFindings([]); return; }
    const host = selectedTarget.real_ip || selectedTarget.value;
    invoke<{ id: string; title: string; severity: string; status: string }[]>("findings_for_host", {
      host,
      projectPath: getProjectPath(),
    }).then(setNodeFindings).catch(() => setNodeFindings([]));
  }, [selectedTarget]);

  const elements = useMemo(() => buildGraphElements(targets), [targets]);

  const renderCytoscape = useCallback((retries = 0) => {
    const container = containerRef.current;
    if (!container || elements.length === 0) return;
    const parent = container.parentElement;
    if (!parent) return;
    const parentRect = parent.getBoundingClientRect();
    if (parentRect.width <= 0 || parentRect.height <= 0) {
      if (retries < 20) {
        retryRef.current = setTimeout(() => renderCytoscape(retries + 1), 150);
      }
      return;
    }

    if (cyRef.current) cyRef.current.destroy();

    container.style.width = `${parentRect.width}px`;
    container.style.height = `${parentRect.height}px`;

    const typeStyles = Object.entries(TYPE_ICON_SVG).map(([type, svg]) => ({
      selector: `node[nodeType='${type}']`,
      style: {
        "background-image": svgUri(svg),
      } as cytoscape.Css.Node,
    }));

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
            "font-weight": "500" as any,
            "text-valign": "bottom",
            "text-halign": "center",
            "text-margin-y": 6,
            "text-max-width": "140px",
            "text-wrap": "ellipsis",
            "min-zoomed-font-size": 8,
            "overlay-padding": "4px",
            "background-color": "transparent",
            "background-fit": "contain",
            "background-clip": "none",
            "background-width": "100%",
            "background-height": "100%",
            "border-width": 0,
            shape: "ellipse",
            width: 34,
            height: 34,
          },
        },
        ...typeStyles,
        {
          selector: "node[nodeType='domain-group']",
          style: { width: 38, height: 38 },
        },
        {
          selector: "node[scope='out']",
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
            "curve-style": "bezier",
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
      layout: { name: "preset", fit: true, padding: 48, animate: false },
      minZoom: 0.2,
      maxZoom: 5,
      textureOnViewport: true,
      hideEdgesOnViewport: true,
      hideLabelsOnViewport: true,
    });

    container.style.opacity = "0";
    requestAnimationFrame(() => {
      cy.resize();
      cy.fit(undefined, 60);
      const fitZ = cy.zoom();
      cy.zoom({
        level: Math.min(fitZ * 0.7, 1.5),
        renderedPosition: { x: container.clientWidth / 2, y: container.clientHeight / 2 },
      });
      container.style.transition = "opacity 150ms ease-in";
      container.style.opacity = "1";
    });

    cy.on("tap", "node", (evt) => {
      const node = evt.target;
      const d = node.data();

      cy.stop();
      cy.animate({
        center: { eles: node },
        zoom: Math.max(cy.zoom(), 1.2),
      }, {
        duration: 200,
        easing: "ease-out-quad",
        complete: () => {
          if (d.targetId) {
            const t = targets.find((t) => t.id === d.targetId);
            if (t) setSelectedTarget(t);
          }
        },
      });

      if (!d.targetId) setSelectedTarget(null);
    });
    cy.on("tap", (evt) => {
      if (evt.target === cy) {
        setSelectedTarget(null);
        cy.stop();
        cy.animate({
          fit: { eles: cy.elements(), padding: 60 },
        }, { duration: 200, easing: "ease-out-quad" });
      }
    });

    cyRef.current = cy;
  }, [elements, targets]);

  useEffect(() => {
    renderCytoscape();
    return () => {
      if (retryRef.current) clearTimeout(retryRef.current);
      cyRef.current?.destroy();
      cyRef.current = null;
    };
  }, [renderCytoscape]);

  useEffect(() => {
    const parent = containerRef.current?.parentElement;
    if (!parent) return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
          const container = containerRef.current;
          if (container) {
            container.style.width = `${entry.contentRect.width}px`;
            container.style.height = `${entry.contentRect.height}px`;
          }
          if (cyRef.current) {
            cyRef.current.resize();
            cyRef.current.fit(undefined, 40);
          } else if (elements.length > 0) {
            renderCytoscape();
          }
        }
      }
    });
    observer.observe(parent);
    return () => observer.disconnect();
  }, [elements, renderCytoscape]);

  const rootTargets = useMemo(() => targets.filter((t) => !t.parent_id), [targets]);
  const childMap = useMemo(() => {
    const map = new Map<string, Target[]>();
    for (const t of targets) {
      if (t.parent_id) {
        const arr = map.get(t.parent_id) || [];
        arr.push(t);
        map.set(t.parent_id, arr);
      }
    }
    return map;
  }, [targets]);

  const domainGroups = useMemo(() => {
    const map = new Map<string, Target[]>();
    for (const t of rootTargets) {
      const domain = getRootDomain(t.value);
      const arr = map.get(domain) || [];
      arr.push(t);
      map.set(domain, arr);
    }
    return [...map.entries()].sort((a, b) => b[1].length - a[1].length);
  }, [rootTargets]);

  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [searchFilter, setSearchFilter] = useState("");

  const focusNode = useCallback((targetId: string) => {
    const cy = cyRef.current;
    if (!cy) return;
    const node = cy.getElementById(`target:${targetId}`);
    if (node.length > 0) {
      cy.stop();
      cy.animate({
        center: { eles: node },
        zoom: Math.max(cy.zoom(), 1.2),
      }, {
        duration: 200,
        easing: "ease-out-quad",
        complete: () => {
          const t = targets.find((t) => t.id === targetId);
          if (t) setSelectedTarget(t);
        },
      });
    } else {
      const t = targets.find((t) => t.id === targetId);
      if (t) setSelectedTarget(t);
    }
  }, [targets]);

  const filteredGroups = useMemo(() => {
    if (!searchFilter) return domainGroups;
    const q = searchFilter.toLowerCase();
    return domainGroups
      .map(([domain, items]) => {
        const filtered = items.filter((t) =>
          t.value.toLowerCase().includes(q) ||
          t.name.toLowerCase().includes(q) ||
          childMap.get(t.id)?.some((c) => c.value.toLowerCase().includes(q))
        );
        return [domain, filtered] as [string, Target[]];
      })
      .filter(([, items]) => items.length > 0);
  }, [domainGroups, searchFilter, childMap]);

  return (
    <div className="relative h-full w-full overflow-hidden flex">
      {/* Sidebar target list */}
      <div className="w-52 border-r border-border/30 flex flex-col flex-shrink-0 bg-card/50">
        <div className="px-2 py-2 border-b border-border/20">
          <div className="relative">
            <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-muted-foreground/40" />
            <input
              className="w-full pl-7 pr-2 py-1 text-[11px] bg-background/50 border border-border/30 rounded focus:border-accent/50 outline-none"
              placeholder="Filter targets..."
              value={searchFilter}
              onChange={(e) => setSearchFilter(e.target.value)}
            />
          </div>
        </div>
        <div className="flex-1 overflow-y-auto px-1 py-1">
          {filteredGroups.map(([domain, items]) => {
            const isMulti = items.length > 1;
            const isExpanded = expandedGroups.has(domain) || !!searchFilter;

            return (
              <div key={domain}>
                {isMulti && (
                  <button
                    type="button"
                    className="flex items-center gap-1 px-2 py-1.5 w-full rounded-md hover:bg-muted/30 text-left text-[11px]"
                    onClick={() => setExpandedGroups((prev) => {
                      const next = new Set(prev);
                      if (next.has(domain)) next.delete(domain); else next.add(domain);
                      return next;
                    })}
                  >
                    <ChevronRight className={cn("w-3 h-3 text-muted-foreground/40 transition-transform", isExpanded && "rotate-90")} />
                    <Globe className="w-3 h-3 text-blue-400/70 flex-shrink-0" />
                    <span className="truncate flex-1 font-medium text-foreground/80">{domain}</span>
                    <span className="text-[9px] text-muted-foreground/40">{items.length}</span>
                  </button>
                )}

                {(isExpanded || !isMulti) && items.map((target) => {
                  const isSelected = selectedTarget?.id === target.id;
                  const children = childMap.get(target.id) || [];
                  return (
                    <div key={target.id} className={isMulti ? "pl-3" : ""}>
                      <button
                        type="button"
                        className={cn(
                          "flex items-center gap-1.5 px-2 py-1 w-full rounded-md text-left text-[10px] transition-colors",
                          isSelected ? "bg-accent/15 text-accent" : "hover:bg-muted/20 text-muted-foreground hover:text-foreground",
                          target.scope === "out" && "opacity-50",
                        )}
                        onClick={() => focusNode(target.id)}
                      >
                        <span className={cn(
                          "w-1.5 h-1.5 rounded-full flex-shrink-0",
                          target.scope === "in" ? "bg-emerald-400" : "bg-zinc-500",
                        )} />
                        <span className="truncate flex-1 font-mono">{target.value}</span>
                        {target.ports?.length > 0 && (
                          <span className="text-[9px] text-emerald-400/50">{target.ports.length}p</span>
                        )}
                      </button>
                      {children.length > 0 && (isExpanded || !isMulti) && (
                        <div className="pl-4">
                          {children.slice(0, 8).map((child) => (
                            <button
                              key={child.id}
                              type="button"
                              className={cn(
                                "flex items-center gap-1 px-1.5 py-0.5 w-full rounded text-left text-[9px] transition-colors",
                                selectedTarget?.id === child.id ? "text-accent" : "text-muted-foreground/50 hover:text-muted-foreground",
                              )}
                              onClick={() => focusNode(child.id)}
                            >
                              <span className="truncate flex-1 font-mono">{child.value}</span>
                            </button>
                          ))}
                          {children.length > 8 && (
                            <span className="px-1.5 text-[8px] text-muted-foreground/30">+{children.length - 8} more</span>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
        <div className="px-2 py-1.5 border-t border-border/20 text-[10px] text-muted-foreground/40 text-center">
          {targets.length} targets
        </div>
      </div>

      {/* Graph canvas */}
      <div className="flex-1 min-w-0 relative">
        <div
          ref={containerRef}
          className="absolute inset-0"
          style={{
            background: "var(--background)",
            backgroundImage: "radial-gradient(circle, hsl(var(--muted-foreground) / 0.08) 1px, transparent 1px)",
            backgroundSize: "24px 24px",
          }}
        />

        {targets.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="flex flex-col items-center gap-3 text-muted-foreground/60">
              <Network className="w-12 h-12" />
              <p className="text-sm">{t("targets.noTargets")}</p>
            </div>
          </div>
        )}

      {selectedTarget && (
        <aside className={cn(
          "absolute top-3 right-3 z-10 flex max-h-[min(28rem,calc(100%-1.5rem))] w-[min(19rem,calc(100%-1.5rem))] flex-col overflow-hidden rounded-xl border border-border/60",
          "bg-popover/95 shadow-xl shadow-black/10 backdrop-blur-md",
          "ring-1 ring-black/[0.04] dark:ring-white/[0.06]",
        )}>
          <div className="flex items-start gap-2.5 border-b border-border/50 bg-muted/25 px-3 py-2.5">
            <div className={cn(
              "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg",
              selectedTarget.scope === "out" ? "bg-muted text-muted-foreground" : "bg-accent/15 text-accent",
            )}>
              {selectedTarget.type === "domain" ? <Globe className="h-4 w-4" /> :
               selectedTarget.type === "ip" ? <Hash className="h-4 w-4" /> :
               selectedTarget.type === "cidr" ? <Network className="h-4 w-4" /> :
               selectedTarget.type === "wildcard" ? <Crosshair className="h-4 w-4" /> :
               <Server className="h-4 w-4" />}
            </div>
            <div className="min-w-0 flex-1 pt-0.5">
              <h3 className="truncate text-sm font-semibold leading-snug tracking-tight text-foreground font-mono">
                {selectedTarget.value}
              </h3>
              <div className="mt-1 flex flex-wrap items-center gap-1.5">
                <span className={cn(
                  "rounded-md border px-1.5 py-0.5 text-[10px] font-medium",
                  selectedTarget.scope === "in"
                    ? "border-emerald-500/20 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                    : "border-red-500/20 bg-red-500/10 text-red-600 dark:text-red-400",
                )}>
                  {selectedTarget.scope === "in" ? "In Scope" : "Out of Scope"}
                </span>
                <span className="rounded-md border border-border/50 bg-muted/50 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                  {selectedTarget.type}
                </span>
              </div>
            </div>
            <button
              type="button"
              className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted/80 hover:text-foreground"
              onClick={() => setSelectedTarget(null)}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 space-y-3">
            {(selectedTarget.real_ip || selectedTarget.os_info || selectedTarget.webserver || selectedTarget.cdn_waf) && (
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-xs">
                {selectedTarget.real_ip && (
                  <><dt className="text-muted-foreground">IP</dt><dd className="font-mono text-foreground tabular-nums">{selectedTarget.real_ip}</dd></>
                )}
                {selectedTarget.os_info && (
                  <><dt className="text-muted-foreground">OS</dt><dd className="text-foreground">{selectedTarget.os_info}</dd></>
                )}
                {selectedTarget.webserver && (
                  <><dt className="text-muted-foreground">Server</dt><dd className="text-foreground">{selectedTarget.webserver}</dd></>
                )}
                {selectedTarget.cdn_waf && (
                  <><dt className="text-muted-foreground">CDN/WAF</dt><dd className="text-foreground">{selectedTarget.cdn_waf}</dd></>
                )}
              </dl>
            )}

            {selectedTarget.ports?.length > 0 && (
              <div>
                <div className="mb-2 flex items-center justify-between gap-2">
                  <span className="text-xs font-medium text-foreground flex items-center gap-1">
                    <Wifi className="w-3 h-3 text-emerald-400" />
                    Ports
                  </span>
                  <span className="rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                    {selectedTarget.ports.length}
                  </span>
                </div>
                <ul className="space-y-1">
                  {selectedTarget.ports.map((p) => {
                    const dot =
                      p.state === "open" ? "bg-emerald-400 ring-1 ring-emerald-500/40" :
                      p.state === "filtered" ? "bg-amber-400 ring-1 ring-amber-500/40" :
                      "bg-rose-400 ring-1 ring-rose-500/40";
                    return (
                      <li key={`${p.port}-${p.protocol}`} className="flex items-center gap-2 rounded-lg border border-border/40 bg-muted/15 px-2 py-1.5 text-xs">
                        <span className={cn("h-2 w-2 shrink-0 rounded-full", dot)} />
                        <span className="w-[4.25rem] shrink-0 font-mono tabular-nums text-foreground">{p.port}/{p.protocol}</span>
                        <span className="min-w-0 truncate text-muted-foreground">{p.service || "—"}</span>
                        {p.http_status != null && (
                          <span className={cn("text-[10px] font-mono", p.http_status < 400 ? "text-green-400" : "text-red-400")}>
                            [{p.http_status}]
                          </span>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </div>
            )}

            {(() => {
              const children = targets.filter((t) => t.parent_id === selectedTarget.id);
              if (children.length === 0) return null;
              return (
                <div className="border-t border-border/40 pt-3">
                  <div className="mb-2 flex items-center justify-between text-xs font-medium text-foreground">
                    <span className="flex items-center gap-1">
                      <Network className="w-3 h-3 text-accent/60" />
                      Subdomains
                    </span>
                    <span className="rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                      {children.length}
                    </span>
                  </div>
                  <ul className="space-y-1">
                    {children.map((child) => (
                      <li
                        key={child.id}
                        className="flex items-center gap-2 rounded-lg border border-border/40 bg-muted/10 px-2 py-1.5 text-xs cursor-pointer hover:bg-muted/20 transition-colors"
                        onClick={() => {
                          const cy = cyRef.current;
                          if (cy) {
                            const node = cy.getElementById(`target:${child.id}`);
                            if (node.length > 0) {
                              cy.stop();
                              cy.animate({
                                center: { eles: node },
                                zoom: Math.max(cy.zoom(), 1.2),
                              }, {
                                duration: 200,
                                easing: "ease-out-quad",
                                complete: () => setSelectedTarget(child),
                              });
                              return;
                            }
                          }
                          setSelectedTarget(child);
                        }}
                      >
                        <Globe className="w-3 h-3 text-blue-400 flex-shrink-0" />
                        <span className="font-mono text-foreground/70 flex-1 truncate">{child.value}</span>
                        {child.ports?.length > 0 && (
                          <span className="flex items-center gap-0.5 text-[10px] text-emerald-400/60">
                            <Wifi className="w-2.5 h-2.5" />{child.ports.length}
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              );
            })()}

            {nodeFindings.length > 0 && (
              <div className="border-t border-border/40 pt-3">
                <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-foreground">
                  <Bug className="h-3.5 w-3.5 text-muted-foreground" />
                  Findings
                  <span className="ml-auto rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                    {nodeFindings.length}
                  </span>
                </div>
                <ul className="space-y-1">
                  {nodeFindings.map((f) => (
                    <li key={f.id} className="flex items-start gap-2 rounded-lg border border-border/40 bg-muted/10 px-2 py-1.5">
                      <span className={cn(
                        "mt-0.5 shrink-0 rounded border px-1 py-px text-[9px] font-semibold uppercase leading-none",
                        FINDING_SEV_BADGE[f.severity] || FINDING_SEV_BADGE.info,
                      )}>
                        {FINDING_SEV_LABEL[f.severity] || FINDING_SEV_LABEL.info}
                      </span>
                      <span className="min-w-0 flex-1 text-[11px] leading-snug text-foreground">{f.title}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            <div className="border-t border-border/40 pt-3">
              <p className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/80">Notes</p>
              <QuickNotes entityType="target" entityId={selectedTarget.id} compact />
            </div>
          </div>
        </aside>
      )}
      </div>
    </div>
  );
}
