import { invoke } from "@tauri-apps/api/core";

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

// ─── Operation / Audit Log ─────────────────────────────────────────────

export async function oplogList(
  projectPath: string,
  limit?: number
): Promise<AuditRow[]> {
  return invoke("oplog_list", { projectPath, limit });
}

export async function oplogListByTarget(
  targetId: string,
  limit?: number
): Promise<AuditRow[]> {
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

export async function targetAssetsList(
  targetId: string
): Promise<TargetAsset[]> {
  return invoke("target_assets_list", { targetId });
}

export async function apiEndpointsList(
  targetId: string
): Promise<ApiEndpoint[]> {
  return invoke("api_endpoints_list", { targetId });
}

export async function apiEndpointsUntested(
  targetId: string
): Promise<ApiEndpoint[]> {
  return invoke("api_endpoints_untested", { targetId });
}

export async function fingerprintsList(
  targetId: string
): Promise<Fingerprint[]> {
  return invoke("fingerprints_list", { targetId });
}

export async function jsAnalysisList(
  targetId: string
): Promise<JsAnalysisResult[]> {
  return invoke("js_analysis_list", { targetId });
}

export async function passiveScansList(
  targetId: string,
  limit?: number
): Promise<PassiveScanLog[]> {
  return invoke("passive_scans_list", { targetId, limit });
}

export async function passiveScansVulnerable(
  targetId: string
): Promise<PassiveScanLog[]> {
  return invoke("passive_scans_vulnerable", { targetId });
}

export async function passiveScansStats(
  targetId: string
): Promise<Record<string, number>> {
  return invoke("passive_scans_stats", { targetId });
}

export async function targetSecurityOverview(
  targetId: string
): Promise<SecurityOverview> {
  return invoke("target_security_overview", { targetId });
}
