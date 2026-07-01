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
  capturePath: string | null;
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
  filePath: string | null;
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
 * A single collected request/response capture, read from disk by the
 * `pentest_read_capture` IPC for the Burp-style Inspector. Payload v2 adds the
 * request `headers`/`body`; v1 captures (response-only) degrade to empty
 * request headers and a `null` body.
 */
export interface CaptureRequest {
  method: string;
  url: string;
  resourceType: string;
  headers: Record<string, string>;
  body: string | null;
}
export interface CaptureResponse {
  status: number | null;
  headers: Record<string, string>;
  contentType: string;
  bodyLen: number | null;
  bodyTextSample: string;
  bodyBase64: string | null;
}
export interface CapturePayload {
  version: number;
  capturedAt: string;
  request: CaptureRequest;
  response: CaptureResponse;
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

type ApiRecord = Record<string, unknown>;

function asRecord(value: unknown): ApiRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as ApiRecord) : {};
}

function get(row: ApiRecord, ...keys: string[]): unknown {
  for (const key of keys) {
    if (key in row) return row[key];
  }
  return undefined;
}

function stringField(row: ApiRecord, ...keys: string[]): string {
  const value = get(row, ...keys);
  if (typeof value === "string") return value;
  if (value == null) return "";
  return String(value);
}

function nullableStringField(row: ApiRecord, ...keys: string[]): string | null {
  const value = get(row, ...keys);
  if (value == null) return null;
  return typeof value === "string" ? value : String(value);
}

function nullableNumberField(row: ApiRecord, ...keys: string[]): number | null {
  const value = get(row, ...keys);
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function booleanField(row: ApiRecord, ...keys: string[]): boolean {
  const value = get(row, ...keys);
  return typeof value === "boolean" ? value : value === "true";
}

function arrayField(row: ApiRecord, ...keys: string[]): unknown[] {
  const value = get(row, ...keys);
  return Array.isArray(value) ? value : [];
}

function recordField(row: ApiRecord, ...keys: string[]): Record<string, unknown> {
  return asRecord(get(row, ...keys));
}

function listOf<T>(value: unknown, normalize: (row: unknown) => T): T[] {
  return Array.isArray(value) ? value.map(normalize) : [];
}

function normalizeAuditRow(value: unknown): AuditRow {
  const row = asRecord(value);
  return {
    ...(row as Partial<AuditRow>),
    id: nullableNumberField(row, "id") ?? 0,
    action: stringField(row, "action"),
    category: stringField(row, "category"),
    details: stringField(row, "details"),
    entityType: nullableStringField(row, "entityType", "entity_type"),
    entityId: nullableStringField(row, "entityId", "entity_id"),
    source: stringField(row, "source"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    targetId: nullableStringField(row, "targetId", "target_id"),
    sessionId: nullableStringField(row, "sessionId", "session_id"),
    toolName: nullableStringField(row, "toolName", "tool_name"),
    status: stringField(row, "status"),
    detail: recordField(row, "detail"),
    createdAt: nullableNumberField(row, "createdAt", "created_at") ?? 0,
  };
}

function normalizeTargetAsset(value: unknown): TargetAsset {
  const row = asRecord(value);
  return {
    ...(row as Partial<TargetAsset>),
    id: stringField(row, "id"),
    targetId: stringField(row, "targetId", "target_id"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    assetType: stringField(row, "assetType", "asset_type"),
    value: stringField(row, "value"),
    port: nullableNumberField(row, "port"),
    protocol: nullableStringField(row, "protocol"),
    service: nullableStringField(row, "service"),
    version: nullableStringField(row, "version"),
    metadata: recordField(row, "metadata"),
    status: stringField(row, "status"),
    discoveredAt: stringField(row, "discoveredAt", "discovered_at"),
    updatedAt: stringField(row, "updatedAt", "updated_at"),
  };
}

function normalizeApiEndpoint(value: unknown): ApiEndpoint {
  const row = asRecord(value);
  return {
    ...(row as Partial<ApiEndpoint>),
    id: stringField(row, "id"),
    targetId: stringField(row, "targetId", "target_id"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    url: stringField(row, "url"),
    method: stringField(row, "method"),
    path: stringField(row, "path"),
    params: arrayField(row, "params"),
    headers: recordField(row, "headers"),
    authType: nullableStringField(row, "authType", "auth_type"),
    responseType: nullableStringField(row, "responseType", "response_type"),
    statusCode: nullableNumberField(row, "statusCode", "status_code"),
    notes: stringField(row, "notes"),
    source: stringField(row, "source"),
    riskLevel: stringField(row, "riskLevel", "risk_level"),
    tested: booleanField(row, "tested"),
    capturePath: nullableStringField(row, "capturePath", "capture_path"),
    discoveredAt: stringField(row, "discoveredAt", "discovered_at"),
    updatedAt: stringField(row, "updatedAt", "updated_at"),
  };
}

function normalizeFingerprint(value: unknown): Fingerprint {
  const row = asRecord(value);
  return {
    ...(row as Partial<Fingerprint>),
    id: stringField(row, "id"),
    targetId: stringField(row, "targetId", "target_id"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    category: stringField(row, "category"),
    name: stringField(row, "name"),
    version: nullableStringField(row, "version"),
    confidence: nullableNumberField(row, "confidence") ?? 0,
    evidence: arrayField(row, "evidence"),
    cpe: nullableStringField(row, "cpe"),
    source: stringField(row, "source"),
    detectedAt: stringField(row, "detectedAt", "detected_at"),
  };
}

function normalizeJsAnalysisResult(value: unknown): JsAnalysisResult {
  const row = asRecord(value);
  return {
    ...(row as Partial<JsAnalysisResult>),
    id: stringField(row, "id"),
    targetId: stringField(row, "targetId", "target_id"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    url: stringField(row, "url"),
    filename: stringField(row, "filename"),
    filePath: nullableStringField(row, "filePath", "file_path"),
    sizeBytes: nullableNumberField(row, "sizeBytes", "size_bytes"),
    hashSha256: nullableStringField(row, "hashSha256", "hash_sha256"),
    frameworks: arrayField(row, "frameworks"),
    libraries: arrayField(row, "libraries"),
    endpointsFound: arrayField(row, "endpointsFound", "endpoints_found"),
    secretsFound: arrayField(row, "secretsFound", "secrets_found"),
    comments: arrayField(row, "comments"),
    sourceMaps: booleanField(row, "sourceMaps", "source_maps"),
    riskSummary: stringField(row, "riskSummary", "risk_summary"),
    rawAnalysis: recordField(row, "rawAnalysis", "raw_analysis"),
    analyzedAt: stringField(row, "analyzedAt", "analyzed_at"),
  };
}

function normalizePassiveScanLog(value: unknown): PassiveScanLog {
  const row = asRecord(value);
  return {
    ...(row as Partial<PassiveScanLog>),
    id: stringField(row, "id"),
    targetId: stringField(row, "targetId", "target_id"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    testType: stringField(row, "testType", "test_type"),
    payload: stringField(row, "payload"),
    url: stringField(row, "url"),
    parameter: stringField(row, "parameter"),
    result: stringField(row, "result"),
    evidence: stringField(row, "evidence"),
    severity: stringField(row, "severity"),
    toolUsed: stringField(row, "toolUsed", "tool_used"),
    tester: stringField(row, "tester"),
    notes: stringField(row, "notes"),
    detail: recordField(row, "detail"),
    testedAt: stringField(row, "testedAt", "tested_at"),
  };
}

function normalizeTimelineEntry(value: unknown): TimelineEntry {
  const row = asRecord(value);
  return {
    ...(row as Partial<TimelineEntry>),
    source: stringField(row, "source"),
    event: stringField(row, "event"),
    category: stringField(row, "category"),
    details: stringField(row, "details"),
    toolName: nullableStringField(row, "toolName", "tool_name"),
    status: stringField(row, "status"),
    detail: recordField(row, "detail"),
    createdAt: stringField(row, "createdAt", "created_at"),
  };
}

/** Coerce an arbitrary object into a `Record<string, string>` header map. */
function strMap(value: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, raw] of Object.entries(asRecord(value))) out[key] = String(raw);
  return out;
}

export function normalizeCapturePayload(value: unknown): CapturePayload {
  const row = asRecord(value);
  const req = asRecord(get(row, "request"));
  const res = asRecord(get(row, "response"));
  return {
    version: nullableNumberField(row, "version") ?? 1,
    capturedAt: stringField(row, "capturedAt", "captured_at"),
    request: {
      method: (stringField(req, "method") || "GET").toUpperCase(),
      url: stringField(req, "url"),
      resourceType: stringField(req, "resourceType", "resource_type"),
      headers: strMap(get(req, "headers")),
      body: typeof req.body === "string" ? req.body : null,
    },
    response: {
      status: nullableNumberField(res, "status"),
      headers: strMap(get(res, "headers")),
      contentType: stringField(res, "contentType", "content_type"),
      bodyLen: nullableNumberField(res, "bodyLen", "body_len"),
      bodyTextSample: stringField(res, "bodyTextSample", "body_text_sample"),
      bodyBase64: nullableStringField(res, "bodyBase64", "body_base64"),
    },
  };
}

// ─── Operation / Audit Log ─────────────────────────────────────────────

export async function oplogList(projectPath: string, limit?: number): Promise<AuditRow[]> {
  return listOf(await invoke("oplog_list", { projectPath, limit }), normalizeAuditRow);
}

export async function oplogListByTarget(targetId: string, limit?: number): Promise<AuditRow[]> {
  return listOf(await invoke("oplog_list_by_target", { targetId, limit }), normalizeAuditRow);
}

export async function oplogListByType(
  projectPath: string,
  opType: string,
  limit?: number
): Promise<AuditRow[]> {
  return listOf(
    await invoke("oplog_list_by_type", { projectPath, opType, limit }),
    normalizeAuditRow
  );
}

export async function oplogSearch(
  projectPath: string,
  query: string,
  limit?: number
): Promise<AuditRow[]> {
  return listOf(await invoke("oplog_search", { projectPath, query, limit }), normalizeAuditRow);
}

export async function oplogCount(projectPath: string): Promise<number> {
  return invoke("oplog_count", { projectPath });
}

// ─── Target Security Data ──────────────────────────────────────────────

export async function targetAssetsList(targetId: string): Promise<TargetAsset[]> {
  return listOf(await invoke("target_assets_list", { targetId }), normalizeTargetAsset);
}

export async function apiEndpointsList(targetId: string): Promise<ApiEndpoint[]> {
  return listOf(await invoke("api_endpoints_list", { targetId }), normalizeApiEndpoint);
}

export async function apiEndpointsUntested(targetId: string): Promise<ApiEndpoint[]> {
  return listOf(await invoke("api_endpoints_untested", { targetId }), normalizeApiEndpoint);
}

export async function fingerprintsList(targetId: string): Promise<Fingerprint[]> {
  return listOf(await invoke("fingerprints_list", { targetId }), normalizeFingerprint);
}

export async function jsAnalysisList(targetId: string): Promise<JsAnalysisResult[]> {
  return listOf(await invoke("js_analysis_list", { targetId }), normalizeJsAnalysisResult);
}

export async function passiveScansList(
  targetId: string,
  limit?: number
): Promise<PassiveScanLog[]> {
  return listOf(await invoke("passive_scans_list", { targetId, limit }), normalizePassiveScanLog);
}

export async function passiveScansVulnerable(targetId: string): Promise<PassiveScanLog[]> {
  return listOf(await invoke("passive_scans_vulnerable", { targetId }), normalizePassiveScanLog);
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
  return listOf(await invoke("target_timeline", { targetId, limit }), normalizeTimelineEntry);
}

// ─── Capture Inspector (Burp-style Req/Resp viewer) ────────────────────

/**
 * Read one collected request/response capture JSON for the Inspector. The
 * backend guards `capturePath` to stay under `<projectPath>/.golish/captures`
 * (read-only, rejects `..` traversal).
 */
export async function readCapture(
  projectPath: string,
  capturePath: string
): Promise<CapturePayload> {
  return normalizeCapturePayload(
    await invoke("pentest_read_capture", { projectPath, capturePath })
  );
}

export async function readCaptureText(projectPath: string, capturePath: string): Promise<string> {
  return invoke("pentest_read_capture_text", { projectPath, capturePath });
}
