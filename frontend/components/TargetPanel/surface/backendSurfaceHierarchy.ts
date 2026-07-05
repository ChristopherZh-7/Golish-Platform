import type {
  BackendCrawlObservationDto,
  BackendNetworkEndpointDto,
  BackendSurfaceHierarchyDto,
  BackendWebOriginContentCountsDto,
  BackendWebOriginContentRefDto,
  BackendWebOriginDto,
  BackendWebOriginObservationDto,
} from "@/lib/security-analysis";
import type {
  ContentCountSource,
  CrawlObservationVM,
  HostType,
  NetworkEndpointVM,
  OriginScheme,
  SurfaceConfidence,
  SurfaceHierarchyVM,
  WebOriginContentRef,
  WebOriginObservationVM,
  WebOriginVM,
} from "./surfaceHierarchy";
import { normalizeOriginKey } from "./surfaceHierarchy";

export type BackendHierarchyMergeStatus = "backend_identity" | "fallback";

export interface BackendHierarchyMergeResult {
  hierarchy: SurfaceHierarchyVM;
  status: BackendHierarchyMergeStatus;
  message: string;
  fallbackReason: string | null;
}

const BACKEND_ENABLED_MESSAGE =
  "Backend identity hierarchy enabled. Web content is still matched by Web Origin.";
const FRONTEND_FALLBACK_MESSAGE =
  "Using frontend-inferred hierarchy. Backend identity hierarchy is unavailable or empty.";

function normalizeScheme(value: string): OriginScheme {
  const normalized = value.toLowerCase();
  if (normalized === "http" || normalized === "https") return normalized;
  return "unknown";
}

function normalizeHostType(value: string): HostType {
  return value.toLowerCase() === "ip" ? "ip" : "domain";
}

function confidenceFromBackend(value: number): SurfaceConfidence {
  return value >= 0.5 ? "confirmed" : "inferred";
}

function serviceLabel(endpoint: BackendNetworkEndpointDto): string | undefined {
  const parts = [endpoint.serviceName, endpoint.serviceProduct, endpoint.serviceVersion]
    .map((part) => part?.trim())
    .filter(Boolean);
  return parts.length > 0 ? parts.join(" ") : undefined;
}

function hasLegacyContent(origin: WebOriginVM | undefined): boolean {
  if (!origin) return false;
  return (
    origin.urls.length > 0 ||
    origin.apiEndpoints.length > 0 ||
    origin.jsResources.length > 0 ||
    origin.params.length > 0 ||
    origin.directoryEntries.length > 0 ||
    origin.fingerprints.length > 0 ||
    origin.evidence.length > 0
  );
}

function frontendContentCounts(origin: WebOriginVM | undefined): WebOriginVM["counts"] {
  return {
    urls: origin?.urls.length ?? 0,
    apis: origin?.apiEndpoints.length ?? 0,
    js: origin?.jsResources.length ?? 0,
    params: origin?.params.length ?? 0,
    directoryEntries: origin?.directoryEntries.length ?? 0,
    findings: origin?.counts.findings ?? 0,
    evidence: origin?.evidence.length ?? 0,
    passiveLogs: origin?.counts.passiveLogs ?? 0,
  };
}

/**
 * Build display counts from backend `contentCounts`. Detail rows still come
 * from the frontend legacy arrays, so we never sum backend + frontend — backend
 * counts fully replace the display numbers, and `findings` (which the backend
 * does not aggregate) is preserved from the frontend inferred layer.
 */
function backendContentCounts(
  backend: BackendWebOriginContentCountsDto,
  frontendOrigin: WebOriginVM | undefined
): WebOriginVM["counts"] {
  return {
    urls: backend.urlCount,
    apis: backend.apiCount,
    js: backend.jsCount,
    params: backend.paramCount,
    directoryEntries: backend.directoryEntryCount,
    findings: frontendOrigin?.counts.findings ?? 0,
    evidence: backend.evidenceCount,
    passiveLogs: backend.passiveLogCount,
  };
}

function mapContentRef(ref: BackendWebOriginContentRefDto): WebOriginContentRef {
  return {
    kind: ref.kind,
    id: ref.id,
    url: ref.url,
    method: ref.method,
    statusCode: ref.statusCode,
    capturePath: ref.capturePath,
    source: ref.source,
  };
}

function mapCrawlObservation(observation: BackendCrawlObservationDto): CrawlObservationVM {
  return {
    id: observation.id,
    originTargetId: observation.originTargetId,
    originUrl: observation.originUrl,
    originKey: observation.originKey,
    observedUrl: observation.observedUrl,
    observedHost: observation.observedHost,
    observedPath: observation.observedPath,
    kind: observation.kind || "url",
    sameOrigin: observation.sameOrigin,
    sourceTool: observation.sourceTool || "crawler",
    sourceRecordId: observation.sourceRecordId,
    evidenceId: observation.evidenceId,
    metadata: observation.metadata,
    discoveredAt: observation.discoveredAt,
    updatedAt: observation.updatedAt,
  };
}

function frontendFallbackHierarchy(
  frontend: SurfaceHierarchyVM,
  fallbackReason: string | null
): SurfaceHierarchyVM {
  if (frontend.mode !== "ip") return frontend;
  return {
    ...frontend,
    dataSource: "frontend_inferred",
    fallbackReason,
    endpoints: frontend.endpoints.map((endpoint) => ({
      ...endpoint,
      source: "frontend_inferred",
      confidence: "inferred",
    })),
    webOrigins: frontend.webOrigins.map((origin) => ({
      ...origin,
      source: "frontend_inferred",
      confidence: "inferred",
      contentCountSource: "frontend_content_inferred",
      backendIdentityOnly: false,
      contentRefs: [],
      crawlObservations: [],
    })),
    observations: [],
  };
}

function backendCanDriveIdentity(backend: BackendSurfaceHierarchyDto | null | undefined): boolean {
  if (!backend) return false;
  if (backend.mode !== "ip" || backend.dataSource !== "backend_identity") return false;
  return backend.endpoints.length > 0 || backend.webOrigins.length > 0;
}

function originKey(origin: BackendWebOriginDto): string {
  return origin.origin || normalizeOriginKey(origin.scheme, origin.host, origin.port);
}

function mapBackendEndpoint(
  endpoint: BackendNetworkEndpointDto,
  originIdToKey: Map<string, string>
): NetworkEndpointVM {
  return {
    id: endpoint.id,
    ip: endpoint.ip,
    port: endpoint.port,
    transport:
      endpoint.transport === "udp" ? "udp" : endpoint.transport === "tcp" ? "tcp" : "unknown",
    state: endpoint.state || "unknown",
    service: serviceLabel(endpoint),
    serviceName: endpoint.serviceName ?? undefined,
    serviceProduct: endpoint.serviceProduct ?? undefined,
    serviceVersion: endpoint.serviceVersion ?? undefined,
    tls: endpoint.tlsDetected,
    source: "backend_identity",
    webOriginIds: endpoint.webOriginIds
      .map((id) => originIdToKey.get(id))
      .filter((id): id is string => Boolean(id)),
    observationIds: endpoint.observationIds,
    confidence: confidenceFromBackend(endpoint.confidence),
  };
}

function mapBackendObservation(
  observation: BackendWebOriginObservationDto,
  originIdToKey: Map<string, string>
): WebOriginObservationVM {
  return {
    id: observation.id,
    webOriginId: originIdToKey.get(observation.webOriginId) ?? observation.webOriginId,
    networkEndpointId: observation.networkEndpointId,
    targetId: observation.targetId,
    observedIp: observation.observedIp,
    sni: observation.sni,
    hostHeader: observation.hostHeader,
    statusCode: observation.statusCode,
    title: observation.title,
    finalUrl: observation.finalUrl,
    capturePath: observation.capturePath,
    observedAt: observation.observedAt,
    confidence: confidenceFromBackend(observation.confidence),
    source: observation.source,
  };
}

function mapBackendOrigin(
  backendOrigin: BackendWebOriginDto,
  frontendOrigin: WebOriginVM | undefined,
  backendEndpointIds: Set<string>
): WebOriginVM {
  const key = originKey(backendOrigin);
  const endpointIds = backendOrigin.endpointIds.filter((id) => backendEndpointIds.has(id));
  const linkedEndpointIds =
    endpointIds.length > 0 ? endpointIds : (frontendOrigin?.endpointIds ?? []);
  const legacyLinked = hasLegacyContent(frontendOrigin);
  // Counts priority: backend contentCounts (when present) > frontend origin
  // counts > 0. `contentCounts == null` means the backend payload omitted the
  // Phase 2.5A aggregation (older backend), so keep frontend inferred counts.
  const backendCounts = backendOrigin.contentCounts;
  const contentCountSource: ContentCountSource = backendCounts
    ? "backend_content_counts"
    : "frontend_content_inferred";
  const counts = backendCounts
    ? backendContentCounts(backendCounts, frontendOrigin)
    : frontendContentCounts(frontendOrigin);

  return {
    id: key,
    backendId: backendOrigin.id,
    origin: key,
    scheme: normalizeScheme(backendOrigin.scheme),
    host: backendOrigin.host,
    hostType: normalizeHostType(backendOrigin.hostType),
    port: backendOrigin.port,
    endpointIds: linkedEndpointIds,
    observationIds: backendOrigin.observationIds,
    urls: frontendOrigin?.urls ?? [],
    apiEndpoints: frontendOrigin?.apiEndpoints ?? [],
    jsResources: frontendOrigin?.jsResources ?? [],
    params: frontendOrigin?.params ?? [],
    directoryEntries: frontendOrigin?.directoryEntries ?? [],
    fingerprints: frontendOrigin?.fingerprints ?? [],
    evidence: frontendOrigin?.evidence ?? [],
    counts,
    confidence: confidenceFromBackend(backendOrigin.confidence),
    source: "backend_identity",
    contentCountSource,
    backendIdentityOnly: !legacyLinked,
    contentRefs: backendOrigin.refs.map(mapContentRef),
    crawlObservations: backendOrigin.crawlObservations.map(mapCrawlObservation),
  };
}

function frontendOnlyOrigin(origin: WebOriginVM, validEndpointIds: Set<string>): WebOriginVM {
  return {
    ...origin,
    endpointIds: origin.endpointIds.filter((id) => validEndpointIds.has(id)),
    source: "frontend_inferred",
    confidence: "inferred",
    contentCountSource: "frontend_content_inferred",
    backendIdentityOnly: false,
    contentRefs: [],
    crawlObservations: [],
  };
}

function combineUnassignedReason(
  frontend: SurfaceHierarchyVM,
  backend: BackendSurfaceHierarchyDto
): SurfaceHierarchyVM["unassignedWebData"] {
  const backendReason = backend.unassignedWebData.reason?.trim();
  if (!backendReason) return frontend.unassignedWebData;
  return {
    ...frontend.unassignedWebData,
    reason: `${frontend.unassignedWebData.reason} Backend: ${backendReason}`,
  };
}

/**
 * IP Overview / summary content counts prefer backend summary aggregation and
 * fall back to the frontend inferred summary per field. We never sum the two.
 * `findingCount` stays frontend-only (the backend does not aggregate findings).
 */
function mergeSummary(
  frontend: SurfaceHierarchyVM,
  backend: BackendSurfaceHierarchyDto,
  endpointCount: number,
  webOriginCount: number
): SurfaceHierarchyVM["summary"] {
  const { summary } = backend;
  const hasBackendContentCounts =
    summary.urlCount !== null ||
    summary.apiCount !== null ||
    summary.jsCount !== null ||
    summary.paramCount !== null ||
    summary.directoryEntryCount !== null ||
    summary.passiveLogCount !== null ||
    summary.evidenceCount !== null;

  return {
    ...frontend.summary,
    endpointCount,
    webOriginCount,
    urlCount: summary.urlCount ?? frontend.summary.urlCount,
    apiCount: summary.apiCount ?? frontend.summary.apiCount,
    jsCount: summary.jsCount ?? frontend.summary.jsCount,
    paramCount: summary.paramCount ?? frontend.summary.paramCount,
    evidenceCount: summary.evidenceCount ?? frontend.summary.evidenceCount,
    directoryEntryCount: summary.directoryEntryCount ?? undefined,
    passiveLogCount: summary.passiveLogCount ?? undefined,
    contentUnassignedCount: summary.contentUnassignedCount ?? undefined,
    contentUnmatchedOriginCount: summary.contentUnmatchedOriginCount ?? undefined,
    contentCountSource: hasBackendContentCounts
      ? "backend_content_counts"
      : "frontend_content_inferred",
  };
}

export function composeBackendSurfaceHierarchy(
  frontend: SurfaceHierarchyVM,
  backend: BackendSurfaceHierarchyDto | null | undefined
): BackendHierarchyMergeResult {
  if (frontend.mode !== "ip") {
    return {
      hierarchy: frontend,
      status: "fallback",
      message: FRONTEND_FALLBACK_MESSAGE,
      fallbackReason: backend?.fallbackReason ?? null,
    };
  }

  if (!backendCanDriveIdentity(backend)) {
    const fallbackReason = backend?.fallbackReason ?? null;
    return {
      hierarchy: frontendFallbackHierarchy(frontend, fallbackReason),
      status: "fallback",
      message: FRONTEND_FALLBACK_MESSAGE,
      fallbackReason,
    };
  }

  const usableBackend = backend as BackendSurfaceHierarchyDto;
  const frontendOriginsByKey = new Map(
    frontend.webOrigins.map((origin) => [origin.origin, origin])
  );
  const originIdToKey = new Map(
    usableBackend.webOrigins.map((origin) => [origin.id, originKey(origin)])
  );
  const endpoints =
    usableBackend.endpoints.length > 0
      ? usableBackend.endpoints
          .map((endpoint) => mapBackendEndpoint(endpoint, originIdToKey))
          .sort((a, b) => a.port - b.port || a.id.localeCompare(b.id))
      : frontend.endpoints.map((endpoint) => ({
          ...endpoint,
          source: "frontend_inferred" as const,
          confidence: "inferred" as const,
        }));
  const validEndpointIds = new Set(endpoints.map((endpoint) => endpoint.id));
  const backendOrigins = usableBackend.webOrigins.map((backendOrigin) =>
    mapBackendOrigin(
      backendOrigin,
      frontendOriginsByKey.get(originKey(backendOrigin)),
      validEndpointIds
    )
  );
  const backendOriginKeys = new Set(backendOrigins.map((origin) => origin.origin));
  const frontendOnlyOrigins = frontend.webOrigins
    .filter((origin) => !backendOriginKeys.has(origin.origin))
    .map((origin) => frontendOnlyOrigin(origin, validEndpointIds));
  const webOrigins = [...backendOrigins, ...frontendOnlyOrigins].sort((a, b) =>
    a.origin.localeCompare(b.origin)
  );
  const observations = usableBackend.observations.map((observation) =>
    mapBackendObservation(observation, originIdToKey)
  );

  return {
    hierarchy: {
      ...frontend,
      dataSource: "backend_identity",
      fallbackReason: usableBackend.fallbackReason,
      endpoints,
      webOrigins,
      observations,
      unassignedWebData: combineUnassignedReason(frontend, usableBackend),
      summary: mergeSummary(frontend, usableBackend, endpoints.length, webOrigins.length),
    },
    status: "backend_identity",
    message: BACKEND_ENABLED_MESSAGE,
    fallbackReason: usableBackend.fallbackReason,
  };
}
