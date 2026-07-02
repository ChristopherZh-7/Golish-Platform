import type { DirectoryEntry } from "@/lib/pentest/api";
import type { PortInfo, Target } from "@/lib/pentest/types";
import type {
  ApiEndpoint,
  AuditRow,
  Fingerprint,
  JsAnalysisResult,
  PassiveScanLog,
  TargetAsset,
  TimelineEntry,
} from "@/lib/security-analysis";
import { countEndpointParams, getEndpointParamNames } from "./endpointParams";
import { buildSitemapItems } from "./surfaceModel";
import type { SitemapItem } from "./types";

export type SurfaceMode = "ip" | "domain" | "url" | "other";
export type OriginScheme = "http" | "https" | "unknown";
export type HostType = "domain" | "ip";
export type SurfaceConfidence = "confirmed" | "inferred";
export type EndpointTransport = "tcp" | "udp" | "unknown";
export type IdentitySource = "backend_identity" | "frontend_inferred";
/**
 * Where a WebOrigin's *display counts* come from. Independent of `IdentitySource`:
 * a `backend_identity` origin can still fall back to `frontend_content_inferred`
 * counts when the backend payload omitted `contentCounts`.
 */
export type ContentCountSource = "backend_content_counts" | "frontend_content_inferred";
export type EndpointSource =
  | "target_ports"
  | "target_assets"
  | "url_inferred"
  | "backend_identity"
  | "frontend_inferred"
  | "unknown";
export type SurfaceHierarchyDataSource =
  | "frontend_inferred"
  | "backend_identity"
  | "legacy_fallback"
  | "backend_unavailable";

export interface WebOriginKey {
  id: string;
  origin: string;
  scheme: OriginScheme;
  host: string;
  port: number;
  hostType: HostType;
}

export interface SurfaceParamVM {
  id: string;
  endpointId: string;
  method: string;
  url: string;
  name: string;
  source: "api_endpoint";
}

export interface SurfaceEvidenceRef {
  id: string;
  source: string;
  label: string;
  url: string | null;
  capturePath: string | null;
  confidence: SurfaceConfidence;
  raw: unknown;
}

export type RelatedDomainVM = {
  id: string;
  value: string;
  type: Target["type"];
  scope: Target["scope"];
  realIp: string;
  target: Target;
  webOriginIds: string[];
};

export type NetworkEndpointVM = {
  id: string;
  ip?: string;
  port: number;
  transport: EndpointTransport;
  state?: string;
  service?: string;
  serviceName?: string;
  serviceProduct?: string;
  serviceVersion?: string;
  tls?: boolean;
  source: EndpointSource;
  webOriginIds: string[];
  observationIds?: string[];
  confidence: SurfaceConfidence;
};

/**
 * Lightweight backend content ref (Phase 2.5C). Mirrors
 * `BackendWebOriginContentRefDto`; used only to render a compact list in the
 * WebOrigin detail when the full frontend legacy rows are not loaded. Never
 * promoted into full `ApiEndpoint` / `JsAnalysisResult` rows.
 */
export type WebOriginContentRef = {
  kind: string;
  id: string;
  url: string;
  method: string | null;
  statusCode: number | null;
  capturePath: string | null;
  source: string | null;
};

export type WebOriginObservationVM = {
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
  capturePath: string | null;
  observedAt: number;
  confidence: SurfaceConfidence;
  source: string;
};

export type WebOriginVM = {
  id: string;
  backendId?: string;
  origin: string;
  scheme: OriginScheme;
  host: string;
  port: number;
  hostType: HostType;
  endpointIds: string[];
  observationIds: string[];

  urls: SitemapItem[];
  apiEndpoints: ApiEndpoint[];
  jsResources: JsAnalysisResult[];
  params: SurfaceParamVM[];
  directoryEntries: DirectoryEntry[];
  fingerprints: Fingerprint[];
  evidence: SurfaceEvidenceRef[];

  counts: {
    urls: number;
    apis: number;
    js: number;
    params: number;
    directoryEntries: number;
    findings: number;
    evidence: number;
    /**
     * Passive scan log count for this origin. Only populated from backend
     * `contentCounts.passiveLogCount`; the frontend inferred layer has no
     * separate passive-log count and leaves it at 0.
     */
    passiveLogs: number;
  };

  confidence: SurfaceConfidence;
  source: IdentitySource;
  /** Source of the display `counts` above (backend aggregation vs frontend rows). */
  contentCountSource: ContentCountSource;
  backendIdentityOnly?: boolean;
  /**
   * Bounded lightweight backend content refs (Phase 2.5C). Empty for
   * frontend-inferred origins and pre-2.5C backend payloads. The detail view
   * renders these when the full frontend rows (`apiEndpoints`/`jsResources`/…)
   * are not loaded, so a backend-only origin is not a dead end.
   */
  contentRefs: WebOriginContentRef[];
};

export type SurfaceHierarchyVM = {
  rootTarget: Target;
  mode: SurfaceMode;
  dataSource: SurfaceHierarchyDataSource;
  fallbackReason?: string | null;

  endpoints: NetworkEndpointVM[];
  webOrigins: WebOriginVM[];
  observations: WebOriginObservationVM[];
  relatedDomains: RelatedDomainVM[];

  unassignedWebData: {
    urls: unknown[];
    apis: unknown[];
    js: unknown[];
    params: unknown[];
    reason: string;
  };

  summary: {
    endpointCount: number;
    webOriginCount: number;
    domainCount: number;
    urlCount: number;
    apiCount: number;
    jsCount: number;
    paramCount: number;
    findingCount: number;
    evidenceCount: number;
    // Phase 2.5B content aggregation extras. Present when backend content
    // summary counts drove the merge; `undefined` when only frontend inferred
    // counts were available.
    directoryEntryCount?: number;
    passiveLogCount?: number;
    contentUnassignedCount?: number;
    contentUnmatchedOriginCount?: number;
    contentCountSource?: ContentCountSource;
  };
};

export interface BuildSurfaceHierarchyInput {
  rootTarget: Target;
  servicePorts?: PortInfo[];
  relatedDomains?: Target[];
  relatedWebTargets?: Target[];
  assets?: TargetAsset[];
  apiEndpoints?: ApiEndpoint[];
  jsResults?: JsAnalysisResult[];
  directoryEntries?: DirectoryEntry[];
  fingerprints?: Fingerprint[];
  passiveScans?: PassiveScanLog[];
  timeline?: TimelineEntry[];
  logs?: AuditRow[];
}

const UNASSIGNED_REASON = "这些数据缺少完整 URL、host 或 port，无法可靠归属到某个 Web Origin。";
const SOURCE_PRIORITY: Record<EndpointSource, number> = {
  unknown: 0,
  url_inferred: 1,
  frontend_inferred: 1,
  target_assets: 2,
  target_ports: 3,
  backend_identity: 4,
};

function defaultPortForScheme(scheme: OriginScheme): number {
  if (scheme === "http") return 80;
  if (scheme === "https") return 443;
  return 0;
}

function normalizeHost(host: string): string {
  return host.trim().replace(/^\[/, "").replace(/\]$/, "").replace(/\.$/, "").toLowerCase();
}

function isIpHost(host: string): boolean {
  const normalized = normalizeHost(host);
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(normalized)) return true;
  return normalized.includes(":") && !/[a-z]/i.test(normalized);
}

function normalizeScheme(scheme: string | null | undefined): OriginScheme {
  const normalized = (scheme ?? "").replace(/:$/, "").toLowerCase();
  if (normalized === "http" || normalized === "https") return normalized;
  return "unknown";
}

function normalizeTransport(transport: string | null | undefined): EndpointTransport {
  const normalized = (transport ?? "").toLowerCase();
  if (normalized === "tcp" || normalized === "udp") return normalized;
  return "unknown";
}

function normalizePort(port: number | string | null | undefined, scheme?: OriginScheme): number {
  if (typeof port === "number" && Number.isFinite(port) && port > 0) return Math.round(port);
  if (typeof port === "string" && port.trim()) {
    const parsed = Number(port);
    if (Number.isFinite(parsed) && parsed > 0) return Math.round(parsed);
  }
  return scheme ? defaultPortForScheme(scheme) : 0;
}

export function normalizeOriginKey(
  scheme: string | null | undefined,
  host: string,
  port?: number | string | null
): string {
  const normalizedScheme = normalizeScheme(scheme);
  const normalizedHost = normalizeHost(host);
  const normalizedPort = normalizePort(port, normalizedScheme);
  return `${normalizedScheme}://${normalizedHost}:${normalizedPort}`;
}

export function normalizeEndpointKey(
  ip: string | null | undefined,
  port: number | string | null | undefined,
  transport: string | null | undefined
): string {
  return `${normalizeHost(ip || "unknown")}:${normalizePort(port)}:${normalizeTransport(transport)}`;
}

function originFromParts(
  scheme: string | null | undefined,
  host: string,
  port?: number | string | null
): WebOriginKey | null {
  const normalizedScheme = normalizeScheme(scheme);
  if (normalizedScheme === "unknown") return null;
  const normalizedHost = normalizeHost(host);
  const normalizedPort = normalizePort(port, normalizedScheme);
  if (!normalizedHost || normalizedPort <= 0) return null;
  const id = normalizeOriginKey(normalizedScheme, normalizedHost, normalizedPort);
  return {
    id,
    origin: id,
    scheme: normalizedScheme,
    host: normalizedHost,
    port: normalizedPort,
    hostType: isIpHost(normalizedHost) ? "ip" : "domain",
  };
}

function fallbackOrigin(fallback?: WebOriginKey | string | null): WebOriginKey | null {
  if (!fallback) return null;
  if (typeof fallback === "string") return parseWebOrigin(fallback);
  return fallback;
}

export function parseWebOrigin(
  url: string | null | undefined,
  fallback?: WebOriginKey | string | null
): WebOriginKey | null {
  const trimmed = (url ?? "").trim();
  if (!trimmed) return null;
  if (!/^https?:\/\//i.test(trimmed)) return fallbackOrigin(fallback);

  try {
    const parsed = new URL(trimmed);
    return originFromParts(parsed.protocol, parsed.hostname, parsed.port);
  } catch {
    return null;
  }
}

function targetMode(target: Target): SurfaceMode {
  if (target.type === "ip") return "ip";
  if (target.type === "domain" || target.type === "wildcard") return "domain";
  if (target.type === "url") return "url";
  return "other";
}

function targetHost(target: Target): string {
  const value = target.value.trim();
  if (!value) return "";
  const parsed = parseWebOrigin(value);
  if (parsed) return parsed.host;
  return normalizeHost(value.replace(/^\*\./, "").split(/[/:?#]/, 1)[0] ?? value);
}

function targetIp(target: Target): string | undefined {
  if (target.type === "ip" && target.value.trim()) return normalizeHost(target.value);
  if (target.real_ip.trim()) return normalizeHost(target.real_ip);
  return undefined;
}

function targetResolvesToRootIp(target: Target, rootIp: string | undefined): boolean {
  return Boolean(rootIp && target.real_ip.trim() && normalizeHost(target.real_ip) === rootIp);
}

function portTls(port: PortInfo): boolean {
  const service = `${port.service ?? ""} ${port.webserver ?? ""} ${port.url ?? ""}`.toLowerCase();
  return service.includes("https") || service.includes("ssl") || service.includes("tls");
}

function assetTls(asset: TargetAsset): boolean {
  const service =
    `${asset.service ?? ""} ${asset.protocol ?? ""} ${asset.value ?? ""}`.toLowerCase();
  return service.includes("https") || service.includes("ssl") || service.includes("tls");
}

function createOrigin(key: WebOriginKey): WebOriginVM {
  return {
    ...key,
    endpointIds: [],
    observationIds: [],
    urls: [],
    apiEndpoints: [],
    jsResources: [],
    params: [],
    directoryEntries: [],
    fingerprints: [],
    evidence: [],
    counts: {
      urls: 0,
      apis: 0,
      js: 0,
      params: 0,
      directoryEntries: 0,
      findings: 0,
      evidence: 0,
      passiveLogs: 0,
    },
    confidence: "inferred",
    source: "frontend_inferred",
    contentCountSource: "frontend_content_inferred",
    contentRefs: [],
  };
}

function addUnique<T>(list: T[], item: T, key: (value: T) => string): void {
  const itemKey = key(item);
  if (!list.some((candidate) => key(candidate) === itemKey)) list.push(item);
}

function addEndpoint(
  endpointsById: Map<string, NetworkEndpointVM>,
  input: {
    ip?: string;
    port: number;
    transport?: string | null;
    service?: string | null;
    tls?: boolean;
    source: EndpointSource;
    confidence: SurfaceConfidence;
  }
): NetworkEndpointVM | null {
  if (!Number.isFinite(input.port) || input.port <= 0) return null;
  const id = normalizeEndpointKey(input.ip, input.port, input.transport);
  const current = endpointsById.get(id);
  if (current) {
    if (!current.service && input.service) current.service = input.service;
    current.tls = current.tls || input.tls || false;
    if (SOURCE_PRIORITY[input.source] > SOURCE_PRIORITY[current.source])
      current.source = input.source;
    if (input.confidence === "confirmed") current.confidence = "confirmed";
    return current;
  }

  const endpoint: NetworkEndpointVM = {
    id,
    ip: input.ip,
    port: input.port,
    transport: normalizeTransport(input.transport),
    state: "unknown",
    service: input.service || undefined,
    tls: input.tls,
    source: input.source,
    webOriginIds: [],
    observationIds: [],
    confidence: input.confidence,
  };
  endpointsById.set(id, endpoint);
  return endpoint;
}

function linkOriginEndpoint(origin: WebOriginVM, endpoint: NetworkEndpointVM): void {
  if (!origin.endpointIds.includes(endpoint.id)) origin.endpointIds.push(endpoint.id);
  if (!endpoint.webOriginIds.includes(origin.id)) endpoint.webOriginIds.push(origin.id);
  if (
    endpoint.confidence === "confirmed" &&
    endpoint.ip &&
    normalizeHost(endpoint.ip) === origin.host
  ) {
    origin.confidence = "confirmed";
  }
}

function sitemapByKindId(
  items: SitemapItem[],
  kind: SitemapItem["kind"],
  id: string
): SitemapItem[] {
  return items.filter((item) => item.kind === kind && item.id === id);
}

function addEndpointParams(origin: WebOriginVM, endpoint: ApiEndpoint): SurfaceParamVM[] {
  const params = getEndpointParamNames(endpoint.params).map((name) => ({
    id: `${endpoint.id}:${name}`,
    endpointId: endpoint.id,
    method: endpoint.method || "GET",
    url: endpoint.url || endpoint.path,
    name,
    source: "api_endpoint" as const,
  }));
  for (const param of params) addUnique(origin.params, param, (item) => item.id);
  return params;
}

function evidenceRef(input: {
  source: string;
  label: string;
  url?: string | null;
  capturePath?: string | null;
  confidence: SurfaceConfidence;
  raw: unknown;
  id?: string;
}): SurfaceEvidenceRef {
  return {
    id:
      input.id ||
      `${input.source}:${input.label}:${input.url ?? ""}:${input.capturePath ?? ""}`.slice(0, 180),
    source: input.source,
    label: input.label,
    url: input.url ?? null,
    capturePath: input.capturePath ?? null,
    confidence: input.confidence,
    raw: input.raw,
  };
}

function firstString(record: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function captureHostPort(
  capturePath: string | null | undefined
): { host: string; port: number } | null {
  const match = (capturePath ?? "").match(/(?:^|\/)captures\/([^/]+)\/(\d+)\//);
  if (!match) return null;
  return { host: normalizeHost(match[1]), port: Number(match[2]) };
}

function findOriginByCapture(
  capturePath: string | null | undefined,
  originsById: Map<string, WebOriginVM>
): WebOriginVM | null {
  const captured = captureHostPort(capturePath);
  if (!captured) return null;
  const matches = [...originsById.values()].filter(
    (origin) => origin.host === captured.host && origin.port === captured.port
  );
  return matches.length === 1 ? matches[0] : null;
}

function findOriginForUnknownEvidence(
  value: unknown,
  originsById: Map<string, WebOriginVM>
): WebOriginVM | null {
  if (typeof value === "string") {
    const parsed = parseWebOrigin(value);
    if (parsed) return originsById.get(parsed.id) ?? null;
    return findOriginByCapture(value, originsById);
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;

  const record = value as Record<string, unknown>;
  const url = firstString(record, ["url", "endpoint", "origin", "request_url", "requestUrl"]);
  const parsed = parseWebOrigin(url);
  if (parsed && originsById.has(parsed.id)) return originsById.get(parsed.id) ?? null;

  const capturePath = firstString(record, ["capturePath", "capture_path", "filePath", "file_path"]);
  return findOriginByCapture(capturePath, originsById);
}

function attachPassiveScanEvidence(
  originsById: Map<string, WebOriginVM>,
  passiveScans: PassiveScanLog[]
): void {
  for (const scan of passiveScans) {
    const parsed = parseWebOrigin(scan.url);
    const origin = parsed ? originsById.get(parsed.id) : null;
    if (!origin) continue;
    addUnique(
      origin.evidence,
      evidenceRef({
        id: scan.id,
        source: scan.toolUsed || "passive_scan",
        label: scan.testType || scan.result || scan.evidence || "passive scan",
        url: scan.url,
        confidence: "confirmed",
        raw: scan,
      }),
      (item) => item.id
    );
    if (["vulnerable", "potential"].includes(scan.result)) origin.counts.findings += 1;
  }
}

function attachTimelineEvidence(
  originsById: Map<string, WebOriginVM>,
  timeline: TimelineEntry[],
  logs: AuditRow[]
): void {
  for (const entry of timeline) {
    const origin = findOriginForUnknownEvidence(entry.detail, originsById);
    if (!origin) continue;
    addUnique(
      origin.evidence,
      evidenceRef({
        source: entry.source,
        label: entry.details || entry.event,
        url: firstString(entry.detail, ["url", "endpoint", "origin"]),
        capturePath: firstString(entry.detail, ["capturePath", "capture_path"]),
        confidence: "inferred",
        raw: entry,
      }),
      (item) => item.id
    );
  }
  for (const log of logs) {
    const origin = findOriginForUnknownEvidence(log.detail, originsById);
    if (!origin) continue;
    addUnique(
      origin.evidence,
      evidenceRef({
        id: String(log.id),
        source: log.toolName || log.source || "operation_log",
        label: log.details || log.action,
        url: firstString(log.detail, ["url", "endpoint", "origin"]),
        capturePath: firstString(log.detail, ["capturePath", "capture_path"]),
        confidence: "inferred",
        raw: log,
      }),
      (item) => item.id
    );
  }
}

function attachFingerprintEvidence(
  originsById: Map<string, WebOriginVM>,
  fingerprints: Fingerprint[]
): void {
  for (const fingerprint of fingerprints) {
    const origin =
      fingerprint.evidence
        .map((item) => findOriginForUnknownEvidence(item, originsById))
        .find(Boolean) ?? null;
    if (!origin) continue;
    addUnique(origin.fingerprints, fingerprint, (item) => item.id);
    addUnique(
      origin.evidence,
      evidenceRef({
        id: fingerprint.id,
        source: fingerprint.source || "fingerprint",
        label: [fingerprint.category, fingerprint.name, fingerprint.version]
          .filter(Boolean)
          .join(" · "),
        confidence: "inferred",
        raw: fingerprint,
      }),
      (item) => item.id
    );
  }
}

function hasUsefulEvidence(endpoint: ApiEndpoint): boolean {
  return Boolean(endpoint.capturePath);
}

function finalizeOriginCounts(origin: WebOriginVM): void {
  origin.counts = {
    urls: origin.urls.length,
    apis: origin.apiEndpoints.length,
    js: origin.jsResources.length,
    params: origin.params.length,
    directoryEntries: origin.directoryEntries.length,
    findings: origin.counts.findings,
    evidence: origin.evidence.length,
    passiveLogs: origin.counts.passiveLogs,
  };
  origin.endpointIds.sort();
}

export function buildSurfaceHierarchy(input: BuildSurfaceHierarchyInput): SurfaceHierarchyVM {
  const rootTarget = {
    ...input.rootTarget,
    ports: Array.isArray(input.rootTarget.ports) ? input.rootTarget.ports : [],
  };
  const mode = targetMode(rootTarget);
  const rootIp = targetIp(rootTarget);
  const servicePorts = input.servicePorts ?? rootTarget.ports;
  const assets = input.assets ?? [];
  const apiEndpoints = input.apiEndpoints ?? [];
  const jsResults = input.jsResults ?? [];
  const directoryEntries = input.directoryEntries ?? [];
  const fingerprints = input.fingerprints ?? [];
  const passiveScans = input.passiveScans ?? [];
  const timeline = input.timeline ?? [];
  const logs = input.logs ?? [];
  const sitemapItems = buildSitemapItems(apiEndpoints, jsResults, directoryEntries);
  const endpointsById = new Map<string, NetworkEndpointVM>();
  const originsById = new Map<string, WebOriginVM>();
  const unassignedWebData: SurfaceHierarchyVM["unassignedWebData"] = {
    urls: [],
    apis: [],
    js: [],
    params: [],
    reason: UNASSIGNED_REASON,
  };

  const ensureOrigin = (key: WebOriginKey): WebOriginVM => {
    let origin = originsById.get(key.id);
    if (!origin) {
      origin = createOrigin(key);
      originsById.set(key.id, origin);
    }
    return origin;
  };

  for (const port of servicePorts) {
    addEndpoint(endpointsById, {
      ip: rootIp,
      port: normalizePort(port.port),
      transport: port.protocol ?? "tcp",
      service: [port.service, port.webserver].filter(Boolean).join(" · ") || null,
      tls: portTls(port),
      source: "target_ports",
      confidence: "confirmed",
    });
  }

  for (const asset of assets) {
    if (!asset.port) continue;
    addEndpoint(endpointsById, {
      ip: rootIp || (isIpHost(asset.value) ? normalizeHost(asset.value) : undefined),
      port: asset.port,
      transport: asset.protocol,
      service: asset.service,
      tls: assetTls(asset),
      source: "target_assets",
      confidence: "confirmed",
    });
  }

  if (mode === "ip") {
    for (const relatedTarget of input.relatedWebTargets ?? input.relatedDomains ?? []) {
      if (relatedTarget.type !== "url") continue;
      const parsed = parseWebOrigin(relatedTarget.value);
      if (!parsed) continue;
      const hostMatchesRootIp = Boolean(rootIp && parsed.host === rootIp);
      if (!hostMatchesRootIp && !targetResolvesToRootIp(relatedTarget, rootIp)) continue;
      ensureOrigin(parsed);
    }
  }

  for (const endpoint of apiEndpoints) {
    const parsed = parseWebOrigin(endpoint.url || endpoint.path);
    if (!parsed) {
      unassignedWebData.apis.push(endpoint);
      for (const param of getEndpointParamNames(endpoint.params)) {
        unassignedWebData.params.push({
          id: `${endpoint.id}:${param}`,
          endpointId: endpoint.id,
          method: endpoint.method || "GET",
          url: endpoint.url || endpoint.path,
          name: param,
          source: "api_endpoint",
        } satisfies SurfaceParamVM);
      }
      continue;
    }
    const origin = ensureOrigin(parsed);
    addUnique(origin.apiEndpoints, endpoint, (item) => item.id);
    for (const item of sitemapByKindId(sitemapItems, "endpoint", endpoint.id)) {
      addUnique(
        origin.urls,
        item,
        (candidate) => `${candidate.kind}:${candidate.id}:${candidate.url}`
      );
    }
    addEndpointParams(origin, endpoint);
    if (hasUsefulEvidence(endpoint)) {
      addUnique(
        origin.evidence,
        evidenceRef({
          id: `${endpoint.id}:capture`,
          source: endpoint.source || "api_endpoint",
          label: `${endpoint.method || "GET"} ${endpoint.path || endpoint.url}`,
          url: endpoint.url,
          capturePath: endpoint.capturePath,
          confidence: "confirmed",
          raw: endpoint,
        }),
        (item) => item.id
      );
    }
  }

  for (const js of jsResults) {
    const parsed = parseWebOrigin(js.url);
    if (!parsed) {
      unassignedWebData.js.push(js);
      continue;
    }
    const origin = ensureOrigin(parsed);
    addUnique(origin.jsResources, js, (item) => item.id);
    for (const item of sitemapByKindId(sitemapItems, "script", js.id)) {
      addUnique(
        origin.urls,
        item,
        (candidate) => `${candidate.kind}:${candidate.id}:${candidate.url}`
      );
    }
    if (js.filePath) {
      addUnique(
        origin.evidence,
        evidenceRef({
          id: `${js.id}:file`,
          source: "js_file",
          label: js.filename || js.url,
          url: js.url,
          capturePath: js.filePath,
          confidence: "confirmed",
          raw: js,
        }),
        (item) => item.id
      );
    }
  }

  for (const entry of directoryEntries) {
    const parsed = parseWebOrigin(entry.url);
    if (!parsed) {
      unassignedWebData.urls.push(entry);
      continue;
    }
    const origin = ensureOrigin(parsed);
    addUnique(origin.directoryEntries, entry, (item) => item.id);
    for (const item of sitemapByKindId(sitemapItems, "directory", entry.id)) {
      addUnique(
        origin.urls,
        item,
        (candidate) => `${candidate.kind}:${candidate.id}:${candidate.url}`
      );
    }
  }

  const rootIpCandidates = new Set([
    rootIp,
    normalizeHost(rootTarget.value),
    normalizeHost(rootTarget.real_ip),
  ]);
  if (mode === "ip") {
    for (const origin of originsById.values()) {
      const matches = [...endpointsById.values()].filter(
        (endpoint) => endpoint.port === origin.port
      );
      for (const endpoint of matches) linkOriginEndpoint(origin, endpoint);
      if (matches.length > 0) continue;
      if (origin.hostType === "ip" && rootIpCandidates.has(origin.host)) {
        const inferred = addEndpoint(endpointsById, {
          ip: origin.host,
          port: origin.port,
          transport: "tcp",
          service: origin.scheme === "https" ? "https" : origin.scheme === "http" ? "http" : null,
          tls: origin.scheme === "https",
          source: "url_inferred",
          confidence: "inferred",
        });
        if (inferred) linkOriginEndpoint(origin, inferred);
      }
    }
  }

  attachPassiveScanEvidence(originsById, passiveScans);
  attachTimelineEvidence(originsById, timeline, logs);
  attachFingerprintEvidence(originsById, fingerprints);

  const webOrigins = [...originsById.values()].sort((a, b) => a.origin.localeCompare(b.origin));
  for (const origin of webOrigins) finalizeOriginCounts(origin);

  const relatedDomains = (input.relatedDomains ?? []).map((target) => {
    const host = targetHost(target);
    return {
      id: target.id,
      value: target.value,
      type: target.type,
      scope: target.scope,
      realIp: target.real_ip,
      target,
      webOriginIds: webOrigins.filter((origin) => origin.host === host).map((origin) => origin.id),
    };
  });

  const findingCount =
    passiveScans.filter((scan) => ["vulnerable", "potential"].includes(scan.result)).length +
    timeline.filter((entry) => entry.source === "findings").length;
  const evidenceCount =
    timeline.length +
    logs.length +
    apiEndpoints.filter((endpoint) => endpoint.capturePath).length +
    jsResults.filter((js) => js.filePath).length +
    fingerprints.reduce((sum, fingerprint) => sum + fingerprint.evidence.length, 0);

  return {
    rootTarget,
    mode,
    dataSource: "frontend_inferred",
    fallbackReason: null,
    endpoints: [...endpointsById.values()].sort(
      (a, b) => a.port - b.port || a.id.localeCompare(b.id)
    ),
    webOrigins,
    observations: [],
    relatedDomains,
    unassignedWebData,
    summary: {
      endpointCount: endpointsById.size,
      webOriginCount: webOrigins.length,
      domainCount: relatedDomains.length,
      urlCount: sitemapItems.length,
      apiCount: apiEndpoints.length,
      jsCount: jsResults.length,
      paramCount: countEndpointParams(apiEndpoints),
      findingCount,
      evidenceCount,
      contentCountSource: "frontend_content_inferred",
    },
  };
}
