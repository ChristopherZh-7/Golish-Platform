import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import cytoscape from "cytoscape";
import { getRootDomain } from "@/lib/domain";
import { type Target } from "@/lib/pentest/types";

// ── Graph element construction ──

export function buildGraphElements(targets: Target[]): cytoscape.ElementDefinition[] {
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

// ── SVG icons for graph node types ──

export const TYPE_ICON_SVG: Record<string, string> = {
  "domain-group": `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#60a5fa" stroke-width="1.2"/><ellipse cx="20" cy="20" rx="14" ry="5" fill="none" stroke="#60a5fa" stroke-width="0.6" opacity="0.5"/><ellipse cx="20" cy="20" rx="5" ry="14" fill="none" stroke="#60a5fa" stroke-width="0.6" opacity="0.5"/><circle cx="20" cy="20" r="2" fill="#60a5fa" opacity="0.7"/></svg>`,
  domain: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#38bdf8" stroke-width="1.2"/><ellipse cx="20" cy="20" rx="14" ry="5" fill="none" stroke="#38bdf8" stroke-width="0.6" opacity="0.5"/><ellipse cx="20" cy="20" rx="5" ry="14" fill="none" stroke="#38bdf8" stroke-width="0.6" opacity="0.5"/><line x1="6" y1="20" x2="34" y2="20" stroke="#38bdf8" stroke-width="0.5" opacity="0.3"/><circle cx="20" cy="20" r="2" fill="#38bdf8" opacity="0.7"/></svg>`,
  ip: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect x="6" y="8" width="28" height="24" rx="3" fill="#0f172a" stroke="#4ade80" stroke-width="1.2"/><rect x="10" y="12" width="20" height="4" rx="1" fill="#1e293b" stroke="#4ade80" stroke-width="0.5" opacity="0.6"/><circle cx="28" cy="14" r="1.5" fill="#4ade80" opacity="0.8"/><rect x="10" y="20" width="20" height="4" rx="1" fill="#1e293b" stroke="#4ade80" stroke-width="0.5" opacity="0.6"/><circle cx="28" cy="22" r="1.5" fill="#4ade80" opacity="0.8"/></svg>`,
  cidr: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="none" stroke="#facc15" stroke-width="1.2" stroke-dasharray="4 2"/><circle cx="20" cy="20" r="8" fill="#0f172a" stroke="#facc15" stroke-width="0.8"/><circle cx="20" cy="20" r="2" fill="#facc15" opacity="0.7"/></svg>`,
  url: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#c084fc" stroke-width="1.2"/><path d="M14 20 L20 14 L26 20" fill="none" stroke="#c084fc" stroke-width="1.2" stroke-linecap="round"/><line x1="20" y1="14" x2="20" y2="28" stroke="#c084fc" stroke-width="1.2" stroke-linecap="round"/></svg>`,
  wildcard: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><circle cx="20" cy="20" r="14" fill="#0f172a" stroke="#fb923c" stroke-width="1.2"/><text x="20" y="26" text-anchor="middle" fill="#fb923c" font-size="18" font-weight="bold">*</text></svg>`,
};

export function svgUri(svg: string) {
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

// ── Cytoscape style definitions ──

function getCytoscapeStyles() {
  const typeStyles = Object.entries(TYPE_ICON_SVG).map(([type, svg]) => ({
    selector: `node[nodeType='${type}']`,
    style: {
      "background-image": svgUri(svg),
    } as cytoscape.Css.Node,
  }));

  return [
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
      } as unknown as cytoscape.Css.Node,
    },
    ...typeStyles,
    {
      selector: "node[nodeType='domain-group']",
      style: { width: 38, height: 38 } as cytoscape.Css.Node,
    },
    {
      selector: "node[scope='out']",
      style: { opacity: 0.35, color: "#475569" } as cytoscape.Css.Node,
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
      } as cytoscape.Css.Edge,
    },
    {
      selector: "node:active",
      style: { "overlay-color": "#6366f1", "overlay-opacity": 0.12 } as cytoscape.Css.Node,
    },
    {
      selector: ":selected",
      style: { "overlay-color": "#f59e0b", "overlay-opacity": 0.2 } as cytoscape.Css.Node,
    },
  ];
}

// ── Main hook ──

export function useGraphLayout(
  containerRef: React.RefObject<HTMLDivElement | null>,
  targets: Target[],
) {
  const cyRef = useRef<cytoscape.Core | null>(null);
  const retryRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [selectedTarget, setSelectedTarget] = useState<Target | null>(null);

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

    const cy = cytoscape({
      container,
      elements,
      style: getCytoscapeStyles(),
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
  }, [elements, targets, containerRef]);

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
  }, [elements, renderCytoscape, containerRef]);

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

  return {
    cyRef,
    elements,
    selectedTarget,
    setSelectedTarget,
    focusNode,
  };
}
