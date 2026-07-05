/**
 * Compat re-export — actual implementation lives at `@/lib/api/security-analysis`.
 *
 * Surface narrowed to symbols that are imported through this wrapper path
 * (M2.3 cleanup — previously `export *` re-exported the entire module).
 */

export {
  type ApiEndpoint,
  type AuditRow,
  apiEndpointsList,
  type BackendCrawlObservationDto,
  type BackendNetworkEndpointDto,
  type BackendRelatedDomainDto,
  type BackendSurfaceHierarchyDto,
  type BackendSurfaceSummaryDto,
  type BackendSurfaceTargetDto,
  type BackendUnassignedWebDataCountsDto,
  type BackendUnassignedWebDataDto,
  type BackendWebOriginContentCountsDto,
  type BackendWebOriginContentRefDto,
  type BackendWebOriginDto,
  type BackendWebOriginObservationDto,
  type Fingerprint,
  fingerprintsList,
  type JsAnalysisResult,
  jsAnalysisList,
  normalizeBackendSurfaceHierarchy,
  oplogList,
  oplogListByTarget,
  oplogSearch,
  type PassiveScanLog,
  passiveScansList,
  type SurfaceIdentityBackfillSummary,
  surfaceIdentityBackfill,
  type TargetAsset,
  type TimelineEntry,
  targetAssetsList,
  targetSurfaceHierarchyGet,
  targetTimeline,
} from "@/lib/api/security-analysis";
