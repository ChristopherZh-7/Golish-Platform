/**
 * Shared types for the Target panel domain helpers.
 *
 * Extracted from the former 2600-line `TargetGroupedView.tsx` god component so
 * the pure org/engagement/asset-intel logic can live in `@/lib/target-panel`
 * and be unit-tested independently of React. These are the cross-module types;
 * module-specific types stay co-located with their module.
 */

/** Right-side workspace tabs in the org workspace panel. */
export type WorkspaceTab = "overview" | "fields" | "scope" | "targets" | "activity";

/** Loosely-typed engagement metadata bag stored under `org.intel.engagement`. */
export type EngagementRecord = Record<string, unknown>;

export type OrgActionKind =
  | "import_targets"
  | "hydrate_subsidiaries"
  | "enrich_organization"
  | "choose_next_step"
  | "add_child";

export type AssetIntelOrgActionKind = "hydrate_subsidiaries" | "enrich_organization";

export interface OrgActionItem {
  kind: OrgActionKind;
  label: string;
}
