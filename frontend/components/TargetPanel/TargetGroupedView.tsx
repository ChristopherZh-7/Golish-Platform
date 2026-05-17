/**
 * `TargetGroupedView` — Schema E (2026-05-17) unified org + target panel.
 *
 * History:
 * - Before E: this was a *read-only* tree that visualized targets folded by
 *   `organization_id` and the only entry point for *creating* organizations
 *   was a separate "OrganizationsPanel" (different tab in the side bar).
 * - After E: the org and target lifecycles got merged. This panel now owns
 *   both, so the natural workflow ("create org → add targets under it")
 *   happens in one place. Per-node hover actions:
 *
 *   - `+ Crosshair`  → inline add target under this org
 *   - `+ Building2`  → inline add **child** org under this org
 *   - `Pencil`       → inline edit org name / owner
 *   - `Trash2`       → delete the org (cascade — backend uses ON DELETE CASCADE)
 *
 *   The top action bar exposes `+ create root org` for the "I want a new
 *   top-level org" path; OrganizationsPanel is retired.
 *
 * Notes:
 * - Org CRUD is wired directly to `orgsApi` so the side bar isn't a fork of
 *   state.
 * - Target CRUD is delegated upward via `onAdd` / `onDelete` / `onToggleScope`
 *   so that this view shares the same audit/event/store plumbing as
 *   `TargetListView`.
 * - `editingOrgId`, `addingChildTo`, `addingTargetTo`, `editingTargetId` are
 *   mutually exclusive — opening one auto-closes the others to avoid the
 *   UI sprouting a forest of inline editors.
 */

import {
  Building2,
  Check,
  ChevronDown,
  Crosshair,
  FolderOpen,
  Globe,
  Hash,
  Info,
  Loader2,
  Network,
  Pencil,
  Plus,
  Shield,
  ShieldOff,
  Trash2,
  Wifi,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { organizations as orgsApi } from "@/lib/api";
import type { Organization } from "@/lib/api/organizations";
import type { Target, TargetStatus } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { cn } from "@/lib/utils";
import { OrgProfileDrawer } from "./OrgProfileDrawer";
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
const ROOT_PARENT_KEY = "__root__";

interface OrgTreeNode {
  id: string;
  name: string;
  children: OrgTreeNode[];
  targets: Target[];
}

/**
 * Build the redteam tree: organizations form the spine (parent_id chain),
 * targets attach to their `organization_id`, and any orphan targets land in
 * a dedicated "unassigned" bucket at the bottom so legacy/imported data is
 * still reachable rather than silently hidden.
 */
function buildOrgTree(
  orgs: Organization[],
  targets: Target[],
  unassignedLabel: string
): OrgTreeNode[] {
  const nodeMap = new Map<string, OrgTreeNode>();
  for (const o of orgs) {
    nodeMap.set(o.id, { id: o.id, name: o.name, children: [], targets: [] });
  }

  const roots: OrgTreeNode[] = [];
  for (const o of orgs) {
    const node = nodeMap.get(o.id);
    if (!node) continue;
    if (o.parent_id && nodeMap.has(o.parent_id)) {
      nodeMap.get(o.parent_id)!.children.push(node);
    } else {
      roots.push(node);
    }
  }

  const unassigned: OrgTreeNode = {
    id: UNASSIGNED_KEY,
    name: unassignedLabel,
    children: [],
    targets: [],
  };
  for (const t of targets) {
    const orgId = t.organization_id;
    if (orgId && nodeMap.has(orgId)) {
      nodeMap.get(orgId)!.targets.push(t);
    } else {
      unassigned.targets.push(t);
    }
  }

  const orderByOrg = new Map<string, number>();
  orgs.forEach((o, idx) => {
    orderByOrg.set(o.id, o.sort_order * 1_000_000 + idx);
  });

  const sortNodes = (nodes: OrgTreeNode[]): void => {
    nodes.sort((a, b) => {
      if (a.id === UNASSIGNED_KEY) return 1;
      if (b.id === UNASSIGNED_KEY) return -1;
      return (
        (orderByOrg.get(a.id) ?? 0) - (orderByOrg.get(b.id) ?? 0) ||
        a.name.localeCompare(b.name, "zh")
      );
    });
    for (const n of nodes) sortNodes(n.children);
  };
  sortNodes(roots);

  if (unassigned.targets.length > 0) {
    roots.push(unassigned);
  }
  return roots;
}

function countAllTargets(node: OrgTreeNode): { total: number; inScope: number } {
  let total = node.targets.length;
  let inScope = node.targets.filter((t) => t.scope === "in").length;
  for (const child of node.children) {
    const sub = countAllTargets(child);
    total += sub.total;
    inScope += sub.inScope;
  }
  return { total, inScope };
}

interface AddTargetForm {
  name: string;
  value: string;
  notes: string;
  tags: string;
  grp: string;
  owner: string;
  timeWindowStart: string;
  timeWindowEnd: string;
  organizationId?: string;
}

interface TargetGroupedViewProps {
  targets: Target[];
  t: (key: string) => string;
  onAdd: (form: AddTargetForm) => Promise<string | null>;
  onDelete: (id: string) => Promise<void>;
  onToggleScope: (target: Target) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
  onScan: (target: Target) => void;
}

export function TargetGroupedView({
  targets,
  t,
  onAdd,
  onDelete,
  onToggleScope,
  onUpdateNotes,
  onScan,
}: TargetGroupedViewProps) {
  const [orgs, setOrgs] = useState<Organization[]>([]);
  const [orgLoading, setOrgLoading] = useState(true);
  const [orgError, setOrgError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [editingTargetId, setEditingTargetId] = useState<string | null>(null);

  const [profileOrgId, setProfileOrgId] = useState<string | null>(null);
  const profileOrgName = useMemo(
    () => orgs.find((o) => o.id === profileOrgId)?.name,
    [orgs, profileOrgId]
  );

  // Inline editor / creator state — only one can be open at a time. `ROOT_PARENT_KEY`
  // is used by `addingChildTo` to mean "creating a new top-level org".
  const [addingChildTo, setAddingChildTo] = useState<string | null>(null);
  const [addingTargetTo, setAddingTargetTo] = useState<string | null>(null);
  const [editingOrgId, setEditingOrgId] = useState<string | null>(null);

  const [orgFormName, setOrgFormName] = useState("");
  const [orgFormOwner, setOrgFormOwner] = useState("");
  const [orgFormDesc, setOrgFormDesc] = useState("");
  const [targetFormValue, setTargetFormValue] = useState("");
  const [targetFormName, setTargetFormName] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [inlineError, setInlineError] = useState<string | null>(null);

  const refreshOrgs = useCallback(async () => {
    setOrgLoading(true);
    setOrgError(null);
    try {
      const list = await orgsApi.listOrganizations(getProjectPath());
      setOrgs(list);
    } catch (e) {
      setOrgError(String(e));
    } finally {
      setOrgLoading(false);
    }
  }, []);

  // Re-fetch orgs when targets count changes so that newly imported targets
  // land under the right org bucket even when the user is on this view.
  useEffect(() => {
    refreshOrgs();
  }, [refreshOrgs, targets.length]);

  const unassignedLabel = t("targets.unassigned");
  const roots = useMemo(
    () => buildOrgTree(orgs, targets, unassignedLabel),
    [orgs, targets, unassignedLabel]
  );

  const toggleCollapse = useCallback((id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const resetForms = useCallback(() => {
    setOrgFormName("");
    setOrgFormOwner("");
    setOrgFormDesc("");
    setTargetFormValue("");
    setTargetFormName("");
    setInlineError(null);
  }, []);

  const closeAllEditors = useCallback(() => {
    setAddingChildTo(null);
    setAddingTargetTo(null);
    setEditingOrgId(null);
    resetForms();
  }, [resetForms]);

  const handleStartAddChild = useCallback(
    (parentId: string | null) => {
      closeAllEditors();
      setAddingChildTo(parentId ?? ROOT_PARENT_KEY);
    },
    [closeAllEditors]
  );

  const handleStartAddTarget = useCallback(
    (orgId: string) => {
      closeAllEditors();
      setAddingTargetTo(orgId);
    },
    [closeAllEditors]
  );

  const handleStartEditOrg = useCallback(
    (node: OrgTreeNode) => {
      const orgRow = orgs.find((o) => o.id === node.id);
      closeAllEditors();
      setEditingOrgId(node.id);
      setOrgFormName(node.name);
      setOrgFormOwner(orgRow?.owner ?? "");
      setOrgFormDesc(orgRow?.description ?? "");
    },
    [orgs, closeAllEditors]
  );

  const handleCreateOrg = useCallback(
    async (parentId: string | null) => {
      if (!orgFormName.trim()) {
        setInlineError(t("organizations.namePlaceholder"));
        return;
      }
      setSubmitting(true);
      setInlineError(null);
      try {
        await orgsApi.createOrganization({
          projectPath: getProjectPath(),
          name: orgFormName.trim(),
          parentId: parentId ?? undefined,
          owner: orgFormOwner.trim() || undefined,
        });
        await refreshOrgs();
        closeAllEditors();
      } catch (e) {
        setInlineError(String(e));
      } finally {
        setSubmitting(false);
      }
    },
    [orgFormName, orgFormOwner, refreshOrgs, closeAllEditors, t]
  );

  const handleSaveEditOrg = useCallback(async () => {
    if (!editingOrgId || !orgFormName.trim()) return;
    setSubmitting(true);
    setInlineError(null);
    try {
      await orgsApi.updateOrganization({
        id: editingOrgId,
        name: orgFormName.trim(),
        owner: orgFormOwner.trim(),
        description: orgFormDesc.trim(),
      });
      await refreshOrgs();
      closeAllEditors();
    } catch (e) {
      setInlineError(String(e));
    } finally {
      setSubmitting(false);
    }
  }, [editingOrgId, orgFormName, orgFormOwner, orgFormDesc, refreshOrgs, closeAllEditors]);

  const handleDeleteOrg = useCallback(
    async (id: string, name: string) => {
      const confirmMsg = t("organizations.deleteConfirm").replace("{{name}}", name);
      if (!confirm(confirmMsg)) return;
      try {
        await orgsApi.deleteOrganization(id);
        await refreshOrgs();
      } catch (e) {
        alert(String(e));
      }
    },
    [refreshOrgs, t]
  );

  const handleAddTargetSubmit = useCallback(
    async (orgId: string) => {
      if (!targetFormValue.trim()) {
        setInlineError(t("targets.valueRequired"));
        return;
      }
      setSubmitting(true);
      setInlineError(null);
      const err = await onAdd({
        name: targetFormName,
        value: targetFormValue.trim(),
        notes: "",
        tags: "",
        grp: "",
        owner: "",
        timeWindowStart: "",
        timeWindowEnd: "",
        // The "unassigned" bucket is virtual — submitting from there leaves
        // organization_id undefined so the target stays unanchored.
        organizationId: orgId === UNASSIGNED_KEY ? undefined : orgId,
      });
      setSubmitting(false);
      if (err) {
        setInlineError(err);
      } else {
        closeAllEditors();
      }
    },
    [targetFormValue, targetFormName, onAdd, closeAllEditors, t]
  );

  const renderTarget = (target: Target) => {
    const cfg = target.status ? STATUS_CONFIG[target.status] || STATUS_CONFIG.new : null;
    const isEditing = editingTargetId === target.id;
    return (
      <div
        key={target.id}
        className={cn(
          "px-3 py-1.5 hover:bg-muted/30 transition-colors group cursor-pointer rounded",
          target.scope === "out" && "opacity-50",
          isEditing && "bg-muted/20"
        )}
        onClick={() => setEditingTargetId(isEditing ? null : target.id)}
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
          <TargetDetailView target={target} t={t} onUpdateNotes={onUpdateNotes} onScan={onScan} />
        )}
      </div>
    );
  };

  const renderInlineCreateOrgForm = (parentId: string | null, depth: number) => (
    <div
      className="px-2 py-2 bg-muted/10 border-l-2 border-accent/40"
      style={{ marginLeft: `${8 + depth * 16}px` }}
    >
      <div className="flex items-center gap-2">
        <Building2 className="w-3 h-3 text-accent/70" />
        <input
          className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
          placeholder={t("organizations.namePlaceholder")}
          value={orgFormName}
          onChange={(e) => setOrgFormName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleCreateOrg(parentId);
            if (e.key === "Escape") closeAllEditors();
          }}
          // biome-ignore lint/a11y/noAutofocus: inline edit affordance needs immediate focus
          autoFocus
        />
        <input
          className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-32"
          placeholder={t("organizations.ownerPlaceholder")}
          value={orgFormOwner}
          onChange={(e) => setOrgFormOwner(e.target.value)}
        />
        <button
          type="button"
          className={cn(
            "p-1 rounded text-green-400 hover:bg-green-500/10",
            (!orgFormName.trim() || submitting) && "opacity-50"
          )}
          disabled={!orgFormName.trim() || submitting}
          onClick={() => handleCreateOrg(parentId)}
        >
          <Check className="w-3 h-3" />
        </button>
        <button
          type="button"
          className="p-1 rounded text-muted-foreground hover:text-foreground"
          onClick={closeAllEditors}
        >
          <X className="w-3 h-3" />
        </button>
      </div>
      {inlineError && <p className="text-[10px] text-red-400 mt-1">{inlineError}</p>}
    </div>
  );

  const renderInlineAddTargetForm = (orgId: string, depth: number) => (
    <div
      className="px-2 py-2 bg-muted/10 border-l-2 border-accent/40"
      style={{ marginLeft: `${8 + depth * 16}px` }}
    >
      <div className="flex items-center gap-2">
        <Crosshair className="w-3 h-3 text-accent/70" />
        <input
          className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
          placeholder={`${t("targets.value")} * (e.g. example.com)`}
          value={targetFormValue}
          onChange={(e) => setTargetFormValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleAddTargetSubmit(orgId);
            if (e.key === "Escape") closeAllEditors();
          }}
          // biome-ignore lint/a11y/noAutofocus: inline edit affordance needs immediate focus
          autoFocus
        />
        <input
          className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-32"
          placeholder={`${t("targets.name")} (${t("common.default")}: ${t("targets.value")})`}
          value={targetFormName}
          onChange={(e) => setTargetFormName(e.target.value)}
        />
        <button
          type="button"
          className={cn(
            "p-1 rounded text-green-400 hover:bg-green-500/10",
            (!targetFormValue.trim() || submitting) && "opacity-50"
          )}
          disabled={!targetFormValue.trim() || submitting}
          onClick={() => handleAddTargetSubmit(orgId)}
        >
          <Check className="w-3 h-3" />
        </button>
        <button
          type="button"
          className="p-1 rounded text-muted-foreground hover:text-foreground"
          onClick={closeAllEditors}
        >
          <X className="w-3 h-3" />
        </button>
      </div>
      {inlineError && <p className="text-[10px] text-red-400 mt-1">{inlineError}</p>}
    </div>
  );

  const renderOrgEditForm = (depth: number) => (
    <div
      className="flex items-center gap-2 px-2 py-1.5 bg-muted/15 rounded"
      style={{ paddingLeft: `${8 + depth * 16}px` }}
    >
      <Building2 className="w-3.5 h-3.5 text-accent/70" />
      <input
        className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent"
        value={orgFormName}
        onChange={(e) => setOrgFormName(e.target.value)}
        // biome-ignore lint/a11y/noAutofocus: inline edit affordance needs immediate focus
        autoFocus
      />
      <input
        className="text-xs bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent w-32"
        placeholder={t("organizations.ownerPlaceholder")}
        value={orgFormOwner}
        onChange={(e) => setOrgFormOwner(e.target.value)}
      />
      <button
        type="button"
        className={cn(
          "p-1 rounded text-green-400 hover:bg-green-500/10",
          (!orgFormName.trim() || submitting) && "opacity-50"
        )}
        disabled={!orgFormName.trim() || submitting}
        onClick={handleSaveEditOrg}
      >
        <Check className="w-3 h-3" />
      </button>
      <button
        type="button"
        className="p-1 rounded text-muted-foreground hover:text-foreground"
        onClick={closeAllEditors}
      >
        <X className="w-3 h-3" />
      </button>
    </div>
  );

  const renderNode = (node: OrgTreeNode, depth: number) => {
    const isCollapsed = collapsed.has(node.id);
    const counts = countAllTargets(node);
    const isUnassigned = node.id === UNASSIGNED_KEY;
    const isEditingThis = editingOrgId === node.id;

    return (
      <div key={node.id}>
        {isEditingThis ? (
          renderOrgEditForm(depth)
        ) : (
          <div
            className="flex items-center gap-1 px-2 py-1.5 hover:bg-muted/20 transition-colors group rounded"
            style={{ paddingLeft: `${8 + depth * 16}px` }}
          >
            <button
              type="button"
              onClick={() => toggleCollapse(node.id)}
              className="flex items-center gap-2 flex-1 text-left min-w-0"
            >
              <ChevronDown
                className={cn(
                  "w-3 h-3 text-muted-foreground/60 transition-transform flex-shrink-0",
                  isCollapsed && "-rotate-90"
                )}
              />
              {isUnassigned ? (
                <FolderOpen className="w-3.5 h-3.5 text-muted-foreground/60 flex-shrink-0" />
              ) : (
                <Building2 className="w-3.5 h-3.5 text-accent/70 flex-shrink-0" />
              )}
              <span className="text-xs font-medium text-foreground truncate">{node.name}</span>
              <span className="text-[10px] text-muted-foreground/60 tabular-nums">
                {counts.total}
              </span>
              {counts.inScope > 0 && (
                <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-400">
                  {counts.inScope} in
                </span>
              )}
              {!isUnassigned && node.children.length > 0 && (
                <span className="text-[9px] text-muted-foreground/50">
                  · {node.children.length} sub
                </span>
              )}
            </button>

            <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
              <button
                type="button"
                className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-accent"
                onClick={() => handleStartAddTarget(node.id)}
                title={t("targets.addTarget")}
              >
                <Crosshair className="w-3 h-3" />
              </button>
              {!isUnassigned && (
                <>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-accent"
                    onClick={() => handleStartAddChild(node.id)}
                    title={t("organizations.create")}
                  >
                    <Building2 className="w-3 h-3" />
                  </button>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-blue-400"
                    onClick={() => setProfileOrgId(node.id)}
                    title={t("organizations.profile.openButton")}
                  >
                    <Info className="w-3 h-3" />
                  </button>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground"
                    onClick={() => handleStartEditOrg(node)}
                    title={t("organizations.edit")}
                  >
                    <Pencil className="w-3 h-3" />
                  </button>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-red-500/20 text-muted-foreground hover:text-red-400"
                    onClick={() => handleDeleteOrg(node.id, node.name)}
                    title={t("organizations.delete")}
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </>
              )}
            </div>
          </div>
        )}

        {addingTargetTo === node.id && renderInlineAddTargetForm(node.id, depth + 1)}
        {addingChildTo === node.id && renderInlineCreateOrgForm(node.id, depth + 1)}

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
            {node.children.map((child) => renderNode(child, depth + 1))}
          </div>
        )}
      </div>
    );
  };

  if (orgLoading) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground text-xs gap-2">
        <Loader2 className="w-4 h-4 animate-spin" />
        {t("organizations.loading")}
      </div>
    );
  }

  if (orgError) {
    return <div className="px-4 py-3 text-[11px] text-red-400 bg-red-500/10">{orgError}</div>;
  }

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 py-2 border-b border-border/30 bg-muted/5 flex items-center gap-2">
        <button
          type="button"
          className="flex items-center gap-1.5 px-2 py-1 text-xs rounded bg-accent/10 hover:bg-accent/20 text-accent transition-colors"
          onClick={() => handleStartAddChild(null)}
        >
          <Plus className="w-3 h-3" />
          {t("organizations.create")}
        </button>
        <span className="text-[10px] text-muted-foreground/60 tabular-nums">
          {orgs.length} orgs · {targets.length} targets
        </span>
      </div>

      {addingChildTo === ROOT_PARENT_KEY && renderInlineCreateOrgForm(null, 0)}

      {roots.length === 0 && targets.length === 0 ? (
        <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground px-6 text-center">
          <Building2 className="w-8 h-8 mb-2 opacity-30" />
          <p className="text-xs">{t("targets.treeEmpty")}</p>
          <p className="text-[10px] text-muted-foreground/60 mt-1">{t("targets.treeEmptyHint")}</p>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto py-2 px-1 space-y-px">
          {roots.map((node) => renderNode(node, 0))}
        </div>
      )}

      <OrgProfileDrawer
        orgId={profileOrgId}
        orgName={profileOrgName}
        open={profileOrgId !== null}
        onClose={() => {
          setProfileOrgId(null);
          // Refresh the flat list — name / metadata may have changed and
          // the tree pulls labels straight from the row, so a stale name
          // would otherwise linger until the next remount.
          void refreshOrgs();
        }}
        t={t}
      />
    </div>
  );
}
