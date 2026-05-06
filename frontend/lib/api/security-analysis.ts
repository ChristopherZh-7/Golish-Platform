import { invoke } from "@/lib/api/client";

export interface AuditRow {
  id: number;
  action: string;
  category: string;
  details: string;
  entityType: string | null;
  entityId: string | null;
  source: string;
  projectPath: string | null;
  targetId: string | null;
  sessionId: string | null;
  toolName: string | null;
  status: string;
  detail: Record<string, unknown>;
  createdAt: number;
}

export interface TargetAsset {
  id: string;
  targetId: string;
  projectPath: string | null;
  assetType: string;
  value: string;
  port: number | null;
  protocol: string | null;
  service: string | null;
  version: string | null;
  metadata: Record<string, unknown>;
  status: string;
  discoveredAt: string;
  updatedAt: string;
}

export interface ApiEndpoint {
  id: string;
  targetId: string;
  projectPath: string | null;
  url: string;
  method: string;
  path: string;
  params: unknown[];
  headers: Record<string, unknown>;
  authType: string | null;
  responseType: string | null;
  statusCode: number | null;
  notes: string;
  source: string;
  riskLevel: string;
  tested: boolean;
  discoveredAt: string;
  updatedAt: string;
}

export interface Fingerprint {
  id: string;
  targetId: string;
  projectPath: string | null;
  category: string;
  name: string;
  version: string | null;
  confidence: number;
  evidence: unknown[];
  cpe: string | null;
  source: string;
  detectedAt: string;
}

export interface JsAnalysisResult {
  id: string;
  targetId: string;
  projectPath: string | null;
  url: string;
  filename: string;
  sizeBytes: number | null;
  hashSha256: string | null;
  frameworks: unknown[];
  libraries: unknown[];
  endpointsFound: unknown[];
  secretsFound: unknown[];
  comments: unknown[];
  sourceMaps: boolean;
  riskSummary: string;
  rawAnalysis: Record<string, unknown>;
  analyzedAt: string;
}

export interface PassiveScanLog {
  id: string;
  targetId: string;
  projectPath: string | null;
  testType: string;
  payload: string;
  url: string;
  parameter: string;
  result: string;
  evidence: string;
  severity: string;
  toolUsed: string;
  tester: string;
  notes: string;
  detail: Record<string, unknown>;
  testedAt: string;
}

export interface SecurityOverview {
  assetsCount: number;
  endpointsTotal: number;
  endpointsTested: number;
  scanStats: Record<string, number>;
}

/**
 * Cross-table activity entry returned by the `target_timeline` IPC command.
 * Aggregates rows from `audit_log`, `target_assets`, `api_endpoints`,
 * `passive_scan_logs`, and `findings` into a unified shape ordered by
 * `createdAt` descending. `toolName` is `null` when the source row carried
 * no tool attribution (e.g. asset discovery).
 */
export interface TimelineEntry {
  /** Origin table label. */
  source:
    | "audit_log"
    | "target_assets"
    | "api_endpoints"
    | "passive_scan_logs"
    | "findings"
    | string;
  /** Event keyword (e.g. `target_added`, `endpoint_discovered`, `xss`). */
  event: string;
  /** Bucket category (`scan`, `targets`, `api`, severity, ...). */
  category: string;
  /** Human-readable summary line. */
  details: string;
  /** Tool that produced the event, when applicable. */
  toolName: string | null;
  /** Status / verdict (`completed`, `vulnerable`, `tested`, `open`, ...). */
  status: string;
  /** Source-specific JSON payload. */
  detail: Record<string, unknown>;
  /** ISO 8601 timestamp from PostgreSQL `TIMESTAMPTZ`. */
  createdAt: string;
}

// ─── Operation / Audit Log ─────────────────────────────────────────────

export async function oplogList(projectPath: string, limit?: number): Promise<AuditRow[]> {
  return invoke("oplog_list", { projectPath, limit });
}

export async function oplogListByTarget(targetId: string, limit?: number): Promise<AuditRow[]> {
  return invoke("oplog_list_by_target", { targetId, limit });
}

export async function oplogListByType(
  projectPath: string,
  opType: string,
  limit?: number
): Promise<AuditRow[]> {
  return invoke("oplog_list_by_type", { projectPath, opType, limit });
}

export async function oplogSearch(
  projectPath: string,
  query: string,
  limit?: number
): Promise<AuditRow[]> {
  return invoke("oplog_search", { projectPath, query, limit });
}

export async function oplogCount(projectPath: string): Promise<number> {
  return invoke("oplog_count", { projectPath });
}

// ─── Target Security Data ──────────────────────────────────────────────

export async function targetAssetsList(targetId: string): Promise<TargetAsset[]> {
  return invoke("target_assets_list", { targetId });
}

export async function apiEndpointsList(targetId: string): Promise<ApiEndpoint[]> {
  return invoke("api_endpoints_list", { targetId });
}

export async function apiEndpointsUntested(targetId: string): Promise<ApiEndpoint[]> {
  return invoke("api_endpoints_untested", { targetId });
}

export async function fingerprintsList(targetId: string): Promise<Fingerprint[]> {
  return invoke("fingerprints_list", { targetId });
}

export async function jsAnalysisList(targetId: string): Promise<JsAnalysisResult[]> {
  return invoke("js_analysis_list", { targetId });
}

export async function passiveScansList(
  targetId: string,
  limit?: number
): Promise<PassiveScanLog[]> {
  return invoke("passive_scans_list", { targetId, limit });
}

export async function passiveScansVulnerable(targetId: string): Promise<PassiveScanLog[]> {
  return invoke("passive_scans_vulnerable", { targetId });
}

export async function passiveScansStats(targetId: string): Promise<Record<string, number>> {
  return invoke("passive_scans_stats", { targetId });
}

export async function targetSecurityOverview(targetId: string): Promise<SecurityOverview> {
  return invoke("target_security_overview", { targetId });
}

// ─── Target Activity Timeline ──────────────────────────────────────────

/**
 * Fetch a unified per-target activity timeline aggregated across
 * `audit_log`, `target_assets`, `api_endpoints`, `passive_scan_logs`,
 * and `findings`. Newest event first. `limit` defaults to 200 server-side.
 */
export async function targetTimeline(targetId: string, limit?: number): Promise<TimelineEntry[]> {
  return invoke("target_timeline", { targetId, limit });
}
