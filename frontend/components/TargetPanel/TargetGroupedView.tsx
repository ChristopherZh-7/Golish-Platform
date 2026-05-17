import {
  Building2,
  ChevronDown,
  Crosshair,
  FolderOpen,
  Globe,
  Hash,
  Network,
  Shield,
  ShieldOff,
  Trash2,
  Wifi,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { Target, TargetStatus } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import { TargetDetailView } from "./TargetDetail";

const TYPE_ICONS: Record<string, React.ReactNode> = {
  domain: <Globe className="w-3.5 h-3.5 text-blue-400" />,
  ip: <Hash className="w-3.5 h-3.5 text-green-400" />,
  cidr: <Network className="w-3.5 h-3.5 text-yellow-400" />,
  url: <Globe className="w-3.5 h-3.5 text-purple-400" />,
  wildcard: <Crosshair className="w-3.5 h-3.5 text-orange-400" />,
};

const STATUS_CONFIG: Record<TargetStatus, { label: string; color: string; bg: string }> = {
  new: { label: "New", color: "text-gray-400", bg: "bg-gray-500/10" },
  recon: { label: "Recon", color: "text-blue-400", bg: "bg-blue-500/10" },
  recondone: { label: "Recon Done", color: "text-cyan-400", bg: "bg-cyan-500/10" },
  scanning: { label: "Scanning", color: "text-yellow-400", bg: "bg-yellow-500/10" },
  tested: { label: "Tested", color: "text-green-400", bg: "bg-green-500/10" },
};

const UNASSIGNED_KEY = "__unassigned__";

interface TreeNode {
  name: string;
  path: string;
  children: Map<string, TreeNode>;
  targets: Target[];
}

function makeNode(name: string, path: string): TreeNode {
  return { name, path, children: new Map(), targets: [] };
}

function buildTree(targets: Target[], unassignedLabel: string): TreeNode[] {
  const root = new Map<string, TreeNode>();

  for (const t of targets) {
    const raw = (t.grp ?? "").trim();
    const isUnassigned = !raw || raw === "default";

    if (isUnassigned) {
      let node = root.get(UNASSIGNED_KEY);
      if (!node) {
        node = makeNode(unassignedLabel, UNASSIGNED_KEY);
        root.set(UNASSIGNED_KEY, node);
      }
      node.targets.push(t);
      continue;
    }

    const segments = raw
      .split("/")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    if (segments.length === 0) {
      continue;
    }

    let cursor: Map<string, TreeNode> = root;
    let pathAcc = "";
    let node: TreeNode | undefined;
    for (let i = 0; i < segments.length; i += 1) {
      const seg = segments[i];
      pathAcc = pathAcc ? `${pathAcc}/${seg}` : seg;
      node = cursor.get(seg);
      if (!node) {
        node = makeNode(seg, pathAcc);
        cursor.set(seg, node);
      }
      cursor = node.children;
    }
    if (node) {
      node.targets.push(t);
    }
  }

  const sortNodes = (nodes: TreeNode[]): TreeNode[] => {
    nodes.sort((a, b) => {
      if (a.path === UNASSIGNED_KEY) return 1;
      if (b.path === UNASSIGNED_KEY) return -1;
      return a.name.localeCompare(b.name, "zh");
    });
    for (const n of nodes) {
      const arr = sortNodes([...n.children.values()]);
      n.children = new Map(arr.map((c) => [c.name, c]));
    }
    return nodes;
  };

  return sortNodes([...root.values()]);
}

function countAllTargets(node: TreeNode): { total: number; inScope: number } {
  let total = node.targets.length;
  let inScope = node.targets.filter((t) => t.scope === "in").length;
  for (const child of node.children.values()) {
    const sub = countAllTargets(child);
    total += sub.total;
    inScope += sub.inScope;
  }
  return { total, inScope };
}

interface TargetGroupedViewProps {
  targets: Target[];
  t: (key: string) => string;
  onDelete: (id: string) => Promise<void>;
  onToggleScope: (target: Target) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
  onScan: (target: Target) => void;
}

export function TargetGroupedView({
  targets,
  t,
  onDelete,
  onToggleScope,
  onUpdateNotes,
  onScan,
}: TargetGroupedViewProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [editingId, setEditingId] = useState<string | null>(null);

  const unassignedLabel = t("targets.unassigned");
  const roots = useMemo(() => buildTree(targets, unassignedLabel), [targets, unassignedLabel]);

  const toggleCollapse = useCallback((path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const renderTarget = (target: Target) => {
    const cfg = target.status ? STATUS_CONFIG[target.status] || STATUS_CONFIG.new : null;
    const isEditing = editingId === target.id;
    return (
      <div
        key={target.id}
        className={cn(
          "px-3 py-1.5 hover:bg-muted/30 transition-colors group cursor-pointer rounded",
          target.scope === "out" && "opacity-50",
          isEditing && "bg-muted/20"
        )}
        onClick={() => setEditingId(isEditing ? null : target.id)}
      >
        <div className="flex items-center gap-2">
          {TYPE_ICONS[target.type] || <Globe className="w-3.5 h-3.5" />}
          <button
            type="button"
            className={cn(
              "p-0.5 rounded transition-colors",
              target.scope === "in"
                ? "text-green-400 hover:text-green-300"
                : "text-red-400 hover:text-red-300"
            )}
            onClick={(e) => {
              e.stopPropagation();
              onToggleScope(target);
            }}
            title={target.scope === "in" ? t("targets.inScope") : t("targets.outOfScope")}
          >
            {target.scope === "in" ? (
              <Shield className="w-3 h-3" />
            ) : (
              <ShieldOff className="w-3 h-3" />
            )}
          </button>
          <span className="text-xs font-mono text-foreground flex-1 truncate">{target.value}</span>
          {cfg && target.status !== "new" && (
            <span
              className={cn("text-[10px] px-1.5 py-0.5 rounded font-medium", cfg.color, cfg.bg)}
            >
              {cfg.label}
            </span>
          )}
          {target.ports && target.ports.length > 0 && (
            <span
              className="flex items-center gap-0.5 text-[10px] text-emerald-400/80"
              title={`${target.ports.length} open port(s)`}
            >
              <Wifi className="w-2.5 h-2.5" />
              {target.ports.length}
            </span>
          )}
          <button
            type="button"
            className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 text-muted-foreground hover:text-red-400 transition-all"
            onClick={(e) => {
              e.stopPropagation();
              onDelete(target.id);
            }}
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
        {isEditing && (
          <TargetDetailView
            target={target}
            t={t}
            onUpdateNotes={onUpdateNotes}
            onScan={onScan}
          />
        )}
      </div>
    );
  };

  const renderNode = (node: TreeNode, depth: number) => {
    const isCollapsed = collapsed.has(node.path);
    const counts = countAllTargets(node);
    const isUnassigned = node.path === UNASSIGNED_KEY;
    return (
      <div key={node.path}>
        <button
          type="button"
          className="flex items-center gap-2 w-full px-2 py-1.5 hover:bg-muted/20 transition-colors text-left rounded"
          style={{ paddingLeft: `${8 + depth * 16}px` }}
          onClick={() => toggleCollapse(node.path)}
        >
          <ChevronDown
            className={cn(
              "w-3 h-3 text-muted-foreground/60 transition-transform",
              isCollapsed && "-rotate-90"
            )}
          />
          {isUnassigned ? (
            <FolderOpen className="w-3.5 h-3.5 text-muted-foreground/60" />
          ) : (
            <Building2 className="w-3.5 h-3.5 text-accent/70" />
          )}
          <span className="text-xs font-medium text-foreground">{node.name}</span>
          <span className="text-[10px] text-muted-foreground/60 tabular-nums">{counts.total}</span>
          {counts.inScope > 0 && (
            <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-400">
              {counts.inScope} in
            </span>
          )}
          {!isUnassigned && node.children.size > 0 && (
            <span className="text-[9px] text-muted-foreground/50">
              · {node.children.size} sub
            </span>
          )}
        </button>
        {!isCollapsed && (
          <div className="space-y-px">
            {node.targets.length > 0 && (
              <div
                className="space-y-px py-0.5"
                style={{ paddingLeft: `${8 + (depth + 1) * 16}px` }}
              >
                {node.targets.map((target) => renderTarget(target))}
              </div>
            )}
            {[...node.children.values()].map((child) => renderNode(child, depth + 1))}
          </div>
        )}
      </div>
    );
  };

  if (targets.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <Crosshair className="w-8 h-8 mb-2 opacity-30" />
        <p className="text-xs">{t("targets.noTargets")}</p>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto py-2 px-1 space-y-px">
      {roots.map((node) => renderNode(node, 0))}
    </div>
  );
}
