/**
 * Engagement-mode domain helpers for the Target panel.
 *
 * Pure logic extracted from `TargetGroupedView.tsx`: derive the per-org action
 * model + right-side workspace model from the engagement mode, read engagement
 * settings, and map them into asset-intel hydrate configs.
 */

import type { EngagementMode } from "@/components/TargetPanel/NewEngagementDialog";
import type { AssetIntelHydrateConfig } from "@/lib/api/asset-intel";
import type { Organization } from "@/lib/api/organizations";
import type {
  AssetIntelOrgActionKind,
  EngagementRecord,
  OrgActionItem,
  OrgActionKind,
} from "./types";

export const ENGAGEMENT_BADGES = {
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

export function isAssetIntelOrgAction(kind: OrgActionKind): kind is AssetIntelOrgActionKind {
  return kind === "hydrate_subsidiaries" || kind === "enrich_organization";
}

export function isAssetIntelOrgActionItem(
  action: OrgActionItem | undefined
): action is OrgActionItem & { kind: AssetIntelOrgActionKind } {
  return Boolean(action && isAssetIntelOrgAction(action.kind));
}

export function getOrgActionModel(
  mode: EngagementMode | null,
  options: { isChild?: boolean } = {}
): {
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
    if (options.isChild) {
      return {
        primary: { kind: "enrich_organization", label: "补字段" },
      };
    }
    return {
      primary: { kind: "hydrate_subsidiaries", label: "查子公司" },
      secondary: { kind: "enrich_organization", label: "补字段" },
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

export function buildHydrateConfigFromEngagement(
  engagement: EngagementRecord | null | undefined
): AssetIntelHydrateConfig {
  const minOwnership =
    typeof engagement?.min_ownership_percent === "string" ? engagement.min_ownership_percent : null;
  const depth = typeof engagement?.depth === "string" ? engagement.depth : null;
  const includeBranches =
    typeof engagement?.include_branches === "boolean" ? engagement.include_branches : null;
  const isLegacyHeavyDefault = minOwnership === "51" && depth === "2" && includeBranches === true;

  return {
    minOwnershipPercent: isLegacyHeavyDefault ? null : minOwnership,
    depth: isLegacyHeavyDefault ? null : depth,
    includeBranches: isLegacyHeavyDefault ? null : includeBranches,
    createCandidates:
      typeof engagement?.create_candidates === "boolean" ? engagement.create_candidates : true,
  };
}

export function buildDiscoveryHydrateConfigFromEngagement(
  engagement: EngagementRecord | null | undefined
): AssetIntelHydrateConfig {
  const minOwnership =
    typeof engagement?.min_ownership_percent === "string" && engagement.min_ownership_percent.trim()
      ? engagement.min_ownership_percent
      : "51";
  const depth =
    typeof engagement?.depth === "string" && engagement.depth.trim() ? engagement.depth : "1";
  const includeBranches =
    typeof engagement?.include_branches === "boolean" ? engagement.include_branches : false;

  return {
    minOwnershipPercent: minOwnership,
    depth,
    includeBranches,
    createCandidates:
      typeof engagement?.create_candidates === "boolean" ? engagement.create_candidates : true,
  };
}

export function getEngagementMode(org?: Organization): EngagementMode | null {
  const engagement = org?.intel?.engagement;
  if (!engagement || typeof engagement !== "object" || Array.isArray(engagement)) return null;
  const mode = (engagement as { mode?: unknown }).mode;
  if (mode === "customer_targets" || mode === "discover_assets" || mode === "profile_only") {
    return mode;
  }
  return null;
}

export function getEffectiveEngagementMode(
  org: Organization | undefined,
  orgs: Organization[]
): EngagementMode | null {
  let current = org;
  const seen = new Set<string>();
  while (current && !seen.has(current.id)) {
    seen.add(current.id);
    const mode = getEngagementMode(current);
    if (mode) return mode;
    const parentId = current.parent_id;
    current = parentId ? orgs.find((item) => item.id === parentId) : undefined;
  }
  return null;
}

export function getEngagementRecord(org?: Organization): EngagementRecord | null {
  const engagement = org?.intel?.engagement;
  if (!engagement || typeof engagement !== "object" || Array.isArray(engagement)) return null;
  return engagement as EngagementRecord;
}
