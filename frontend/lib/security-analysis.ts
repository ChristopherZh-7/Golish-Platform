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
  type Fingerprint,
  fingerprintsList,
  type JsAnalysisResult,
  jsAnalysisList,
  oplogList,
  oplogListByTarget,
  oplogSearch,
  type PassiveScanLog,
  passiveScansList,
  type TargetAsset,
  type TimelineEntry,
  targetAssetsList,
  targetTimeline,
} from "@/lib/api/security-analysis";
