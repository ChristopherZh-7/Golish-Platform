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
import { type EngagementMode, NewEngagementDialog } from "./NewEngagementDialog";
import { TargetDetailView } from "./TargetDetail";

const TYPE_ICONS: Record<string, React.ReactNode> = {
  domain: <Globe className="w-3 h-3 text-blue-400" />,
  ip: <Hash className="w-3 h-3 text-green-400" />,
  cidr: <Network className="w-3 h-3 text-yellow-400" />,
  url: <Globe className="w-3 h-3 text-purple-400" />,
  wildcard: <Crosshair className="w-3 h-3 text-orange-400" />,
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

const ENGAGEMENT_BADGES = {
  customer_targets: {
    label: "customer scope",
    className: "bg-green-500/10 text-green-400",
  },
  discover_assets: {
    label: "discovery",
    className: "bg-blue-500/10 text-blue-400",
  },
  profile_only: {
    label: "profile",
    className: "bg-muted/40 text-muted-foreground",
  },
} as const;

type OrgActionKind =
  | "import_targets"
  | "hydrate_intel"
  | "choose_next_step"
  | "review_scope"
  | "add_child";

interface OrgActionItem {
  kind: OrgActionKind;
  label: string;
}

export function getOrgActionModel(mode: EngagementMode | null): {
  primary: OrgActionItem;
  secondary?: OrgActionItem;
} {
  if (mode === "customer_targets") {
    return {
      primary: { kind: "import_targets", label: "Import targets" },
      secondary: { kind: "review_scope", label: "Review scope" },
    };
  }
  if (mode === "discover_assets") {
    return {
      primary: { kind: "hydrate_intel", label: "Hydrate intel" },
      secondary: { kind: "add_child", label: "Add child org" },
    };
  }
  return {
    primary: { kind: "choose_next_step", label: "Choose next step" },
    secondary: { kind: "import_targets", label: "Import targets" },
  };
}

export function getWorkspaceModel(mode: EngagementMode | null): {
  title: string;
  eyebrow: string;
  description: string;
} {
  if (mode === "customer_targets") {
    return {
      title: "Targets & Testing",
      eyebrow: "Customer provided scope",
      description: "Review the customer-provided target list, confirm scope, then start recon.",
    };
  }
  if (mode === "discover_assets") {
    return {
      title: "Scope & Intel",
      eyebrow: "Asset discovery workspace",
      description:
        "Hydrate company intel, review discovered candidates, then promote approved assets.",
    };
  }
  return {
    title: "Overview",
    eyebrow: "Organization profile",
    description:
      "This org is a customer record. Choose whether to import targets or discover assets.",
  };
}

type EngagementRecord = Record<string, unknown>;

export function getEngagementDetails(
  engagement: EngagementRecord | null | undefined
): Array<[string, string]> {
  if (!engagement) return [];
  const mode = engagement.mode;
  if (mode === "discover_assets") {
    const details: Array<[string, string]> = [];
    const ownership = String(engagement.min_ownership_percent ?? "").trim();
    if (ownership) details.push(["Min ownership", `${ownership}%`]);
    const depth = String(engagement.depth ?? "").trim();
    if (depth) details.push(["Depth", depth]);
    if (typeof engagement.include_branches === "boolean") {
      details.push(["Branches", engagement.include_branches ? "included" : "excluded"]);
    }
    if (typeof engagement.create_candidates === "boolean") {
      details.push(["Candidates", engagement.create_candidates ? "review first" : "disabled"]);
    }
    return details;
  }
  if (mode === "customer_targets") {
    const details: Array<[string, string]> = [];
    const source = String(engagement.source ?? "").trim();
    if (source) details.push(["Source", source]);
    if (typeof engagement.target_count === "number") {
      details.push(["Imported targets", String(engagement.target_count)]);
    }
    return details;
  }
  return [];
}

export function getCandidateCounts(engagement: EngagementRecord | null | undefined): {
  organizations: number;
  targets: number;
} {
  const candidates = engagement?.candidates;
  if (!candidates || typeof candidates !== "object" || Array.isArray(candidates)) {
    return { organizations: 0, targets: 0 };
  }
  const record = candidates as { organizations?: unknown; targets?: unknown };
  return {
    organizations: Array.isArray(record.organizations) ? record.organizations.length : 0,
    targets: Array.isArray(record.targets) ? record.targets.length : 0,
  };
}

type WorkspaceTab = "overview" | "fields" | "scope" | "targets" | "candidates" | "activity";

interface OrgFieldInput {
  aliases?: unknown[];
  industry?: string;
  tier?: string;
  credit_code?: string;
  domains?: unknown[];
  ip_ranges?: unknown[];
  asns?: unknown[];
  email_domains?: unknown[];
  scope_rules?: unknown;
  intel?: Record<string, unknown>;
  notes?: string;
  certificates?: unknown[];
  subsidiaries?: unknown[];
  business_systems?: unknown[];
  cloud_assets?: unknown[];
  github_orgs?: unknown[];
  social_accounts?: unknown[];
  historical_vulns?: unknown[];
  contacts?: unknown[];
}

interface OrgFieldView {
  key: string;
  label: string;
  value: string;
  filled: boolean;
}

interface OrgFieldGroup {
  title: string;
  fields: OrgFieldView[];
}

function displayAtom(value: unknown): string {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    return String(record.domain ?? record.name ?? record.value ?? record.id ?? "").trim();
  }
  return String(value ?? "").trim();
}

export function formatFieldValue(value: unknown): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return "—";
    const samples = value.map(displayAtom).filter(Boolean);
    if (samples.length === 0) return `${value.length} item(s)`;
    const shown = samples.slice(0, 3).join(", ");
    const rest = samples.length - 3;
    return rest > 0 ? `${shown} +${rest}` : shown;
  }
  if (value && typeof value === "object") {
    return Object.keys(value as Record<string, unknown>).length > 0 ? "configured" : "—";
  }
  const text = String(value ?? "").trim();
  return text || "—";
}

function field(key: string, label: string, value: unknown): OrgFieldView {
  const formatted = formatFieldValue(value);
  return { key, label, value: formatted, filled: formatted !== "—" };
}

export function getOrgFieldGroups(org: OrgFieldInput): OrgFieldGroup[] {
  return [
    {
      title: "Basic",
      fields: [
        field("aliases", "Aliases", org.aliases),
        field("industry", "Industry", org.industry),
        field("tier", "Priority", org.tier),
        field("credit_code", "Unified social credit code", org.credit_code),
      ],
    },
    {
      title: "Domains",
      fields: [field("domains", "Domains", org.domains)],
    },
    {
      title: "Network",
      fields: [
        field("ip_ranges", "IP ranges", org.ip_ranges),
        field("asns", "ASNs", org.asns),
        field("email_domains", "Email domains", org.email_domains),
      ],
    },
    {
      title: "Scope",
      fields: [field("scope_rules", "Scope rules", org.scope_rules)],
    },
    {
      title: "Other",
      fields: [
        field("intel", "Intel records", org.intel),
        field("notes", "Notes", org.notes),
        field("certificates", "Certificates", org.certificates),
        field("subsidiaries", "Subsidiaries", org.subsidiaries),
        field("business_systems", "Business systems", org.business_systems),
        field("cloud_assets", "Cloud assets", org.cloud_assets),
        field("github_orgs", "GitHub orgs", org.github_orgs),
        field("social_accounts", "Social accounts", org.social_accounts),
        field("historical_vulns", "Historical vulns", org.historical_vulns),
        field("contacts", "Contacts", org.contacts),
      ],
    },
  ];
}

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

function getEngagementMode(org?: Organization): EngagementMode | null {
  const engagement = org?.intel?.engagement;
  if (!engagement || typeof engagement !== "object" || Array.isArray(engagement)) return null;
  const mode = (engagement as { mode?: unknown }).mode;
  if (mode === "customer_targets" || mode === "discover_assets" || mode === "profile_only") {
    return mode;
  }
  return null;
}

function getEngagementRecord(org?: Organization): EngagementRecord | null {
  const engagement = org?.intel?.engagement;
  if (!engagement || typeof engagement !== "object" || Array.isArray(engagement)) return null;
  return engagement as EngagementRecord;
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
  onBatchAdd: (
    values: string,
    grp: string,
    organizationId?: string,
    source?: string
  ) => Promise<Target[]>;
  onDelete: (id: string) => Promise<void>;
  onToggleScope: (target: Target) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
  onScan: (target: Target) => void;
}

export function TargetGroupedView({
  targets,
  t,
  onAdd,
  onBatchAdd,
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
  const [newEngagementOpen, setNewEngagementOpen] = useState(false);
  const [newEngagementMode, setNewEngagementMode] = useState<EngagementMode>("customer_targets");
  const [selectedOrgId, setSelectedOrgId] = useState<string | null>(null);
  const [workspaceTab, setWorkspaceTab] = useState<WorkspaceTab>("overview");

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
  const selectedOrg = useMemo(
    () => orgs.find((o) => o.id === selectedOrgId) ?? orgs[0] ?? null,
    [orgs, selectedOrgId]
  );
  const selectedMode = getEngagementMode(selectedOrg ?? undefined);
  const selectedTargets = useMemo(
    () =>
      selectedOrg ? targets.filter((target) => target.organization_id === selectedOrg.id) : [],
    [selectedOrg, targets]
  );

  useEffect(() => {
    if (!selectedOrgId && orgs.length > 0) {
      setSelectedOrgId(orgs[0].id);
      return;
    }
    if (selectedOrgId && !orgs.some((org) => org.id === selectedOrgId)) {
      setSelectedOrgId(orgs[0]?.id ?? null);
    }
  }, [orgs, selectedOrgId]);

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

  const handleCreateEngagementOrg = useCallback(
    async (params: { name: string; owner: string; description: string }) =>
      orgsApi.createOrganization({
        projectPath: getProjectPath(),
        name: params.name,
        owner: params.owner || undefined,
        description: params.description || undefined,
      }),
    []
  );

  const handleUpdateEngagementProfile = useCallback(
    (id: string, patch: Parameters<typeof orgsApi.updateOrganizationProfile>[1]) =>
      orgsApi.updateOrganizationProfile(id, patch),
    []
  );

  const renderOrgActionButton = (action: OrgActionItem, node: OrgTreeNode) => {
    const runAction = (e: React.MouseEvent) => {
      e.stopPropagation();
      switch (action.kind) {
        case "import_targets":
          handleStartAddTarget(node.id);
          break;
        case "hydrate_intel":
          setSelectedOrgId(node.id);
          setWorkspaceTab("activity");
          break;
        case "choose_next_step":
          setSelectedOrgId(node.id);
          setWorkspaceTab("overview");
          break;
        case "review_scope":
          setSelectedOrgId(node.id);
          setWorkspaceTab("candidates");
          break;
        case "add_child":
          handleStartAddChild(node.id);
          break;
      }
    };

    const icon =
      action.kind === "hydrate_intel" ? (
        <Network className="w-3 h-3" />
      ) : action.kind === "add_child" ? (
        <Building2 className="w-3 h-3" />
      ) : action.kind === "review_scope" ? (
        <Shield className="w-3 h-3" />
      ) : action.kind === "choose_next_step" ? (
        <Info className="w-3 h-3" />
      ) : (
        <Crosshair className="w-3 h-3" />
      );

    return (
      <button
        key={action.kind}
        type="button"
        className={cn(
          "p-1 rounded hover:bg-muted/50 text-muted-foreground transition-colors",
          action.kind === "hydrate_intel" && "hover:text-blue-400",
          action.kind === "import_targets" && "hover:text-green-400",
          action.kind === "review_scope" && "hover:text-amber-400",
          action.kind === "choose_next_step" && "hover:text-accent",
          action.kind === "add_child" && "hover:text-accent"
        )}
        onClick={runAction}
        title={action.label}
      >
        {icon}
      </button>
    );
  };

  const renderWorkspacePanel = () => {
    if (!selectedOrg) {
      return (
        <div className="h-full flex items-center justify-center text-center text-muted-foreground px-8">
          <div>
            <Building2 className="w-8 h-8 mx-auto mb-2 opacity-30" />
            <p className="text-xs">Select or create an organization to start.</p>
          </div>
        </div>
      );
    }

    const workspace = getWorkspaceModel(selectedMode);
    const engagementRecord = getEngagementRecord(selectedOrg);
    const engagementDetails = getEngagementDetails(engagementRecord);
    const candidateCounts = getCandidateCounts(engagementRecord);
    const inScopeCount = selectedTargets.filter((target) => target.scope === "in").length;
    const outScopeCount = selectedTargets.filter((target) => target.scope === "out").length;
    const badge = selectedMode ? ENGAGEMENT_BADGES[selectedMode] : null;
    const fieldGroups = getOrgFieldGroups(selectedOrg);

    return (
      <div className="h-full overflow-y-auto p-3 space-y-3">
        <section className="border-b border-border/30 pb-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-[9px] uppercase tracking-wide text-muted-foreground/55">
                {workspace.eyebrow}
              </p>
              <div className="mt-1 flex items-center gap-1.5 min-w-0">
                <Building2 className="w-3 h-3 text-accent/80 flex-shrink-0" />
                <h3 className="text-[12px] font-medium text-foreground truncate">
                  {selectedOrg.name}
                </h3>
                {badge && (
                  <span className={cn("text-[9px] px-1.5 py-0.5 rounded", badge.className)}>
                    {badge.label}
                  </span>
                )}
              </div>
              <p className="mt-1 text-[10px] text-muted-foreground/70 leading-relaxed">
                {workspace.description}
              </p>
              {engagementDetails.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1">
                  {engagementDetails.map(([label, value]) => (
                    <span
                      key={`${label}:${value}`}
                      className="rounded border border-border/35 bg-background/40 px-1.5 py-0.5 text-[9px] text-muted-foreground"
                    >
                      {label}: <span className="text-foreground/80">{value}</span>
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>

          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <span className="rounded bg-muted/20 px-1.5 py-0.5 text-[10px] text-muted-foreground">
              Targets <span className="text-foreground">{selectedTargets.length}</span>
            </span>
            <span className="rounded bg-green-500/10 px-1.5 py-0.5 text-[10px] text-green-400">
              In <span className="text-green-300">{inScopeCount}</span>
            </span>
            <span className="rounded bg-muted/20 px-1.5 py-0.5 text-[10px] text-muted-foreground">
              Out <span className="text-foreground/75">{outScopeCount}</span>
            </span>
          </div>
        </section>

        <nav className="flex items-center gap-1 border-b border-border/30 pb-2">
          {[
            ["overview", "Overview"],
            ["fields", "Fields"],
            ["scope", "Scope"],
            ["targets", "Targets"],
            ["candidates", "Candidates"],
            ["activity", "Activity"],
          ].map(([id, label]) => (
            <button
              key={id}
              type="button"
              className={cn(
                "px-2 py-1 rounded text-[10px] transition-colors",
                workspaceTab === id
                  ? "bg-accent/15 text-accent"
                  : "text-muted-foreground hover:bg-muted/30 hover:text-foreground"
              )}
              onClick={() => setWorkspaceTab(id as WorkspaceTab)}
            >
              {label}
            </button>
          ))}
        </nav>

        {workspaceTab === "activity" && (
          <section className="rounded border border-border/35 bg-muted/5 p-3">
            <h4 className="text-xs font-medium text-foreground">Activity</h4>
            <p className="mt-1 text-[10px] text-muted-foreground/70">
              Import, hydrate, candidate review, and recon runs will appear here.
            </p>
            <div className="mt-3 rounded border border-dashed border-border/35 p-3 text-center">
              <p className="text-[11px] text-muted-foreground">No activity yet.</p>
            </div>
          </section>
        )}

        {workspaceTab === "fields" && (
          <section className="rounded border border-border/35 bg-muted/5 p-3">
            <h4 className="text-xs font-medium text-foreground">Intel fields</h4>
            <p className="mt-1 text-[10px] text-muted-foreground/70">
              Field coverage by group. Editing will be a separate compact mode.
            </p>
            <div className="mt-3 space-y-2">
              {fieldGroups.map((group) => (
                <div
                  key={group.title}
                  className="rounded border border-border/30 bg-background/35 p-2"
                >
                  <div className="flex items-center justify-between">
                    <p className="text-[11px] font-medium text-foreground">{group.title}</p>
                    <span className="text-[10px] text-muted-foreground">
                      {group.fields.filter((item) => item.filled).length}/{group.fields.length}
                    </span>
                  </div>
                  <div className="mt-1.5 grid grid-cols-2 gap-1">
                    {group.fields.map((item) => (
                      <div
                        key={item.key}
                        className="flex items-center justify-between gap-2 text-[10px]"
                      >
                        <span className="text-muted-foreground truncate">{item.label}</span>
                        <span
                          className={cn(
                            "truncate",
                            item.filled ? "text-foreground" : "text-muted-foreground/50"
                          )}
                        >
                          {item.value}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </section>
        )}

        {(workspaceTab === "overview" || workspaceTab === "targets") && (
          <section className="rounded border border-border/35 bg-muted/5 p-3">
            <div className="flex items-center justify-between gap-2">
              <div>
                <h4 className="text-xs font-medium text-foreground">{workspace.title}</h4>
                <p className="text-[10px] text-muted-foreground/70 mt-1">
                  Mode-aware workspace skeleton. Backend orchestration and coverage panels come
                  next.
                </p>
              </div>
            </div>

            {selectedTargets.length === 0 ? (
              <div className="mt-3 rounded border border-dashed border-border/35 p-3 text-center">
                <Crosshair className="w-5 h-5 mx-auto text-muted-foreground/35 mb-1.5" />
                <p className="text-[11px] text-muted-foreground">
                  No targets linked to this organization yet.
                </p>
              </div>
            ) : (
              <div className="mt-3 space-y-1">
                {selectedTargets.slice(0, 8).map((target) => (
                  <button
                    key={target.id}
                    type="button"
                    className="w-full flex items-center gap-2 rounded px-2 py-1 text-left hover:bg-muted/30"
                    onClick={() => setEditingTargetId(target.id)}
                  >
                    {TYPE_ICONS[target.type] || <Globe className="w-3.5 h-3.5" />}
                    <span className="text-xs font-mono text-foreground truncate flex-1">
                      {target.value}
                    </span>
                    <span
                      className={cn(
                        "text-[9px] px-1.5 py-0.5 rounded",
                        target.scope === "in"
                          ? "bg-green-500/10 text-green-400"
                          : "bg-muted/40 text-muted-foreground"
                      )}
                    >
                      {target.scope}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </section>
        )}

        {workspaceTab === "candidates" && (
          <section
            className={cn(
              "rounded border p-3",
              candidateCounts.organizations + candidateCounts.targets > 0
                ? "border-amber-500/30 bg-amber-500/5"
                : "border-border/40 bg-muted/5"
            )}
          >
            <h4 className="text-xs font-medium text-foreground">Discovery candidates</h4>
            <p className="text-[10px] text-muted-foreground/70 mt-1">
              Candidate review is the next backend phase. Discovered subsidiaries and assets should
              land here before they become in-scope targets.
            </p>
            <div className="mt-3 grid grid-cols-2 gap-1.5">
              <div className="rounded border border-dashed border-border/40 p-2.5">
                <p className="text-[10px] text-muted-foreground/60">Organization candidates</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {candidateCounts.organizations > 0
                    ? `${candidateCounts.organizations} candidate(s) waiting for review`
                    : "No candidates yet."}
                </p>
              </div>
              <div className="rounded border border-dashed border-border/40 p-2.5">
                <p className="text-[10px] text-muted-foreground/60">Target candidates</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  {candidateCounts.targets > 0
                    ? `${candidateCounts.targets} candidate(s) waiting for review`
                    : "No candidates yet."}
                </p>
              </div>
            </div>
          </section>
        )}

        {workspaceTab === "scope" && (
          <section className="rounded border border-border/35 bg-muted/5 p-3">
            <h4 className="text-xs font-medium text-foreground">Scope</h4>
            <p className="mt-1 text-[10px] text-muted-foreground/70">
              Scope rules and authorization windows will be edited here.
            </p>
          </section>
        )}
      </div>
    );
  };

  const handleStartAddTarget = useCallback(
    (orgId: string) => {
      closeAllEditors();
      setAddingTargetTo(orgId);
    },
    [closeAllEditors]
  );

  const openNewEngagement = useCallback(
    (mode: EngagementMode) => {
      closeAllEditors();
      setNewEngagementMode(mode);
      setNewEngagementOpen(true);
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
          "px-2 py-1 hover:bg-muted/30 transition-colors group cursor-pointer rounded",
          target.scope === "out" && "opacity-50",
          isEditing && "bg-muted/20"
        )}
        onClick={() => setEditingTargetId(isEditing ? null : target.id)}
      >
        <div className="flex items-center gap-2">
          {TYPE_ICONS[target.type] || <Globe className="w-3 h-3" />}
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
              <Shield className="w-2.5 h-2.5" />
            ) : (
              <ShieldOff className="w-2.5 h-2.5" />
            )}
          </button>
          <span className="text-[11px] font-mono text-foreground flex-1 truncate">
            {target.value}
          </span>
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
            <Trash2 className="w-2.5 h-2.5" />
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
    const orgRow = orgs.find((o) => o.id === node.id);
    const engagementMode = getEngagementMode(orgRow);
    const badge = engagementMode ? ENGAGEMENT_BADGES[engagementMode] : null;
    const actionModel = getOrgActionModel(engagementMode);

    return (
      <div key={node.id}>
        {isEditingThis ? (
          renderOrgEditForm(depth)
        ) : (
          <div
            className={cn(
              "flex items-center gap-1 px-2 py-1 hover:bg-muted/20 transition-colors group rounded",
              selectedOrgId === node.id && !isUnassigned && "bg-muted/15"
            )}
            style={{ paddingLeft: `${8 + depth * 16}px` }}
          >
            <button
              type="button"
              onClick={() => {
                if (!isUnassigned) setSelectedOrgId(node.id);
                toggleCollapse(node.id);
              }}
              className="flex items-center gap-2 flex-1 text-left min-w-0"
            >
              <ChevronDown
                className={cn(
                  "w-3 h-3 text-muted-foreground/60 transition-transform flex-shrink-0",
                  isCollapsed && "-rotate-90"
                )}
              />
              {isUnassigned ? (
                <FolderOpen className="w-3 h-3 text-muted-foreground/60 flex-shrink-0" />
              ) : (
                <Building2 className="w-3 h-3 text-accent/70 flex-shrink-0" />
              )}
              <span className="text-[11px] font-medium text-foreground truncate">{node.name}</span>
              <span className="text-[10px] text-muted-foreground/60 tabular-nums">
                {counts.total}
              </span>
              {counts.inScope > 0 && (
                <span className="text-[9px] px-1 py-0.5 rounded bg-green-500/10 text-green-400">
                  {counts.inScope} in
                </span>
              )}
              {badge && (
                <span className={cn("text-[9px] px-1 py-0.5 rounded", badge.className)}>
                  {badge.label}
                </span>
              )}
              {!isUnassigned && node.children.length > 0 && (
                <span className="text-[9px] text-muted-foreground/50">
                  · {node.children.length} sub
                </span>
              )}
            </button>

            <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
              {isUnassigned ? (
                <button
                  type="button"
                  className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-accent"
                  onClick={() => handleStartAddTarget(node.id)}
                  title={t("targets.addTarget")}
                >
                  <Crosshair className="w-3 h-3" />
                </button>
              ) : (
                <>
                  {renderOrgActionButton(actionModel.primary, node)}
                  {actionModel.secondary && renderOrgActionButton(actionModel.secondary, node)}
                </>
              )}
              {!isUnassigned && (
                <>
                  <div className="w-px h-3 bg-border/40 mx-0.5" />
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-blue-400"
                    onClick={() => {
                      setSelectedOrgId(node.id);
                      setWorkspaceTab("fields");
                    }}
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
          onClick={() => openNewEngagement("customer_targets")}
        >
          <Plus className="w-3 h-3" />
          New Engagement
        </button>
        <button
          type="button"
          className="flex items-center gap-1.5 px-2 py-1 text-xs rounded hover:bg-muted/40 text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => handleStartAddChild(null)}
        >
          <Building2 className="w-3 h-3" />
          Quick org
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
          <div className="mt-4 grid grid-cols-1 sm:grid-cols-3 gap-2 w-full max-w-2xl">
            <button
              type="button"
              className="rounded-lg border border-border/40 bg-muted/10 px-3 py-3 text-left hover:bg-muted/20 transition-colors"
              onClick={() => openNewEngagement("customer_targets")}
            >
              <Shield className="w-4 h-4 text-green-400 mb-2" />
              <p className="text-xs text-foreground">Import targets</p>
              <p className="text-[10px] text-muted-foreground/70 mt-1">
                Customer provided scope list.
              </p>
            </button>
            <button
              type="button"
              className="rounded-lg border border-border/40 bg-muted/10 px-3 py-3 text-left hover:bg-muted/20 transition-colors"
              onClick={() => openNewEngagement("discover_assets")}
            >
              <Network className="w-4 h-4 text-accent mb-2" />
              <p className="text-xs text-foreground">Discover assets</p>
              <p className="text-[10px] text-muted-foreground/70 mt-1">
                Create org and prepare ASM flow.
              </p>
            </button>
            <button
              type="button"
              className="rounded-lg border border-border/40 bg-muted/10 px-3 py-3 text-left hover:bg-muted/20 transition-colors"
              onClick={() => handleStartAddChild(null)}
            >
              <Building2 className="w-4 h-4 text-muted-foreground mb-2" />
              <p className="text-xs text-foreground">Org profile only</p>
              <p className="text-[10px] text-muted-foreground/70 mt-1">Create a customer record.</p>
            </button>
          </div>
        </div>
      ) : (
        <div className="flex-1 min-h-0 grid grid-cols-[minmax(280px,0.9fr)_minmax(360px,1.1fr)]">
          <div className="min-h-0 overflow-y-auto py-2 px-1 space-y-px border-r border-border/30">
            {roots.map((node) => renderNode(node, 0))}
          </div>
          <div className="min-h-0 overflow-hidden bg-background/40">{renderWorkspacePanel()}</div>
        </div>
      )}

      <NewEngagementDialog
        open={newEngagementOpen}
        onOpenChange={setNewEngagementOpen}
        initialMode={newEngagementMode}
        onCreateOrganization={handleCreateEngagementOrg}
        onUpdateOrganizationProfile={handleUpdateEngagementProfile}
        onBatchAddTargets={(values, organizationId, source) =>
          onBatchAdd(values, "customer_provided", organizationId, source)
        }
        onCreated={refreshOrgs}
      />
    </div>
  );
}
