import { invoke } from "./client";

/**
 * Domain entry inside `Organization.domains`. The shape is a discriminated
 * union (string-only short form OR object long form) to keep AI-generated
 * payloads and human-entered ones interchangeable; the drawer always
 * normalizes to the object form on write.
 */
export interface OrgDomainEntry {
  domain: string;
  wildcard?: boolean;
  note?: string;
}

/**
 * `scope_rules` shape — modeled after Bugcrowd / HackerOne policy objects
 * so AI tools can read it in one shot when computing in/out-of-scope.
 *
 * - `in` / `out`: allow/deny CIDRs, domains, URLs
 * - `forbid_time`: ["02:00-06:00", "00:00-24:00 on Sundays"] free-form
 * - `forbid_paths`: ["/admin", "/payment/callback"] (path prefix)
 */
export interface OrgScopeRules {
  in?: string[];
  out?: string[];
  forbid_time?: string[];
  forbid_paths?: string[];
}

/**
 * Multi-level organization tree introduced in §S3, upgraded to a full
 * "owner asset intel" record by the 2026-05-17 profile migration. The
 * tree is reconstructed client-side from the flat list returned by
 * `list()` (each node has `parent_id` pointing at its container; root
 * nodes have `parent_id === null`).
 *
 * **Field grouping (mirrors the 5-tab UI in `OrgProfileDrawer`):**
 * - basic : aliases / industry / tier / credit_code
 * - domain: domains
 * - network: ip_ranges / asns / email_domains
 * - scope : scope_rules
 * - other : intel / notes
 *
 * Phase-2 fields (`certificates` … `contacts`) carry data only; UI
 * editors land in a follow-up PR.
 */
export interface Organization {
  id: string;
  project_path: string;
  name: string;
  parent_id: string | null;
  description: string;
  owner: string;
  sort_order: number;
  aliases: string[];
  industry: string;
  /** `critical | high | medium | low | ''` */
  tier: string;
  credit_code: string;
  domains: OrgDomainEntry[];
  ip_ranges: string[];
  asns: string[];
  email_domains: string[];
  scope_rules: OrgScopeRules;
  intel: Record<string, unknown>;
  notes: string;
  certificates: unknown[];
  subsidiaries: unknown[];
  business_systems: unknown[];
  cloud_assets: unknown[];
  github_orgs: unknown[];
  social_accounts: unknown[];
  historical_vulns: unknown[];
  contacts: unknown[];
  created_at: number;
  updated_at: number;
}

/**
 * Partial profile patch. Every field is optional — only the keys you
 * include get written. Empty array / empty string DOES overwrite (use
 * it to clear); `undefined` means "leave as-is".
 *
 * Backend validates CIDR / domain / ASN format and returns a 400 with a
 * human-readable message if anything fails (no partial writes — the
 * whole patch is rejected atomically).
 */
export interface OrganizationProfilePatch {
  aliases?: string[];
  industry?: string;
  tier?: string;
  credit_code?: string;
  domains?: OrgDomainEntry[];
  ip_ranges?: string[];
  asns?: string[];
  email_domains?: string[];
  scope_rules?: OrgScopeRules;
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

export async function listOrganizations(projectPath: string | null): Promise<Organization[]> {
  return invoke<Organization[]>("organization_list", { projectPath });
}

export async function getOrganization(id: string): Promise<Organization> {
  return invoke<Organization>("organization_get", { id });
}

export async function createOrganization(params: {
  projectPath: string | null;
  name: string;
  parentId?: string;
  description?: string;
  owner?: string;
}): Promise<Organization> {
  return invoke<Organization>("organization_create", params);
}

export async function updateOrganization(params: {
  id: string;
  name?: string;
  description?: string;
  owner?: string;
  sortOrder?: number;
}): Promise<Organization> {
  return invoke<Organization>("organization_update", params);
}

/**
 * PATCH-style profile update. Only sets the fields you pass; everything
 * else stays untouched. Throws on backend validation failure with a
 * message like `"validation: ip_ranges=`1.2.3` → invalid CIDR"`.
 */
export async function updateOrganizationProfile(
  id: string,
  patch: OrganizationProfilePatch
): Promise<Organization> {
  return invoke<Organization>("organization_update_profile", { id, patch });
}

export async function moveOrganization(params: {
  id: string;
  newParentId: string | null;
}): Promise<void> {
  await invoke("organization_move", params);
}

export async function deleteOrganization(id: string): Promise<void> {
  await invoke("organization_delete", { id });
}
