import type { Organization } from "@/lib/api/organizations";
import type { PortInfo, Target } from "@/lib/pentest/types";
import type {
  TopologyEdge,
  TopologyMode,
  TopologyModel,
  TopologyNode,
  TopologyVisibility,
} from "./types";

const UNASSIGNED_ORG_ID = "topology:org:unassigned";

const COLUMN_X = [72, 268, 486, 704, 884];
const NODE_WIDTH = [150, 170, 172, 148, 132];
const NODE_HEIGHT = [62, 62, 58, 52, 52];
const ROW_GAP = 84;
const CANVAS_MIN_HEIGHT = 520;
const CANVAS_WIDTH = 1080;

interface BuildOptions {
  mode: TopologyMode;
  visibility: TopologyVisibility;
  query?: string;
}

interface OrgBucket {
  id: string;
  label: string;
  organization?: Organization;
  parentId: string | null;
  targets: Target[];
}

export function buildTopologyModel(
  organizations: Organization[],
  targets: Target[],
  options: BuildOptions
): TopologyModel {
  const q = options.query?.trim().toLowerCase() ?? "";
  const orgBuckets = buildOrgBuckets(organizations, targets);
  const childrenByParent = buildChildrenByParent(orgBuckets);
  const nodes: TopologyNode[] = [];
  const edges: TopologyEdge[] = [];
  const visibleNodeIds = new Set<string>();
  const renderedOrgIds = new Set<string>();
  let nextRow = 0;

  const addNode = (node: Omit<TopologyNode, "x" | "y" | "width" | "height">, rowIndex: number) => {
    if (!options.visibility[node.kind]) return null;
    if (q && !nodeMatchesQuery(node, q)) return null;
    const placed: TopologyNode = {
      ...node,
      x: COLUMN_X[node.column],
      y: 86 + rowIndex * ROW_GAP,
      width: NODE_WIDTH[node.column],
      height: NODE_HEIGHT[node.column],
    };
    nodes.push(placed);
    visibleNodeIds.add(placed.id);
    return placed;
  };

  const renderTarget = (target: Target): { start: number; end: number } => {
    const targetColumn = targetNodeColumn();
    const ports = target.ports ?? [];
    const evidenceCount = estimateEvidenceCount(target);
    const targetNodeId = `target:${target.id}`;
    const start = nextRow;
    addNode(
      {
        id: targetNodeId,
        kind: "target",
        label: target.value,
        subtitle: target.source || target.type,
        column: targetColumn,
        scope: target.scope,
        target,
        metrics: {
          ports: ports.length,
          evidence: evidenceCount,
        },
      },
      start
    );
    nextRow++;

    if (options.mode !== "ownership") {
      for (const [index, port] of ports.slice(0, 4).entries()) {
        const serviceRow = index === 0 ? start : nextRow++;
        const serviceNodeId = `service:${target.id}:${port.port}:${port.protocol ?? "tcp"}`;
        addNode(
          {
            id: serviceNodeId,
            kind: "service",
            label: formatPort(port),
            subtitle: serviceSubtitle(port),
            column: serviceColumnForTargetColumn(targetColumn),
            scope: target.scope,
            target,
            port,
          },
          serviceRow
        );
        edges.push({
          id: `edge:${targetNodeId}:${serviceNodeId}`,
          source: targetNodeId,
          target: serviceNodeId,
          kind: "exposes",
          label: "exposes",
        });
      }
    }

    if (options.mode === "evidence") {
      const evidenceNodeId = `evidence:${target.id}`;
      addNode(
        {
          id: evidenceNodeId,
          kind: "evidence",
          label: evidenceLabel(target, evidenceCount),
          subtitle: evidenceSubtitle(target),
          column: evidenceColumnForTargetColumn(targetColumn),
          scope: target.scope,
          target,
          metrics: {
            evidence: evidenceCount,
            findings: target.status === "tested" ? 1 : 0,
          },
        },
        start
      );
      edges.push({
        id: `edge:${targetNodeId}:${evidenceNodeId}`,
        source: targetNodeId,
        target: evidenceNodeId,
        kind: "produced",
        label: "evidence",
      });
    }

    return { start, end: Math.max(start, nextRow - 1) };
  };

  const renderOrg = (org: OrgBucket, depth: number): { start: number; end: number } | null => {
    if (renderedOrgIds.has(org.id)) return null;
    renderedOrgIds.add(org.id);
    const startBefore = nextRow;
    const childLayouts: Array<{ start: number; end: number }> = [];

    for (const child of orderOrgBuckets(childrenByParent.get(org.id) ?? [])) {
      const childLayout = renderOrg(child, depth + 1);
      if (childLayout) {
        childLayouts.push(childLayout);
        edges.push({
          id: `edge:${org.id}:${child.id}`,
          source: org.id,
          target: child.id,
          kind: "owns",
          label: "owns",
        });
      }
    }

    for (const target of sortTargets(org.targets)) {
      const targetLayout = renderTarget(target);
      childLayouts.push(targetLayout);
      edges.push({
        id: `edge:${org.id}:target:${target.id}`,
        source: org.id,
        target: `target:${target.id}`,
        kind: "contains",
        label: "contains",
      });
    }

    if (childLayouts.length === 0) {
      childLayouts.push({ start: nextRow, end: nextRow });
      nextRow++;
    }

    const start = Math.min(...childLayouts.map((layout) => layout.start));
    const end = Math.max(...childLayouts.map((layout) => layout.end));
    const center = (start + end) / 2;
    const orgTargetStats = countTargetsForOrg(org.id, orgBuckets);

    addNode(
      {
        id: org.id,
        kind: "organization",
        label: org.label,
        subtitle: org.parentId
          ? "subsidiary"
          : org.id === UNASSIGNED_ORG_ID
            ? "unassigned"
            : "root org",
        column: orgColumn(depth),
        organization: org.organization,
        metrics: {
          targets: orgTargetStats.total,
          inScopeTargets: orgTargetStats.inScope,
        },
      },
      center
    );

    return { start: Math.min(startBefore, start), end };
  };

  for (const root of orderOrgBuckets(childrenByParent.get(null) ?? [])) {
    renderOrg(root, 0);
  }

  const filteredEdges = edges.filter(
    (edge) => visibleNodeIds.has(edge.source) && visibleNodeIds.has(edge.target)
  );
  const orderedNodes = [...nodes].sort((a, b) => a.column - b.column || a.y - b.y);

  return {
    nodes: orderedNodes,
    edges: filteredEdges,
    stats: {
      organizations: organizations.length,
      targets: targets.length,
      services: targets.reduce((sum, target) => sum + (target.ports?.length ?? 0), 0),
      evidence: targets.reduce((sum, target) => sum + estimateEvidenceCount(target), 0),
    },
    bounds: {
      width: CANVAS_WIDTH,
      height: Math.max(CANVAS_MIN_HEIGHT, 120 + Math.max(0, nextRow - 1) * ROW_GAP),
    },
  };
}

/**
 * Lineage of a node = its ancestor chain (towards the root org) plus its entire
 * descendant subtree. Siblings and cousins are intentionally excluded so that
 * "focus this unit" only keeps the direct vertical relationship.
 */
export function collectLineageIds(edges: TopologyEdge[], nodeId: string): Set<string> {
  const ids = new Set<string>([nodeId]);
  walkLineage(edges, nodeId, ids, "up");
  walkLineage(edges, nodeId, ids, "down");
  return ids;
}

function walkLineage(
  edges: TopologyEdge[],
  start: string,
  ids: Set<string>,
  direction: "up" | "down"
) {
  let frontier = [start];
  while (frontier.length > 0) {
    const next: string[] = [];
    for (const current of frontier) {
      for (const edge of edges) {
        const from = direction === "up" ? edge.target : edge.source;
        const to = direction === "up" ? edge.source : edge.target;
        if (from === current && !ids.has(to)) {
          ids.add(to);
          next.push(to);
        }
      }
    }
    frontier = next;
  }
}

/**
 * Hard isolate: keep only the focus node's lineage, drop every other branch, and
 * vertically compact the survivors so the isolated path fills the canvas. Returns
 * the model unchanged when there is no focus or the focus node no longer exists.
 */
export function applyTopologyFocus(
  model: TopologyModel,
  focusNodeId: string | null
): TopologyModel {
  if (!focusNodeId) return model;
  if (!model.nodes.some((node) => node.id === focusNodeId)) return model;

  const lineage = collectLineageIds(model.edges, focusNodeId);
  const focusedNodes = model.nodes.filter((node) => lineage.has(node.id));
  const focusedEdges = model.edges.filter(
    (edge) => lineage.has(edge.source) && lineage.has(edge.target)
  );

  const distinctY = [...new Set(focusedNodes.map((node) => node.y))].sort((a, b) => a - b);
  const rowByY = new Map(distinctY.map((y, index) => [y, index]));
  const compactNodes = focusedNodes.map((node) => ({
    ...node,
    y: 86 + (rowByY.get(node.y) ?? 0) * ROW_GAP,
  }));

  return {
    nodes: compactNodes,
    edges: focusedEdges,
    stats: model.stats,
    bounds: {
      width: model.bounds.width,
      height: Math.max(CANVAS_MIN_HEIGHT, 120 + Math.max(0, distinctY.length - 1) * ROW_GAP),
    },
  };
}

function buildOrgBuckets(organizations: Organization[], targets: Target[]): OrgBucket[] {
  const buckets = new Map<string, OrgBucket>();
  for (const org of organizations) {
    buckets.set(org.id, {
      id: org.id,
      label: org.name,
      organization: org,
      parentId: org.parent_id,
      targets: [],
    });
  }

  const unassigned: OrgBucket = {
    id: UNASSIGNED_ORG_ID,
    label: "Unassigned",
    parentId: null,
    targets: [],
  };

  for (const target of targets) {
    const bucket = target.organization_id ? buckets.get(target.organization_id) : null;
    if (bucket) {
      bucket.targets.push(target);
    } else {
      unassigned.targets.push(target);
    }
  }

  if (unassigned.targets.length > 0) buckets.set(unassigned.id, unassigned);
  return [...buckets.values()];
}

function buildChildrenByParent(buckets: OrgBucket[]) {
  const ids = new Set(buckets.map((bucket) => bucket.id));
  const childrenByParent = new Map<string | null, OrgBucket[]>();
  for (const bucket of buckets) {
    const parentId = bucket.parentId && ids.has(bucket.parentId) ? bucket.parentId : null;
    const children = childrenByParent.get(parentId) ?? [];
    children.push(bucket);
    childrenByParent.set(parentId, children);
  }
  return childrenByParent;
}

function orderOrgBuckets(buckets: OrgBucket[]): OrgBucket[] {
  return [...buckets].sort((a, b) => {
    if (a.id === UNASSIGNED_ORG_ID) return 1;
    if (b.id === UNASSIGNED_ORG_ID) return -1;
    const ao = a.organization?.sort_order ?? 0;
    const bo = b.organization?.sort_order ?? 0;
    return ao - bo || a.label.localeCompare(b.label, "zh");
  });
}

function countTargetsForOrg(
  orgId: string,
  buckets: OrgBucket[]
): { total: number; inScope: number } {
  const byParent = new Map<string | null, OrgBucket[]>();
  for (const bucket of buckets) {
    const list = byParent.get(bucket.parentId) ?? [];
    list.push(bucket);
    byParent.set(bucket.parentId, list);
  }

  let total = 0;
  let inScope = 0;
  const visit = (id: string) => {
    const bucket = buckets.find((item) => item.id === id);
    if (!bucket) return;
    total += bucket.targets.length;
    inScope += bucket.targets.filter((target) => target.scope === "in").length;
    for (const child of byParent.get(id) ?? []) visit(child.id);
  };
  visit(orgId);
  return { total, inScope };
}

function sortTargets(targets: Target[]) {
  return [...targets].sort((a, b) => {
    if (a.scope !== b.scope) return a.scope === "in" ? -1 : 1;
    return a.value.localeCompare(b.value);
  });
}

function orgColumn(depth: number) {
  return Math.min(depth, 1);
}

function targetNodeColumn() {
  return 2;
}

function serviceColumnForTargetColumn(targetColumn: number) {
  return Math.min(targetColumn + 1, COLUMN_X.length - 1);
}

function evidenceColumnForTargetColumn(targetColumn: number) {
  return Math.min(targetColumn + 2, COLUMN_X.length - 1);
}

function nodeMatchesQuery(node: Omit<TopologyNode, "x" | "y" | "width" | "height">, query: string) {
  return (
    node.label.toLowerCase().includes(query) ||
    node.subtitle.toLowerCase().includes(query) ||
    node.target?.value.toLowerCase().includes(query) ||
    node.organization?.name.toLowerCase().includes(query)
  );
}

function formatPort(port: PortInfo) {
  return `${port.port}/${port.protocol || "tcp"}`;
}

function serviceSubtitle(port: PortInfo) {
  const status = port.http_status ? ` · ${port.http_status}` : "";
  return `${port.service || port.webserver || "service"}${status}`;
}

function estimateEvidenceCount(target: Target) {
  let count = 0;
  if (target.source) count++;
  if (target.real_ip) count++;
  if (target.http_status != null) count++;
  if (target.cdn_waf) count++;
  if (target.ports?.length) count += target.ports.length;
  if (target.technologies?.length) count += target.technologies.length;
  return count;
}

function evidenceLabel(target: Target, count: number) {
  if (target.status === "tested") return "tested";
  if (target.status === "recondone") return "recon done";
  if (target.status === "recon") return "recon";
  return count > 0 ? `${count} evidence` : "no evidence";
}

function evidenceSubtitle(target: Target) {
  if (target.updated_at) return `updated ${formatRelative(target.updated_at)}`;
  return target.source || "local ledger";
}

function formatRelative(ms: number) {
  const delta = Date.now() - ms;
  if (!Number.isFinite(delta) || delta < 0) return "recently";
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 60) return `${Math.max(1, minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}
