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

export interface BackendSurfaceTargetDto {
  id: string;
  name: string;
  targetType: string;
  value: string;
  tags: unknown;
  notes: string;
  scope: string;
  group: string;
  projectPath: string | null;
  organizationId: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface BackendNetworkEndpointDto {
  id: string;
  ip: string;
  port: number;
  transport: string;
  state: string;
  serviceName: string | null;
  serviceProduct: string | null;
  serviceVersion: string | null;
  banner: string | null;
  tlsDetected: boolean;
  source: string;
  confidence: number;
  firstSeenAt: number;
  lastSeenAt: number;
  lastConfirmedAt: number | null;
  webOriginIds: string[];
  observationIds: string[];
}

export interface BackendWebOriginCountsDto {
  endpointCount: number;
  observationCount: number;
}

/**
 * Phase 2.5A backend legacy content aggregation for a single WebOrigin.
 * These are display counts derived from legacy tables (`api_endpoints`,
 * `js_analysis_results`, `directory_entries`, `passive_scan_logs`); they are
 * NOT row payloads. `null` on the parent origin means the backend did not emit
 * `contentCounts` (older payloads), and the frontend should fall back to its
 * own inferred counts.
 */
export interface BackendWebOriginContentCountsDto {
  urlCount: number;
  apiCount: number;
  jsCount: number;
  paramCount: number;
  directoryEntryCount: number;
  passiveLogCount: number;
  evidenceCount: number;
}

/**
 * Phase 2.5C lightweight pointer to a single legacy content row. This is NOT a
 * full `api_endpoints` / `js_analysis_results` row — only enough metadata for a
 * compact list + deep link. Totals still come from `contentCounts`; refs are a
 * bounded (max ~200 per bucket) best-effort enumeration.
 */
export interface BackendWebOriginContentRefDto {
  kind: string;
  id: string;
  url: string;
  method: string | null;
  statusCode: number | null;
  capturePath: string | null;
  source: string | null;
}

export interface BackendCrawlObservationDto {
  id: string;
  originTargetId: string;
  originUrl: string;
  originKey: string;
  observedUrl: string;
  observedHost: string | null;
  observedPath: string | null;
  kind: string;
  sameOrigin: boolean;
  sourceTool: string;
  sourceRecordId: string | null;
  evidenceId: number | null;
  metadata: Record<string, unknown>;
  discoveredAt: number;
  updatedAt: number;
}

export interface BackendWebOriginDto {
  id: string;
  scheme: string;
  host: string;
  hostType: string;
  port: number;
  origin: string;
  source: string;
  confidence: number;
  firstSeenAt: number;
  lastSeenAt: number;
  lastConfirmedAt: number | null;
  endpointIds: string[];
  observationIds: string[];
  counts: BackendWebOriginCountsDto;
  /** `null` when the backend payload omitted `contentCounts` (pre-2.5A). */
  contentCounts: BackendWebOriginContentCountsDto | null;
  /** Bounded lightweight refs for this origin (empty on pre-2.5C payloads). */
  refs: BackendWebOriginContentRefDto[];
  /** Crawler output owned by this origin; not gate-driving api_endpoints data. */
  crawlObservations: BackendCrawlObservationDto[];
}

export interface BackendWebOriginObservationDto {
  id: string;
  webOriginId: string;
  networkEndpointId: string | null;
  targetId: string | null;
  observedIp: string | null;
  sni: string | null;
  hostHeader: string | null;
  statusCode: number | null;
  title: string | null;
  finalUrl: string | null;
  redirectChain: unknown;
  bodyHash: string | null;
  faviconHash: string | null;
  screenshotPath: string | null;
  capturePath: string | null;
  observedAt: number;
  confidence: number;
  source: string;
}

export interface BackendRelatedDomainDto {
  targetId: string | null;
  host: string;
  source: string;
  relation: string;
}

/**
 * Phase 2.5A backend content aggregation buckets for data that could not be
 * attributed to a backend WebOrigin. `unmatchedOriginCount` counts URLs that
 * parsed to an origin absent from backend identity; the rest are relative /
 * malformed / unsupported-scheme / missing-URL items. These are counts only —
 * the backend never synthesizes identity rows for them.
 */
export interface BackendUnassignedWebDataCountsDto {
  urlCount: number;
  apiCount: number;
  jsCount: number;
  paramCount: number;
  directoryEntryCount: number;
  passiveLogCount: number;
  evidenceCount: number;
  relativeUrlCount: number;
  malformedUrlCount: number;
  unsupportedSchemeCount: number;
  missingOriginCount: number;
  unmatchedOriginCount: number;
  unmatchedOriginItemCount: number;
}

export interface BackendUnassignedWebDataDto {
  urls: unknown[];
  apis: unknown[];
  js: unknown[];
  params: unknown[];
  reason: string;
  /** `null` when the backend payload omitted aggregation counts (pre-2.5A). */
  counts: BackendUnassignedWebDataCountsDto | null;
  /** Bounded lightweight refs for unmatched/unassigned content (pre-2.5C: empty). */
  refs: BackendWebOriginContentRefDto[];
}

/**
 * Phase 2.5C in-app backfill summary (`target_surface_identity_backfill`). All
 * counters describe rows scanned / identity rows upserted; the backfill is
 * additive and idempotent and never mutates legacy tables.
 */
export interface SurfaceIdentityBackfillSummary {
  scannedTargets: number;
  scannedTargetAssets: number;
  scannedApiEndpoints: number;
  scannedJsResults: number;
  scannedDirectoryEntries: number;
  scannedPassiveLogs: number;
  createdOrUpdatedNetworkEndpoints: number;
  createdOrUpdatedWebOrigins: number;
  createdOrUpdatedObservations: number;
  skippedRelativeUrls: number;
  skippedMalformedUrls: number;
  skippedMissingEndpoint: number;
  skippedUnsupportedScheme: number;
  inferredObservations: number;
  confirmedObservations: number;
}

export interface BackendSurfaceSummaryDto {
  endpointCount: number;
  webOriginCount: number;
  observationCount: number;
  inferredObservationCount: number;
  confirmedObservationCount: number;
  relatedDomainCount: number;
  unassignedCount: number;
  // Phase 2.5A content aggregation totals (matched origins only). `null` when
  // the backend payload omitted them, signalling the frontend to keep its own
  // inferred summary counts instead.
  urlCount: number | null;
  apiCount: number | null;
  jsCount: number | null;
  paramCount: number | null;
  directoryEntryCount: number | null;
  passiveLogCount: number | null;
  evidenceCount: number | null;
  contentUnassignedCount: number | null;
  contentUnmatchedOriginCount: number | null;
}

export interface BackendSurfaceHierarchyDto {
  rootTarget: BackendSurfaceTargetDto;
  mode: string;
  dataSource: string;
  generatedAt: number;
  endpoints: BackendNetworkEndpointDto[];
  webOrigins: BackendWebOriginDto[];
  observations: BackendWebOriginObservationDto[];
  relatedDomains: BackendRelatedDomainDto[];
  unassignedWebData: BackendUnassignedWebDataDto;
  summary: BackendSurfaceSummaryDto;
  fallbackReason: string | null;
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

/**
 * Like {@link nullableNumberField} but distinguishes "field absent" (returns
 * `null`) from "field present as 0" (returns `0`). Used for optional Phase 2.5A
 * content-count fields so the frontend can fall back to its own inferred counts
 * only when the backend genuinely did not emit them.
 */
function presentNumberField(row: ApiRecord, ...keys: string[]): number | null {
  const value = get(row, ...keys);
  if (value === undefined || value === null) return null;
  return nullableNumberField(row, ...keys);
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

function normalizeBackendSurfaceTarget(value: unknown): BackendSurfaceTargetDto {
  const row = asRecord(value);
  return {
    id: stringField(row, "id"),
    name: stringField(row, "name"),
    targetType: stringField(row, "targetType", "target_type"),
    value: stringField(row, "value"),
    tags: get(row, "tags") ?? [],
    notes: stringField(row, "notes"),
    scope: stringField(row, "scope"),
    group: stringField(row, "group"),
    projectPath: nullableStringField(row, "projectPath", "project_path"),
    organizationId: nullableStringField(row, "organizationId", "organization_id"),
    createdAt: nullableNumberField(row, "createdAt", "created_at") ?? 0,
    updatedAt: nullableNumberField(row, "updatedAt", "updated_at") ?? 0,
  };
}

function normalizeBackendNetworkEndpoint(value: unknown): BackendNetworkEndpointDto {
  const row = asRecord(value);
  return {
    id: stringField(row, "id"),
    ip: stringField(row, "ip"),
    port: nullableNumberField(row, "port") ?? 0,
    transport: stringField(row, "transport"),
    state: stringField(row, "state"),
    serviceName: nullableStringField(row, "serviceName", "service_name"),
    serviceProduct: nullableStringField(row, "serviceProduct", "service_product"),
    serviceVersion: nullableStringField(row, "serviceVersion", "service_version"),
    banner: nullableStringField(row, "banner"),
    tlsDetected: booleanField(row, "tlsDetected", "tls_detected"),
    source: stringField(row, "source"),
    confidence: nullableNumberField(row, "confidence") ?? 0,
    firstSeenAt: nullableNumberField(row, "firstSeenAt", "first_seen_at") ?? 0,
    lastSeenAt: nullableNumberField(row, "lastSeenAt", "last_seen_at") ?? 0,
    lastConfirmedAt: nullableNumberField(row, "lastConfirmedAt", "last_confirmed_at"),
    webOriginIds: arrayField(row, "webOriginIds", "web_origin_ids").map(String),
    observationIds: arrayField(row, "observationIds", "observation_ids").map(String),
  };
}

function normalizeBackendWebOriginCounts(value: unknown): BackendWebOriginCountsDto {
  const row = asRecord(value);
  return {
    endpointCount: nullableNumberField(row, "endpointCount", "endpoint_count") ?? 0,
    observationCount: nullableNumberField(row, "observationCount", "observation_count") ?? 0,
  };
}

function normalizeBackendWebOriginContentCounts(value: unknown): BackendWebOriginContentCountsDto {
  const row = asRecord(value);
  return {
    urlCount: nullableNumberField(row, "urlCount", "url_count") ?? 0,
    apiCount: nullableNumberField(row, "apiCount", "api_count") ?? 0,
    jsCount: nullableNumberField(row, "jsCount", "js_count") ?? 0,
    paramCount: nullableNumberField(row, "paramCount", "param_count") ?? 0,
    directoryEntryCount:
      nullableNumberField(row, "directoryEntryCount", "directory_entry_count") ?? 0,
    passiveLogCount: nullableNumberField(row, "passiveLogCount", "passive_log_count") ?? 0,
    evidenceCount: nullableNumberField(row, "evidenceCount", "evidence_count") ?? 0,
  };
}

function normalizeBackendWebOriginContentRef(value: unknown): BackendWebOriginContentRefDto {
  const row = asRecord(value);
  return {
    kind: stringField(row, "kind"),
    id: stringField(row, "id"),
    url: stringField(row, "url"),
    method: nullableStringField(row, "method"),
    statusCode: nullableNumberField(row, "statusCode", "status_code"),
    capturePath: nullableStringField(row, "capturePath", "capture_path"),
    source: nullableStringField(row, "source"),
  };
}

function normalizeBackendCrawlObservation(value: unknown): BackendCrawlObservationDto {
  const row = asRecord(value);
  return {
    id: stringField(row, "id"),
    originTargetId: stringField(row, "originTargetId", "origin_target_id"),
    originUrl: stringField(row, "originUrl", "origin_url"),
    originKey: stringField(row, "originKey", "origin_key"),
    observedUrl: stringField(row, "observedUrl", "observed_url"),
    observedHost: nullableStringField(row, "observedHost", "observed_host"),
    observedPath: nullableStringField(row, "observedPath", "observed_path"),
    kind: stringField(row, "kind") || "url",
    sameOrigin: booleanField(row, "sameOrigin", "same_origin"),
    sourceTool: stringField(row, "sourceTool", "source_tool") || "crawler",
    sourceRecordId: nullableStringField(row, "sourceRecordId", "source_record_id"),
    evidenceId: nullableNumberField(row, "evidenceId", "evidence_id"),
    metadata: recordField(row, "metadata"),
    discoveredAt: nullableNumberField(row, "discoveredAt", "discovered_at") ?? 0,
    updatedAt: nullableNumberField(row, "updatedAt", "updated_at") ?? 0,
  };
}

function normalizeBackendWebOrigin(value: unknown): BackendWebOriginDto {
  const row = asRecord(value);
  const contentCountsRaw = get(row, "contentCounts", "content_counts");
  return {
    id: stringField(row, "id"),
    scheme: stringField(row, "scheme"),
    host: stringField(row, "host"),
    hostType: stringField(row, "hostType", "host_type"),
    port: nullableNumberField(row, "port") ?? 0,
    origin: stringField(row, "origin"),
    source: stringField(row, "source"),
    confidence: nullableNumberField(row, "confidence") ?? 0,
    firstSeenAt: nullableNumberField(row, "firstSeenAt", "first_seen_at") ?? 0,
    lastSeenAt: nullableNumberField(row, "lastSeenAt", "last_seen_at") ?? 0,
    lastConfirmedAt: nullableNumberField(row, "lastConfirmedAt", "last_confirmed_at"),
    endpointIds: arrayField(row, "endpointIds", "endpoint_ids").map(String),
    observationIds: arrayField(row, "observationIds", "observation_ids").map(String),
    counts: normalizeBackendWebOriginCounts(get(row, "counts")),
    contentCounts:
      contentCountsRaw == null ? null : normalizeBackendWebOriginContentCounts(contentCountsRaw),
    refs: listOf(get(row, "refs"), normalizeBackendWebOriginContentRef),
    crawlObservations: listOf(
      get(row, "crawlObservations", "crawl_observations"),
      normalizeBackendCrawlObservation
    ),
  };
}

function normalizeBackendWebOriginObservation(value: unknown): BackendWebOriginObservationDto {
  const row = asRecord(value);
  return {
    id: stringField(row, "id"),
    webOriginId: stringField(row, "webOriginId", "web_origin_id"),
    networkEndpointId: nullableStringField(row, "networkEndpointId", "network_endpoint_id"),
    targetId: nullableStringField(row, "targetId", "target_id"),
    observedIp: nullableStringField(row, "observedIp", "observed_ip"),
    sni: nullableStringField(row, "sni"),
    hostHeader: nullableStringField(row, "hostHeader", "host_header"),
    statusCode: nullableNumberField(row, "statusCode", "status_code"),
    title: nullableStringField(row, "title"),
    finalUrl: nullableStringField(row, "finalUrl", "final_url"),
    redirectChain: get(row, "redirectChain", "redirect_chain") ?? [],
    bodyHash: nullableStringField(row, "bodyHash", "body_hash"),
    faviconHash: nullableStringField(row, "faviconHash", "favicon_hash"),
    screenshotPath: nullableStringField(row, "screenshotPath", "screenshot_path"),
    capturePath: nullableStringField(row, "capturePath", "capture_path"),
    observedAt: nullableNumberField(row, "observedAt", "observed_at") ?? 0,
    confidence: nullableNumberField(row, "confidence") ?? 0,
    source: stringField(row, "source"),
  };
}

function normalizeBackendRelatedDomain(value: unknown): BackendRelatedDomainDto {
  const row = asRecord(value);
  return {
    targetId: nullableStringField(row, "targetId", "target_id"),
    host: stringField(row, "host"),
    source: stringField(row, "source"),
    relation: stringField(row, "relation"),
  };
}

function normalizeBackendUnassignedWebDataCounts(
  value: unknown
): BackendUnassignedWebDataCountsDto {
  const row = asRecord(value);
  return {
    urlCount: nullableNumberField(row, "urlCount", "url_count") ?? 0,
    apiCount: nullableNumberField(row, "apiCount", "api_count") ?? 0,
    jsCount: nullableNumberField(row, "jsCount", "js_count") ?? 0,
    paramCount: nullableNumberField(row, "paramCount", "param_count") ?? 0,
    directoryEntryCount:
      nullableNumberField(row, "directoryEntryCount", "directory_entry_count") ?? 0,
    passiveLogCount: nullableNumberField(row, "passiveLogCount", "passive_log_count") ?? 0,
    evidenceCount: nullableNumberField(row, "evidenceCount", "evidence_count") ?? 0,
    relativeUrlCount: nullableNumberField(row, "relativeUrlCount", "relative_url_count") ?? 0,
    malformedUrlCount: nullableNumberField(row, "malformedUrlCount", "malformed_url_count") ?? 0,
    unsupportedSchemeCount:
      nullableNumberField(row, "unsupportedSchemeCount", "unsupported_scheme_count") ?? 0,
    missingOriginCount: nullableNumberField(row, "missingOriginCount", "missing_origin_count") ?? 0,
    unmatchedOriginCount:
      nullableNumberField(row, "unmatchedOriginCount", "unmatched_origin_count") ?? 0,
    unmatchedOriginItemCount:
      nullableNumberField(row, "unmatchedOriginItemCount", "unmatched_origin_item_count") ?? 0,
  };
}

function normalizeBackendUnassignedWebData(value: unknown): BackendUnassignedWebDataDto {
  const row = asRecord(value);
  const countsRaw = get(row, "counts");
  return {
    urls: arrayField(row, "urls"),
    apis: arrayField(row, "apis"),
    js: arrayField(row, "js"),
    params: arrayField(row, "params"),
    reason: stringField(row, "reason"),
    counts: countsRaw == null ? null : normalizeBackendUnassignedWebDataCounts(countsRaw),
    refs: listOf(get(row, "refs"), normalizeBackendWebOriginContentRef),
  };
}

function normalizeSurfaceIdentityBackfillSummary(value: unknown): SurfaceIdentityBackfillSummary {
  const row = asRecord(value);
  return {
    scannedTargets: nullableNumberField(row, "scannedTargets", "scanned_targets") ?? 0,
    scannedTargetAssets:
      nullableNumberField(row, "scannedTargetAssets", "scanned_target_assets") ?? 0,
    scannedApiEndpoints:
      nullableNumberField(row, "scannedApiEndpoints", "scanned_api_endpoints") ?? 0,
    scannedJsResults: nullableNumberField(row, "scannedJsResults", "scanned_js_results") ?? 0,
    scannedDirectoryEntries:
      nullableNumberField(row, "scannedDirectoryEntries", "scanned_directory_entries") ?? 0,
    scannedPassiveLogs: nullableNumberField(row, "scannedPassiveLogs", "scanned_passive_logs") ?? 0,
    createdOrUpdatedNetworkEndpoints:
      nullableNumberField(
        row,
        "createdOrUpdatedNetworkEndpoints",
        "created_or_updated_network_endpoints"
      ) ?? 0,
    createdOrUpdatedWebOrigins:
      nullableNumberField(row, "createdOrUpdatedWebOrigins", "created_or_updated_web_origins") ?? 0,
    createdOrUpdatedObservations:
      nullableNumberField(row, "createdOrUpdatedObservations", "created_or_updated_observations") ??
      0,
    skippedRelativeUrls:
      nullableNumberField(row, "skippedRelativeUrls", "skipped_relative_urls") ?? 0,
    skippedMalformedUrls:
      nullableNumberField(row, "skippedMalformedUrls", "skipped_malformed_urls") ?? 0,
    skippedMissingEndpoint:
      nullableNumberField(row, "skippedMissingEndpoint", "skipped_missing_endpoint") ?? 0,
    skippedUnsupportedScheme:
      nullableNumberField(row, "skippedUnsupportedScheme", "skipped_unsupported_scheme") ?? 0,
    inferredObservations:
      nullableNumberField(row, "inferredObservations", "inferred_observations") ?? 0,
    confirmedObservations:
      nullableNumberField(row, "confirmedObservations", "confirmed_observations") ?? 0,
  };
}

function normalizeBackendSurfaceSummary(value: unknown): BackendSurfaceSummaryDto {
  const row = asRecord(value);
  return {
    endpointCount: nullableNumberField(row, "endpointCount", "endpoint_count") ?? 0,
    webOriginCount: nullableNumberField(row, "webOriginCount", "web_origin_count") ?? 0,
    observationCount: nullableNumberField(row, "observationCount", "observation_count") ?? 0,
    inferredObservationCount:
      nullableNumberField(row, "inferredObservationCount", "inferred_observation_count") ?? 0,
    confirmedObservationCount:
      nullableNumberField(row, "confirmedObservationCount", "confirmed_observation_count") ?? 0,
    relatedDomainCount: nullableNumberField(row, "relatedDomainCount", "related_domain_count") ?? 0,
    unassignedCount: nullableNumberField(row, "unassignedCount", "unassigned_count") ?? 0,
    urlCount: presentNumberField(row, "urlCount", "url_count"),
    apiCount: presentNumberField(row, "apiCount", "api_count"),
    jsCount: presentNumberField(row, "jsCount", "js_count"),
    paramCount: presentNumberField(row, "paramCount", "param_count"),
    directoryEntryCount: presentNumberField(row, "directoryEntryCount", "directory_entry_count"),
    passiveLogCount: presentNumberField(row, "passiveLogCount", "passive_log_count"),
    evidenceCount: presentNumberField(row, "evidenceCount", "evidence_count"),
    contentUnassignedCount: presentNumberField(
      row,
      "contentUnassignedCount",
      "content_unassigned_count"
    ),
    contentUnmatchedOriginCount: presentNumberField(
      row,
      "contentUnmatchedOriginCount",
      "content_unmatched_origin_count"
    ),
  };
}

export function normalizeBackendSurfaceHierarchy(value: unknown): BackendSurfaceHierarchyDto {
  const row = asRecord(value);
  return {
    rootTarget: normalizeBackendSurfaceTarget(get(row, "rootTarget", "root_target")),
    mode: stringField(row, "mode"),
    dataSource: stringField(row, "dataSource", "data_source"),
    generatedAt: nullableNumberField(row, "generatedAt", "generated_at") ?? 0,
    endpoints: listOf(get(row, "endpoints"), normalizeBackendNetworkEndpoint),
    webOrigins: listOf(get(row, "webOrigins", "web_origins"), normalizeBackendWebOrigin),
    observations: listOf(get(row, "observations"), normalizeBackendWebOriginObservation),
    relatedDomains: listOf(
      get(row, "relatedDomains", "related_domains"),
      normalizeBackendRelatedDomain
    ),
    unassignedWebData: normalizeBackendUnassignedWebData(
      get(row, "unassignedWebData", "unassigned_web_data")
    ),
    summary: normalizeBackendSurfaceSummary(get(row, "summary")),
    fallbackReason: nullableStringField(row, "fallbackReason", "fallback_reason"),
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

export async function targetSurfaceHierarchyGet(
  targetId: string,
  includeRelated = true
): Promise<BackendSurfaceHierarchyDto> {
  return normalizeBackendSurfaceHierarchy(
    await invoke("target_surface_hierarchy_get", { targetId, includeRelated })
  );
}

/**
 * Build backend surface identity rows from existing legacy data. Additive and
 * idempotent (never mutates legacy tables). Pass the current project path and,
 * when known, the target's organization id to scope the backfill.
 */
export async function surfaceIdentityBackfill(
  projectPath?: string | null,
  organizationId?: string | null
): Promise<SurfaceIdentityBackfillSummary> {
  return normalizeSurfaceIdentityBackfillSummary(
    await invoke("target_surface_identity_backfill", {
      projectPath: projectPath ?? null,
      organizationId: organizationId ?? null,
    })
  );
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
