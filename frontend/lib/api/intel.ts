import { invoke } from "./client";

/**
 * ASM intel-provider IPC wrappers.
 *
 * The wire-format types mirror `golish-intel-providers/src/types.rs` and
 * `golish/src/tools/intel_providers.rs`. Field names are snake_case to
 * match Rust serde defaults.
 *
 * Provider IDs (stable identifiers):
 * - "0.zone"   — 零零信安
 * - "fofa"     — 鹰图
 * - "quake"    — 360 Quake
 * - "hunter"   — 奇安信 Hunter
 * - "shodan"   — Shodan
 */

export type QueryType =
  | "site"
  | "domain"
  | "email"
  | "apk"
  | "sensitive"
  | "code"
  | "member"
  | "cert"
  | "asn"
  | "cidr";

export interface ProviderMeta {
  id: string;
  display_name: string;
  description: string;
  homepage_url: string;
  signup_url: string;
  docs_url: string;
  supported_query_types: QueryType[];
  quota_hint: string;
  requires_paid: boolean;
}

export interface ProviderRecord {
  provider: string;
  query_type: QueryType;
  fields: Record<string, string>;
  raw: unknown;
  /** RFC3339 timestamp string. */
  fetched_at: string;
}

export type ConnectionStatus =
  | { status: "ok"; message: string; quota_remaining: number | null; quota_total: number | null }
  | { status: "auth_failed"; message: string }
  | { status: "quota_exhausted"; message: string }
  | { status: "network_error"; message: string };

export interface IntelQueryResult {
  provider: string;
  query_type: string;
  records: ProviderRecord[];
  /** How many records were persisted into `organizations`. */
  persisted: number;
  /** Per-record persistence errors (non-fatal). */
  errors: string[];
}

/** List all registered ASM intel providers + their static metadata. */
export async function listProviders(): Promise<ProviderMeta[]> {
  return invoke<ProviderMeta[]>("intel_list_providers");
}

/** Test whether the configured API key for `providerId` is valid. */
export async function testConnection(providerId: string): Promise<ConnectionStatus> {
  return invoke<ConnectionStatus>("intel_test_connection", { providerId });
}

/**
 * Run an ASM intel query and persist results into `organizations`.
 *
 * Returns parsed records so the UI can also display them.
 */
export async function queryProvider(args: {
  providerId: string;
  queryType: QueryType;
  query: string;
  projectPath?: string | null;
}): Promise<IntelQueryResult> {
  return invoke<IntelQueryResult>("intel_query_provider", {
    providerId: args.providerId,
    queryType: args.queryType,
    query: args.query,
    projectPath: args.projectPath ?? null,
  });
}
