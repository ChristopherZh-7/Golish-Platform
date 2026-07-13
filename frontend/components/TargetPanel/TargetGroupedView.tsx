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
 * - The tree sidebar, org workspace, and inline forms now live in sibling
 *   components (`OrgTreeSidebar`, `OrgWorkspacePanel`, `InlineOrgForms`); pure
 *   org/engagement/asset-intel logic lives under `@/lib/target-panel/*`.
 */

import { save } from "@tauri-apps/plugin-dialog";
import { Building2, Loader2, Network, Plus, Shield } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { assetIntel, organizationRecon, organizations as orgsApi } from "@/lib/api";
import type { AssetIntelRun } from "@/lib/api/asset-intel";
import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { Organization } from "@/lib/api/organizations";
import { onCustomEvent, onEvent, sendCustomEvent } from "@/lib/events";
import { notify } from "@/lib/notify";
import type { Target } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { isIpLiteralTargetValue, type TargetAssetGroup } from "@/lib/target-panel/asset-groups";
import {
  applyStreamEvent,
  getNextWorkspaceTabAfterAssetIntelRun,
  type HydrateActivity,
} from "@/lib/target-panel/asset-intel";
import {
  buildDiscoveryHydrateConfigFromEngagement,
  buildHydrateConfigFromEngagement,
  getEffectiveEngagementMode,
  getEngagementRecord,
} from "@/lib/target-panel/engagement";
import { translateWithFallback } from "@/lib/target-panel/org-fields";
import {
  buildOrgTree,
  collectSubtreeTargets,
  countOrgDeletionImpact,
  findOrgTreeNode,
  type OrgTreeNode,
  ROOT_PARENT_KEY,
  summarizeTargetCounts,
  type TargetCountSummary,
  UNASSIGNED_KEY,
} from "@/lib/target-panel/org-tree";
import {
  applyOrganizationReconEvent,
  isOrganizationOwnedTarget,
  suggestedReconAssetsFilename,
} from "@/lib/target-panel/organization-recon";
import type { AssetIntelOrgActionKind, WorkspaceTab } from "@/lib/target-panel/types";
import { InlineCreateOrgForm } from "./InlineOrgForms";
import { type EngagementMode, NewEngagementDialog } from "./NewEngagementDialog";
import { OrgTreeSidebar } from "./OrgTreeSidebar";
import { OrgWorkspacePanel } from "./OrgWorkspacePanel";
import { TargetSurfaceWorkbench } from "./TargetSurfaceWorkbench";

// The pure org/engagement/asset-intel helpers now live under
// `@/lib/target-panel/*`. Re-export the names the action test imports from
// "./TargetGroupedView" so that test file stays unchanged.
export {
  applyStreamEvent,
  getNextWorkspaceTabAfterAssetIntelRun,
  getProviderStatusClass,
  type HydrateActivity,
} from "@/lib/target-panel/asset-intel";
export {
  buildDiscoveryHydrateConfigFromEngagement,
  buildHydrateConfigFromEngagement,
  getEngagementDetails,
  getOrgActionModel,
  getWorkspaceModel,
} from "@/lib/target-panel/engagement";
export { formatFieldValue, getOrgFieldGroups } from "@/lib/target-panel/org-fields";

export function toggleCollapsedSet(
  current: Set<string>,
  id: string,
  rootIds: Set<string> = new Set()
): Set<string> {
  const next = new Set(current);
  if (next.has(id)) {
    next.delete(id);
    if (rootIds.has(id)) {
      for (const rootId of rootIds) {
        if (rootId !== id) next.add(rootId);
      }
    }
  } else {
    next.add(id);
  }
  return next;
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
  onDeleteMany: (ids: string[]) => Promise<void>;
  onToggleScope: (target: Target) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
}

/**
 * Build a throwaway IP target for a host node that has no tracked IP row of its
 * own (a "resolution-only" IP, discovered only via some domain's `real_ip`, or
 * the synthetic "unresolved" bucket). The empty `id` makes
 * `useTargetSurfaceData` short-circuit to empty (no IPC); scope mirrors whether
 * any resolving domain is in scope. Lets such nodes reuse the same workbench.
 */
function makeSyntheticHostTarget(host: OrgTreeNode, domains: Target[]): Target {
  return {
    id: "",
    name: host.name,
    type: "ip",
    value: host.name,
    tags: [],
    notes: "",
    scope: domains.some((d) => d.scope === "in") ? "in" : "out",
    status: "new",
    grp: "",
    owner: "",
    time_window_start: null,
    time_window_end: null,
    organization_id: null,
    source: "resolved",
    parent_id: null,
    real_ip: "",
    cdn_waf: "",
    http_title: "",
    http_status: null,
    webserver: "",
    os_info: "",
    content_type: "",
    created_at: 0,
    updated_at: 0,
    ports: [],
  };
}

export function TargetGroupedView({
  targets,
  t,
  onAdd,
  onBatchAdd,
  onDelete,
  onDeleteMany,
  onToggleScope,
  onUpdateNotes,
}: TargetGroupedViewProps) {
  const [orgs, setOrgs] = useState<Organization[]>([]);
  const [orgLoading, setOrgLoading] = useState(true);
  const [orgError, setOrgError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [editingTargetId, setEditingTargetId] = useState<string | null>(null);
  const [newEngagementOpen, setNewEngagementOpen] = useState(false);
  const [newEngagementMode, setNewEngagementMode] = useState<EngagementMode>("customer_targets");
  const [selectedOrgId, setSelectedOrgId] = useState<string | null>(null);
  const [selectedTargetId, setSelectedTargetId] = useState<string | null>(null);
  // The selected synthetic host/bucket node id. When set (and no target is
  // drilled in), the right panel shows that IP's `TargetSurfaceWorkbench`, whose
  // Surface tab lists the domains that resolve to it.
  const [selectedHostId, setSelectedHostId] = useState<string | null>(null);
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
  const [hydratingOrgId, setHydratingOrgId] = useState<string | null>(null);
  const [hydratingAction, setHydratingAction] = useState<AssetIntelOrgActionKind | null>(null);
  const [hydrateRuns, setHydrateRuns] = useState<Record<string, AssetIntelRun>>({});
  const [hydrateErrors, setHydrateErrors] = useState<Record<string, string>>({});
  const [hydrateActivity, setHydrateActivity] = useState<Record<string, HydrateActivity>>({});
  const [organizationReconRuns, setOrganizationReconRuns] = useState<
    Record<string, OrganizationReconRunSnapshot>
  >({});
  const [organizationReconErrors, setOrganizationReconErrors] = useState<Record<string, string>>(
    {}
  );
  const [assetProviders, setAssetProviders] = useState<assetIntel.AssetIntelProviderDescriptor[]>(
    []
  );

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

  useEffect(() => {
    let cancelled = false;
    assetIntel
      .listProviders()
      .then((providers) => {
        if (!cancelled) setAssetProviders(providers);
      })
      .catch(() => {
        if (!cancelled) setAssetProviders([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    organizationRecon
      .listenStream((run) => {
        if (cancelled) return;
        setOrganizationReconRuns((prev) => ({
          ...prev,
          [run.organizationId]: applyOrganizationReconEvent(prev[run.organizationId], run),
        }));
      })
      .then((next) => {
        if (cancelled) next();
        else unlisten = next;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Live-refresh the org tree when the AI writes orgs (scoping:
  // manage_organizations / recon_discover_subsidiaries) or on the umbrella
  // `targets-changed` signal (also fired when the user switches into the Target
  // view — see TargetPanel). Previously the tree only reloaded on a
  // targets.length change, so AI-created orgs never appeared until a manual
  // refresh or the 15s target poll.
  useEffect(() => {
    const ORG_WRITE_TOOLS = new Set(["manage_organizations", "recon_discover_subsidiaries"]);
    const unlistenAi = onEvent("ai-event", (payload) => {
      const p = payload as { type?: string; tool_name?: string };
      if (p.type === "tool_result" && p.tool_name && ORG_WRITE_TOOLS.has(p.tool_name)) {
        refreshOrgs();
      }
    });
    const unlistenChanged = onCustomEvent("targets-changed", () => refreshOrgs());
    return () => {
      runTauriUnlistenFromPromise(unlistenAi);
      runTauriUnlistenFromPromise(unlistenChanged);
    };
  }, [refreshOrgs]);

  const orgById = useMemo(() => new Map(orgs.map((org) => [org.id, org])), [orgs]);
  const visibleTargets = useMemo(
    () =>
      targets.filter((target) => {
        if (!target.organization_id) return true;
        const org = orgById.get(target.organization_id);
        if (!org) return true;
        if (getEffectiveEngagementMode(org, orgs) !== "discover_assets") return true;
        return isOrganizationOwnedTarget(org, target.value);
      }),
    [orgById, orgs, targets]
  );
  const unassignedLabel = t("targets.unassigned");
  // Organization-only tree: targets stay on each node for counts/deletion, but
  // the sidebar does not render asset rows. IP/domain grouping lives in the
  // right-hand workspace so company navigation stays clean.
  const roots = useMemo(
    () => buildOrgTree(orgs, visibleTargets, unassignedLabel),
    [orgs, visibleTargets, unassignedLabel]
  );
  const rootIds = useMemo(() => new Set(roots.map((root) => root.id)), [roots]);
  const selectedOrg = useMemo(
    () => orgs.find((o) => o.id === selectedOrgId) ?? orgs[0] ?? null,
    [orgs, selectedOrgId]
  );
  const selectedMode = getEffectiveEngagementMode(selectedOrg ?? undefined, orgs);
  const selectedOrgNode = useMemo(
    () => (selectedOrg ? findOrgTreeNode(roots, selectedOrg.id) : null),
    [roots, selectedOrg]
  );
  const selectedTargets = useMemo(
    () =>
      selectedOrg
        ? visibleTargets.filter((target) => target.organization_id === selectedOrg.id)
        : [],
    [selectedOrg, visibleTargets]
  );
  const selectedSubtreeTargets = useMemo(
    () => (selectedOrgNode ? collectSubtreeTargets(selectedOrgNode) : selectedTargets),
    [selectedOrgNode, selectedTargets]
  );
  const emptyTargetSummary: TargetCountSummary = {
    ownTotal: 0,
    ownInScope: 0,
    subtreeTotal: 0,
    subtreeInScope: 0,
    descendantOrgCount: 0,
  };
  const selectedTargetSummary = selectedOrgNode
    ? summarizeTargetCounts(selectedOrgNode)
    : emptyTargetSummary;
  const selectedTarget = useMemo(
    () => visibleTargets.find((target) => target.id === selectedTargetId) ?? null,
    [selectedTargetId, visibleTargets]
  );
  const selectedTargetRelatedDomains = useMemo(() => {
    if (!selectedTarget || selectedTarget.type !== "ip") return [];
    const ip = selectedTarget.value.trim();
    if (!ip) return [];
    return visibleTargets
      .filter(
        (target) =>
          target.id !== selectedTarget.id &&
          !isIpLiteralTargetValue(target) &&
          target.organization_id === selectedTarget.organization_id &&
          (target.real_ip ?? "").trim() === ip
      )
      .sort((a, b) => a.value.localeCompare(b.value, "zh"));
  }, [selectedTarget, visibleTargets]);
  const selectedTargetRelatedWebTargets = useMemo(() => {
    if (!selectedTarget || selectedTarget.type !== "ip") return selectedTargetRelatedDomains;
    const ip = selectedTarget.value.trim();
    if (!ip) return selectedTargetRelatedDomains;
    return visibleTargets
      .filter((target) => {
        if (
          target.id === selectedTarget.id ||
          target.organization_id !== selectedTarget.organization_id
        ) {
          return false;
        }
        if (selectedTargetRelatedDomains.some((related) => related.id === target.id)) return true;
        if (target.type !== "url") return false;
        if ((target.real_ip ?? "").trim() === ip) return true;
        return isIpLiteralTargetValue(target);
      })
      .sort((a, b) => a.value.localeCompare(b.value, "zh"));
  }, [selectedTarget, selectedTargetRelatedDomains, visibleTargets]);

  // Flatten the synthetic host/bucket nodes for O(1) lookup of the selected one,
  // so the right-hand workbench can show its IP + the domains resolving to it.
  const hostNodeById = useMemo(() => {
    const map = new Map<string, OrgTreeNode>();
    const walk = (nodes: OrgTreeNode[]) => {
      for (const node of nodes) {
        if (node.kind === "host" || node.kind === "bucket") map.set(node.id, node);
        walk(node.children);
      }
    };
    walk(roots);
    return map;
  }, [roots]);
  const selectedHost = selectedHostId ? (hostNodeById.get(selectedHostId) ?? null) : null;

  // The host panel reuses `TargetSurfaceWorkbench`. Its subject is the IP's own
  // tracked target row when one exists; for a "resolution-only" IP (only
  // referenced via a domain's real_ip, never scanned itself) we synthesise a
  // throwaway IP target with an empty id so the surface hook short-circuits.
  const hostIpTarget = useMemo(
    () =>
      selectedHost
        ? (selectedHost.targets.find(
            (target) => target.type === "ip" && target.value === selectedHost.name
          ) ?? null)
        : null,
    [selectedHost]
  );
  const hostDomains = useMemo(
    () =>
      selectedHost
        ? selectedHost.targets
            .filter((target) => target.id !== hostIpTarget?.id && !isIpLiteralTargetValue(target))
            .sort((a, b) => a.value.localeCompare(b.value))
        : [],
    [selectedHost, hostIpTarget]
  );
  const hostWebTargets = useMemo(
    () =>
      selectedHost
        ? selectedHost.targets
            .filter(
              (target) =>
                target.id !== hostIpTarget?.id &&
                (hostDomains.some((domain) => domain.id === target.id) ||
                  (target.type === "url" && isIpLiteralTargetValue(target)))
            )
            .sort((a, b) => a.value.localeCompare(b.value))
        : [],
    [selectedHost, hostIpTarget, hostDomains]
  );
  const hostWorkbenchTarget = useMemo<Target | null>(() => {
    if (!selectedHost) return null;
    return hostIpTarget ?? makeSyntheticHostTarget(selectedHost, hostDomains);
  }, [selectedHost, hostIpTarget, hostDomains]);

  // Drop a host id that no longer exists (e.g. after the tree rebuilds on a
  // data refresh) so the right panel doesn't dangle.
  useEffect(() => {
    if (selectedHostId && !hostNodeById.has(selectedHostId)) setSelectedHostId(null);
  }, [selectedHostId, hostNodeById]);

  useEffect(() => {
    if (!selectedOrgId && orgs.length > 0) {
      setSelectedOrgId(orgs[0].id);
      return;
    }
    if (selectedOrgId && !orgs.some((org) => org.id === selectedOrgId)) {
      setSelectedOrgId(orgs[0]?.id ?? null);
    }
  }, [orgs, selectedOrgId]);

  useEffect(() => {
    if (selectedTargetId && !targets.some((target) => target.id === selectedTargetId)) {
      setSelectedTargetId(null);
    }
  }, [selectedTargetId, targets]);

  useEffect(() => {
    // Skip while drilling in from a host (IP) panel: there the selected target
    // is scoped to the host, not to `selectedOrg`, so a cross-org mismatch is
    // expected and must not deselect it.
    if (
      !selectedHostId &&
      selectedOrg &&
      selectedTarget &&
      selectedTarget.organization_id &&
      selectedTarget.organization_id !== selectedOrg.id
    ) {
      setSelectedTargetId(null);
    }
  }, [selectedOrg, selectedTarget, selectedHostId]);

  const openHostGroup = useCallback(
    (group: TargetAssetGroup) => {
      if (!group.host) {
        const firstTarget = group.targets[0];
        if (firstTarget) {
          setSelectedHostId(null);
          setSelectedTargetId(firstTarget.id);
        }
        return;
      }

      const groupTargetIds = new Set(group.targets.map((target) => target.id));
      let bestHost: OrgTreeNode | null = null;
      let bestOverlap = 0;
      for (const hostNode of hostNodeById.values()) {
        if (hostNode.kind !== "host" || hostNode.name !== group.host) continue;
        const overlap = hostNode.targets.reduce(
          (count, target) => count + (groupTargetIds.has(target.id) ? 1 : 0),
          0
        );
        if (overlap > bestOverlap) {
          bestHost = hostNode;
          bestOverlap = overlap;
        }
      }

      if (bestHost) {
        setSelectedTargetId(null);
        setSelectedHostId(bestHost.id);
        return;
      }

      const ipTarget =
        group.ipTarget ??
        group.targets.find((target) => target.type === "ip" && target.value.trim() === group.host);
      if (ipTarget) {
        setSelectedHostId(null);
        setSelectedTargetId(ipTarget.id);
      }
    },
    [hostNodeById]
  );

  const toggleCollapse = useCallback(
    (id: string) => {
      setCollapsed((prev) => toggleCollapsedSet(prev, id, rootIds));
    },
    [rootIds]
  );

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

  const handleRunAssetIntel = useCallback(
    async (org: Organization, action: AssetIntelOrgActionKind) => {
      const engagement = getEngagementRecord(org);
      setSelectedOrgId(org.id);
      setWorkspaceTab("activity");
      setHydratingOrgId(org.id);
      setHydratingAction(action);
      setHydrateErrors((prev) => {
        const next = { ...prev };
        delete next[org.id];
        return next;
      });
      setHydrateActivity((prev) => ({
        ...prev,
        [org.id]: { runId: null, providers: {}, providerOrder: [] },
      }));

      // Subscribe to streaming events before kicking off asset intel so we
      // don't miss the first `provider_started` payload. Updates are routed
      // into `hydrateActivity[org.id]` so the Activity panel can render the
      // run as it progresses, not just when the IPC promise resolves.
      const unlisten = await assetIntel.listenStream((event) => {
        setHydrateActivity((prev) => {
          const current = prev[org.id] ?? {
            runId: null,
            providers: {},
            providerOrder: [],
          };
          const next = applyStreamEvent(current, event);
          return { ...prev, [org.id]: next };
        });
      });

      try {
        const config =
          action === "hydrate_subsidiaries"
            ? buildDiscoveryHydrateConfigFromEngagement(engagement)
            : buildHydrateConfigFromEngagement(engagement);
        let run: AssetIntelRun | null = null;
        let nextTab: WorkspaceTab | null = null;

        if (action === "hydrate_subsidiaries") {
          run = await assetIntel.hydrateSubsidiaries({
            organizationId: org.id,
            companyName: org.name,
            config,
          });
          nextTab = getNextWorkspaceTabAfterAssetIntelRun(action, run);
        } else {
          run = await assetIntel.enrichOrganization({
            organizationId: org.id,
            config,
          });
          nextTab = getNextWorkspaceTabAfterAssetIntelRun(action, run);
        }

        if (run) {
          setHydrateRuns((prev) => ({ ...prev, [org.id]: run }));
        }
        setHydrateActivity((prev) => {
          if (!prev[org.id]) return prev;
          const next = { ...prev };
          delete next[org.id];
          return next;
        });
        await refreshOrgs();
        if (nextTab !== null) {
          setWorkspaceTab(nextTab);
        }
      } catch (error) {
        setHydrateErrors((prev) => ({ ...prev, [org.id]: String(error) }));
      } finally {
        unlisten();
        setHydratingOrgId(null);
        setHydratingAction(null);
      }
    },
    [refreshOrgs]
  );

  const handleRunOrganizationRecon = useCallback(async (org: Organization) => {
    setSelectedOrgId(org.id);
    setWorkspaceTab("activity");
    setOrganizationReconErrors((prev) => {
      const next = { ...prev };
      delete next[org.id];
      return next;
    });
    try {
      const run = await organizationRecon.startRun({
        organizationId: org.id,
        allowExternal: true,
        allowActive: true,
      });
      setOrganizationReconRuns((prev) => ({ ...prev, [org.id]: run }));
    } catch (error) {
      setOrganizationReconErrors((prev) => ({ ...prev, [org.id]: String(error) }));
    }
  }, []);

  const handleExportOrganizationReconAssets = useCallback(
    async (org: Organization, run?: OrganizationReconRunSnapshot) => {
      try {
        const outputPath = await save({
          defaultPath: suggestedReconAssetsFilename(org.name, run?.runId),
          filters: [{ name: "Excel workbook", extensions: ["xlsx"] }],
        });
        if (!outputPath) return;
        const result = run
          ? await organizationRecon.exportAssets(run.runId, outputPath)
          : await organizationRecon.exportCurrentAssets(org.id, outputPath);
        const sizeKb = Math.max(1, Math.round(Number(result.bytes) / 1024));
        notify.success("Recon assets exported", {
          message: `${sizeKb} KB written to ${result.outputPath}`,
        });
      } catch (error) {
        notify.error("Recon asset export failed", { message: String(error) });
      }
    },
    []
  );

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
      // Deleting an org cascades to its descendant orgs AND all their targets
      // (DB FKs, migration 20260614000002). Warn with the blast radius up front.
      const { subOrgCount, targetCount } = countOrgDeletionImpact(orgs, targets, id);
      const confirmMsg = t("organizations.deleteConfirm")
        .replace("{{name}}", name)
        .replace("{{subOrgCount}}", String(subOrgCount))
        .replace("{{targetCount}}", String(targetCount));
      if (!confirm(confirmMsg)) return;
      try {
        const projectPath = getProjectPath();
        if (!projectPath) throw new Error("No active project is selected");
        await orgsApi.deleteOrganization({ id, projectPath });
        await refreshOrgs();
        // Targets are cascade-deleted server-side; reload them so the UI drops
        // the removed rows instead of leaving them stale.
        sendCustomEvent("targets-changed").catch(() => {});
      } catch (e) {
        alert(String(e));
      }
    },
    [orgs, targets, refreshOrgs, t]
  );

  const handleDeleteNodeTargets = useCallback(
    async (node: OrgTreeNode) => {
      // Synthetic groups (unassigned / unresolved bucket / IP host) have no DB
      // row of their own, so "delete this group" means deleting the real targets
      // it contains (recursively). Warn with the exact blast radius first.
      const victims = collectSubtreeTargets(node);
      if (victims.length === 0) return;
      const confirmMsg = t("targets.deleteBucketConfirm")
        .replace("{{name}}", node.name)
        .replace("{{count}}", String(victims.length));
      if (!confirm(confirmMsg)) return;
      await onDeleteMany(victims.map((target) => target.id));
      // Drop a selection pointing at the now-emptied group so the right panel
      // doesn't dangle (host ids are also pruned by the tree-rebuild effect).
      if (selectedHostId === node.id) setSelectedHostId(null);
    },
    [onDeleteMany, t, selectedHostId]
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
          className="flex items-center gap-1.5 rounded border border-green-500/25 bg-green-500/10 px-2 py-1 text-xs font-medium text-green-300 transition-colors hover:bg-green-500/15 hover:text-green-200"
          onClick={() => openNewEngagement("customer_targets")}
        >
          <Plus className="w-3 h-3" />
          {translateWithFallback(t, "targetWorkspace.actions.newEngagement", "New Engagement")}
        </button>
        <span className="text-[10px] text-muted-foreground/60 tabular-nums">
          {translateWithFallback(
            t,
            "targetWorkspace.metrics.orgTargetCount",
            "{{orgs}} orgs · {{targets}} targets"
          )
            .replace("{{orgs}}", String(orgs.length))
            .replace("{{targets}}", String(targets.length))}
        </span>
      </div>

      {addingChildTo === ROOT_PARENT_KEY && (
        <InlineCreateOrgForm
          parentId={null}
          depth={0}
          t={t}
          orgFormName={orgFormName}
          setOrgFormName={setOrgFormName}
          orgFormOwner={orgFormOwner}
          setOrgFormOwner={setOrgFormOwner}
          submitting={submitting}
          inlineError={inlineError}
          handleCreateOrg={handleCreateOrg}
          closeAllEditors={closeAllEditors}
        />
      )}

      {roots.length === 0 && targets.length === 0 ? (
        <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground px-6 text-center">
          <Building2 className="w-8 h-8 mb-2 opacity-30" />
          <p className="text-xs">{t("targets.treeEmpty")}</p>
          <p className="text-[10px] text-muted-foreground/60 mt-1">{t("targets.treeEmptyHint")}</p>
          <div className="mt-4 grid grid-cols-1 sm:grid-cols-3 gap-2 w-full max-w-2xl">
            <button
              type="button"
              className="rounded-lg border border-green-500/25 bg-green-500/5 px-3 py-3 text-left transition-colors hover:bg-green-500/10"
              onClick={() => openNewEngagement("customer_targets")}
            >
              <Shield className="w-4 h-4 text-green-400 mb-2" />
              <p className="text-xs text-foreground">
                {translateWithFallback(t, "targetWorkspace.empty.importTargets", "Import targets")}
              </p>
              <p className="text-[10px] text-muted-foreground/70 mt-1">
                {translateWithFallback(
                  t,
                  "targetWorkspace.empty.importTargetsDesc",
                  "Customer provided scope list."
                )}
              </p>
            </button>
            <button
              type="button"
              className="rounded-lg border border-blue-500/25 bg-blue-500/5 px-3 py-3 text-left transition-colors hover:bg-blue-500/10"
              onClick={() => openNewEngagement("discover_assets")}
            >
              <Network className="w-4 h-4 text-blue-400 mb-2" />
              <p className="text-xs text-foreground">
                {translateWithFallback(
                  t,
                  "targetWorkspace.empty.discoverAssets",
                  "Discover assets"
                )}
              </p>
              <p className="text-[10px] text-muted-foreground/70 mt-1">
                {translateWithFallback(
                  t,
                  "targetWorkspace.empty.discoverAssetsDesc",
                  "Create org and prepare ASM flow."
                )}
              </p>
            </button>
            <button
              type="button"
              className="rounded-lg border border-border/40 bg-background/20 px-3 py-3 text-left transition-colors hover:bg-muted/20"
              onClick={() => handleStartAddChild(null)}
            >
              <Building2 className="w-4 h-4 text-muted-foreground mb-2" />
              <p className="text-xs text-foreground">
                {translateWithFallback(t, "targetWorkspace.empty.profileOnly", "Org profile only")}
              </p>
              <p className="text-[10px] text-muted-foreground/70 mt-1">
                {translateWithFallback(
                  t,
                  "targetWorkspace.empty.profileOnlyDesc",
                  "Create a customer record."
                )}
              </p>
            </button>
          </div>
        </div>
      ) : (
        <div className="flex-1 min-h-0 grid grid-cols-[minmax(280px,0.72fr)_minmax(380px,1.28fr)]">
          <OrgTreeSidebar
            roots={roots}
            collapsed={collapsed}
            toggleCollapse={toggleCollapse}
            editingOrgId={editingOrgId}
            orgs={orgs}
            selectedOrgId={selectedOrgId}
            setSelectedOrgId={setSelectedOrgId}
            setSelectedTargetId={setSelectedTargetId}
            setWorkspaceTab={setWorkspaceTab}
            addingTargetTo={addingTargetTo}
            addingChildTo={addingChildTo}
            hydratingOrgId={hydratingOrgId}
            hydratingAction={hydratingAction}
            handleStartAddTarget={handleStartAddTarget}
            handleStartAddChild={handleStartAddChild}
            handleStartEditOrg={handleStartEditOrg}
            handleDeleteOrg={handleDeleteOrg}
            handleDeleteNodeTargets={handleDeleteNodeTargets}
            handleRunAssetIntel={handleRunAssetIntel}
            editingTargetId={editingTargetId}
            setEditingTargetId={setEditingTargetId}
            selectedTargetId={selectedTargetId}
            selectedHostId={selectedHostId}
            setSelectedHostId={setSelectedHostId}
            onToggleScope={onToggleScope}
            onDelete={onDelete}
            onUpdateNotes={onUpdateNotes}
            orgFormName={orgFormName}
            setOrgFormName={setOrgFormName}
            orgFormOwner={orgFormOwner}
            setOrgFormOwner={setOrgFormOwner}
            targetFormValue={targetFormValue}
            setTargetFormValue={setTargetFormValue}
            targetFormName={targetFormName}
            setTargetFormName={setTargetFormName}
            submitting={submitting}
            inlineError={inlineError}
            handleCreateOrg={handleCreateOrg}
            handleAddTargetSubmit={handleAddTargetSubmit}
            handleSaveEditOrg={handleSaveEditOrg}
            closeAllEditors={closeAllEditors}
            t={t}
          />
          <div className="min-h-0 overflow-hidden bg-background/20">
            {selectedTarget ? (
              <TargetSurfaceWorkbench
                target={selectedTarget}
                onUpdateNotes={onUpdateNotes}
                relatedDomains={selectedTargetRelatedDomains}
                relatedWebTargets={selectedTargetRelatedWebTargets}
                onBack={() => setSelectedTargetId(null)}
                backLabel={
                  selectedHost
                    ? t("targets.backToHost")
                    : translateWithFallback(t, "targets.backToOrganization", "Back to organization")
                }
              />
            ) : selectedHost && hostWorkbenchTarget ? (
              <TargetSurfaceWorkbench
                target={hostWorkbenchTarget}
                onUpdateNotes={hostIpTarget ? onUpdateNotes : () => {}}
                relatedDomains={hostDomains}
                relatedWebTargets={hostWebTargets}
              />
            ) : (
              <OrgWorkspacePanel
                selectedOrg={selectedOrg}
                selectedMode={selectedMode}
                selectedTargets={selectedTargets}
                selectedSubtreeTargets={selectedSubtreeTargets}
                targetSummary={selectedTargetSummary}
                t={t}
                workspaceTab={workspaceTab}
                setWorkspaceTab={setWorkspaceTab}
                assetProviders={assetProviders}
                hydrateRuns={hydrateRuns}
                hydrateErrors={hydrateErrors}
                hydrateActivity={hydrateActivity}
                organizationReconRuns={organizationReconRuns}
                organizationReconErrors={organizationReconErrors}
                hydratingOrgId={hydratingOrgId}
                hydratingAction={hydratingAction}
                setEditingTargetId={setEditingTargetId}
                setSelectedTargetId={setSelectedTargetId}
                openHostGroup={openHostGroup}
                handleRunAssetIntel={handleRunAssetIntel}
                handleRunOrganizationRecon={handleRunOrganizationRecon}
                handleExportOrganizationReconAssets={handleExportOrganizationReconAssets}
              />
            )}
          </div>
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
