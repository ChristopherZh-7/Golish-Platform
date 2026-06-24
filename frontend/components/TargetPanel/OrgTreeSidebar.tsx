/**
 * `OrgTreeSidebar` — the left-hand org/target tree.
 *
 * Extracted from `TargetGroupedView.tsx`'s `renderNode` (+ `renderOrgActionButton`).
 * The node row is recursive (`OrgTreeNodeRow` renders its own children) and
 * delegates leaves to `TargetTreeRow` and the inline create/edit forms. All
 * tree + form state stays owned by `TargetGroupedView`; this is a controlled
 * view, so the full handler/state surface is threaded through `OrgTreeProps`.
 */

import {
  Building2,
  ChevronDown,
  Crosshair,
  FolderOpen,
  Info,
  Loader2,
  Network,
  Pencil,
  Shield,
  Trash2,
  Wifi,
} from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import {
  ENGAGEMENT_BADGES,
  getEffectiveEngagementMode,
  getOrgActionModel,
  isAssetIntelOrgAction,
} from "@/lib/target-panel/engagement";
import { countAllTargets, type OrgTreeNode, UNASSIGNED_KEY } from "@/lib/target-panel/org-tree";
import type {
  AssetIntelOrgActionKind,
  OrgActionItem,
  WorkspaceTab,
} from "@/lib/target-panel/types";
import { cn } from "@/lib/utils";
import { InlineAddTargetForm, InlineCreateOrgForm, InlineOrgEditForm } from "./InlineOrgForms";
import { TargetTreeRow } from "./TargetTreeRow";

interface OrgTreeProps {
  collapsed: Set<string>;
  toggleCollapse: (id: string) => void;
  editingOrgId: string | null;
  orgs: Organization[];
  selectedOrgId: string | null;
  setSelectedOrgId: Dispatch<SetStateAction<string | null>>;
  setSelectedTargetId: Dispatch<SetStateAction<string | null>>;
  setWorkspaceTab: Dispatch<SetStateAction<WorkspaceTab>>;
  addingTargetTo: string | null;
  addingChildTo: string | null;
  hydratingOrgId: string | null;
  hydratingAction: AssetIntelOrgActionKind | null;
  handleStartAddTarget: (orgId: string) => void;
  handleStartAddChild: (parentId: string | null) => void;
  handleStartEditOrg: (node: OrgTreeNode) => void;
  handleDeleteOrg: (id: string, name: string) => void;
  // Bulk-delete the real targets behind a synthetic group (unassigned /
  // unresolved bucket / IP host) — those nodes have no DB row to "delete".
  handleDeleteNodeTargets: (node: OrgTreeNode) => void;
  handleRunAssetIntel: (org: Organization, action: AssetIntelOrgActionKind) => void;
  editingTargetId: string | null;
  setEditingTargetId: Dispatch<SetStateAction<string | null>>;
  selectedTargetId: string | null;
  // IP-centric view: the currently selected synthetic host/bucket node id. Host
  // & bucket nodes are leaves here — clicking one opens that IP's workbench on
  // the right (whose Surface tab lists the domains) instead of nesting rows.
  selectedHostId: string | null;
  setSelectedHostId: Dispatch<SetStateAction<string | null>>;
  onToggleScope: (target: Target) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
  orgFormName: string;
  setOrgFormName: Dispatch<SetStateAction<string>>;
  orgFormOwner: string;
  setOrgFormOwner: Dispatch<SetStateAction<string>>;
  targetFormValue: string;
  setTargetFormValue: Dispatch<SetStateAction<string>>;
  targetFormName: string;
  setTargetFormName: Dispatch<SetStateAction<string>>;
  submitting: boolean;
  inlineError: string | null;
  handleCreateOrg: (parentId: string | null) => void;
  handleAddTargetSubmit: (orgId: string) => void;
  handleSaveEditOrg: () => void;
  closeAllEditors: () => void;
  showAssetsInTree?: boolean;
  t: (key: string) => string;
}

// Assets (`node.targets`) live in their own collapsible sub-group so an org with
// a huge asset list can be folded without also hiding its subsidiaries
// (`node.children`, which stay under the separate org-level toggle). Groups
// larger than this start collapsed; the preview cap keeps even an expanded group
// from flooding the sidebar until the user asks for the rest.
const ASSET_GROUP_DEFAULT_COLLAPSE = 20;
const ASSET_PREVIEW_LIMIT = 15;

function OrgTreeNodeRow(props: { node: OrgTreeNode; depth: number } & OrgTreeProps) {
  const {
    node,
    depth,
    collapsed,
    toggleCollapse,
    editingOrgId,
    orgs,
    selectedOrgId,
    setSelectedOrgId,
    setSelectedTargetId,
    setWorkspaceTab,
    addingTargetTo,
    addingChildTo,
    hydratingOrgId,
    hydratingAction,
    handleStartAddTarget,
    handleStartAddChild,
    handleStartEditOrg,
    handleDeleteOrg,
    handleDeleteNodeTargets,
    handleRunAssetIntel,
    editingTargetId,
    setEditingTargetId,
    selectedTargetId,
    selectedHostId,
    setSelectedHostId,
    onToggleScope,
    onDelete,
    onUpdateNotes,
    orgFormName,
    setOrgFormName,
    orgFormOwner,
    setOrgFormOwner,
    targetFormValue,
    setTargetFormValue,
    targetFormName,
    setTargetFormName,
    submitting,
    inlineError,
    handleCreateOrg,
    handleAddTargetSubmit,
    handleSaveEditOrg,
    closeAllEditors,
    showAssetsInTree = false,
    t,
  } = props;

  const renderOrgActionButton = (action: OrgActionItem, node: OrgTreeNode) => {
    const runAction = (e: React.MouseEvent) => {
      e.stopPropagation();
      switch (action.kind) {
        case "import_targets":
          handleStartAddTarget(node.id);
          break;
        case "hydrate_subsidiaries":
        case "enrich_organization":
          {
            const org = orgs.find((item) => item.id === node.id);
            if (org && hydratingOrgId !== node.id) void handleRunAssetIntel(org, action.kind);
          }
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

    const isAssetIntelAction = isAssetIntelOrgAction(action.kind);
    const isAnyAssetIntelRunOnNode = isAssetIntelAction && hydratingOrgId === node.id;
    const isHydrating = isAnyAssetIntelRunOnNode && hydratingAction === action.kind;
    const icon = isHydrating ? (
      <Loader2 className="w-3 h-3 animate-spin" />
    ) : action.kind === "hydrate_subsidiaries" ? (
      <Network className="w-3 h-3" />
    ) : action.kind === "enrich_organization" ? (
      <Wifi className="w-3 h-3" />
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
          action.kind === "hydrate_subsidiaries" && "hover:text-blue-400",
          action.kind === "enrich_organization" && "hover:text-cyan-400",
          action.kind === "import_targets" && "hover:text-green-400",
          action.kind === "review_scope" && "hover:text-amber-400",
          action.kind === "choose_next_step" && "hover:text-accent",
          action.kind === "add_child" && "hover:text-accent"
        )}
        onClick={runAction}
        disabled={isAnyAssetIntelRunOnNode}
        title={action.label}
      >
        {icon}
      </button>
    );
  };

  const counts = countAllTargets(node);
  const isUnassigned = node.id === UNASSIGNED_KEY;
  // Synthetic IP-centric nodes (`buildHostTree`): host = an IP group, bucket =
  // catch-all (e.g. unresolved). Only real org nodes carry org-level actions.
  const kind = node.kind ?? "org";
  const isOrg = kind === "org";
  const isHost = kind === "host";
  const isBucket = kind === "bucket";
  const isSelectedOrg = selectedOrgId === node.id && isOrg && !isUnassigned;
  // Host & bucket nodes are selectable leaves in the IP view: clicking selects
  // them (→ that IP's workbench on the right) rather than expanding nested rows.
  const isLeafSelectable = isHost || isBucket;
  const isCollapsed = collapsed.has(node.id);

  // Asset sub-group state. Membership in `collapsed` flips the size-based
  // default, so an untouched large group starts folded while an untouched small
  // group starts open; `assets-more:` overrides the preview cap on demand.
  const assetKey = `assets:${node.id}`;
  const assetMoreKey = `assets-more:${node.id}`;
  const totalAssets = node.targets.length;
  const assetsDefaultCollapsed = totalAssets > ASSET_GROUP_DEFAULT_COLLAPSE;
  const assetsCollapsed = collapsed.has(assetKey)
    ? !assetsDefaultCollapsed
    : assetsDefaultCollapsed;
  const showAllAssets = collapsed.has(assetMoreKey);
  const visibleAssets = showAllAssets ? node.targets : node.targets.slice(0, ASSET_PREVIEW_LIMIT);
  const hiddenAssetCount = totalAssets - visibleAssets.length;
  const isEditingThis = editingOrgId === node.id;
  const orgRow = orgs.find((o) => o.id === node.id);
  const engagementMode = getEffectiveEngagementMode(orgRow, orgs);
  const badge = engagementMode ? ENGAGEMENT_BADGES[engagementMode] : null;
  const showModeBadge = badge && (depth === 0 || selectedOrgId === node.id);
  const actionModel = getOrgActionModel(engagementMode, {
    isChild: Boolean(orgRow?.parent_id),
  });

  // The flat list of asset rows (+ "show more"), shared by both the org-level
  // "资产" sub-group and the bare host/bucket rendering below.
  const assetRows = (
    <>
      {visibleAssets.map((target) => (
        <TargetTreeRow
          key={target.id}
          target={target}
          t={t}
          editingTargetId={editingTargetId}
          selectedTargetId={selectedTargetId}
          setSelectedTargetId={setSelectedTargetId}
          setSelectedOrgId={setSelectedOrgId}
          setEditingTargetId={setEditingTargetId}
          onToggleScope={onToggleScope}
          onDelete={onDelete}
          onUpdateNotes={onUpdateNotes}
        />
      ))}
      {totalAssets > ASSET_PREVIEW_LIMIT && (
        <button
          type="button"
          onClick={() => toggleCollapse(assetMoreKey)}
          className="px-2 py-0.5 text-left text-[10px] text-accent/80 hover:text-accent"
        >
          {showAllAssets
            ? t("targets.assetsShowLess")
            : `${t("targets.assetsShowMore")} (+${hiddenAssetCount})`}
        </button>
      )}
    </>
  );

  return (
    <div key={node.id}>
      {isEditingThis ? (
        <InlineOrgEditForm
          depth={depth}
          t={t}
          orgFormName={orgFormName}
          setOrgFormName={setOrgFormName}
          orgFormOwner={orgFormOwner}
          setOrgFormOwner={setOrgFormOwner}
          submitting={submitting}
          handleSaveEditOrg={handleSaveEditOrg}
          closeAllEditors={closeAllEditors}
        />
      ) : (
        <div
          className={cn(
            "flex items-center gap-1 border-l-2 border-transparent px-2 py-1 hover:bg-muted/15 transition-colors group rounded-r",
            isSelectedOrg &&
              "border-accent bg-accent/12 shadow-[inset_0_0_0_1px_hsl(var(--accent)/0.18)]",
            isLeafSelectable && selectedHostId === node.id && "bg-muted/20"
          )}
          style={{ paddingLeft: `${8 + depth * 16}px` }}
        >
          <button
            type="button"
            onClick={() => {
              if (isLeafSelectable) {
                // IP / bucket leaf: select it so the right panel lists its
                // member domains/URLs; no nested rows to expand.
                setSelectedHostId(node.id);
                setSelectedTargetId(null);
                return;
              }
              if (isOrg && !isUnassigned) {
                setSelectedOrgId(node.id);
                setSelectedTargetId(null);
                setSelectedHostId(null);
              }
              toggleCollapse(node.id);
            }}
            className="flex items-center gap-2 flex-1 text-left min-w-0"
          >
            {!isLeafSelectable && (
              <ChevronDown
                className={cn(
                  "w-3 h-3 text-muted-foreground/60 transition-transform flex-shrink-0",
                  isCollapsed && "-rotate-90"
                )}
              />
            )}
            {isHost ? (
              <Network className="w-3 h-3 text-blue-400/70 flex-shrink-0" />
            ) : !isOrg || isUnassigned ? (
              <FolderOpen className="w-3 h-3 text-muted-foreground/60 flex-shrink-0" />
            ) : (
              <Building2
                className={cn(
                  "w-3 h-3 flex-shrink-0",
                  isSelectedOrg ? "text-accent" : "text-accent/70"
                )}
              />
            )}
            <span
              className={cn(
                "min-w-0 truncate text-[11px] font-medium",
                isSelectedOrg ? "text-accent" : "text-foreground"
              )}
            >
              {node.name}
            </span>
            <span
              className={cn(
                "flex-shrink-0 text-[10px] tabular-nums",
                isSelectedOrg ? "text-accent/80" : "text-muted-foreground/55"
              )}
            >
              {counts.total}
            </span>
            {counts.inScope > 0 && (
              <span className="flex-shrink-0 whitespace-nowrap rounded bg-green-500/10 px-1 py-0.5 text-[9px] text-green-400">
                {counts.inScope} in
              </span>
            )}
            {showModeBadge && (
              <span
                className={cn(
                  "flex-shrink-0 whitespace-nowrap rounded px-1 py-0.5 text-[9px]",
                  badge.className
                )}
              >
                {badge.label}
              </span>
            )}
            {!isUnassigned && node.children.length > 0 && (
              <span className="inline-flex flex-shrink-0 items-center whitespace-nowrap text-[9px] text-muted-foreground/50">
                · {node.children.length} sub
              </span>
            )}
          </button>

          {isOrg && (
            <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
              {isUnassigned ? (
                <>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-accent"
                    onClick={() => handleStartAddTarget(node.id)}
                    title={t("targets.addTarget")}
                  >
                    <Crosshair className="w-3 h-3" />
                  </button>
                  {counts.total > 0 && (
                    <button
                      type="button"
                      className="p-1 rounded hover:bg-red-500/20 text-muted-foreground hover:text-red-400"
                      onClick={() => handleDeleteNodeTargets(node)}
                      title={t("targets.deleteBucket")}
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  )}
                </>
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
          )}

          {/* Synthetic IP host / catch-all bucket leaves carry no org actions,
              but still need a way to clear out their underlying targets. */}
          {isLeafSelectable && counts.total > 0 && (
            <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
              <button
                type="button"
                className="p-1 rounded hover:bg-red-500/20 text-muted-foreground hover:text-red-400"
                onClick={(e) => {
                  e.stopPropagation();
                  handleDeleteNodeTargets(node);
                }}
                title={t("targets.deleteBucket")}
              >
                <Trash2 className="w-3 h-3" />
              </button>
            </div>
          )}
        </div>
      )}

      {addingTargetTo === node.id && (
        <InlineAddTargetForm
          orgId={node.id}
          depth={depth + 1}
          t={t}
          targetFormValue={targetFormValue}
          setTargetFormValue={setTargetFormValue}
          targetFormName={targetFormName}
          setTargetFormName={setTargetFormName}
          submitting={submitting}
          inlineError={inlineError}
          handleAddTargetSubmit={handleAddTargetSubmit}
          closeAllEditors={closeAllEditors}
        />
      )}
      {addingChildTo === node.id && (
        <InlineCreateOrgForm
          parentId={node.id}
          depth={depth + 1}
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

      {!isCollapsed && (
        <div className="space-y-px">
          {showAssetsInTree && isOrg && node.targets.length > 0 && (
            // Org node: keep the collapsible "资产" sub-group so a large asset
            // list can be folded independently of the org's sub-orgs. Host /
            // bucket nodes intentionally render no nested rows — their domains
            // live in the right-hand workbench's Surface "domains" block instead.
            <div className="space-y-px">
              <button
                type="button"
                onClick={() => toggleCollapse(assetKey)}
                className="flex w-full items-center gap-1.5 rounded px-2 py-0.5 text-left text-muted-foreground/70 hover:bg-muted/10"
                style={{ paddingLeft: `${8 + (depth + 1) * 16}px` }}
              >
                <ChevronDown
                  className={cn(
                    "h-3 w-3 flex-shrink-0 transition-transform",
                    assetsCollapsed && "-rotate-90"
                  )}
                />
                <Crosshair className="h-3 w-3 flex-shrink-0" />
                <span className="text-[10px] font-medium">{t("targets.assetsGroup")}</span>
                <span className="text-[10px] tabular-nums text-muted-foreground/55">
                  {totalAssets}
                </span>
              </button>
              {!assetsCollapsed && (
                <div
                  className="space-y-px py-0.5"
                  style={{ paddingLeft: `${8 + (depth + 1) * 16}px` }}
                >
                  {assetRows}
                </div>
              )}
            </div>
          )}
          {node.children.map((child) => (
            <OrgTreeNodeRow key={child.id} {...props} node={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

export function OrgTreeSidebar({ roots, ...rest }: { roots: OrgTreeNode[] } & OrgTreeProps) {
  return (
    <div className="min-h-0 overflow-y-auto py-2 px-1 space-y-px border-r border-border/25">
      {roots.map((node) => (
        <OrgTreeNodeRow key={node.id} node={node} depth={0} {...rest} />
      ))}
    </div>
  );
}
