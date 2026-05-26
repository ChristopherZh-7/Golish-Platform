import { invoke } from "@/lib/api/client";

/**
 * Evidence Ledger MCP resource access (Phase 1b · Doc 2 §3).
 *
 * Backed by Tauri command `evidence_read` exposed via
 * `commands_facade/evidence.rs`. Read-only entry point: gives the LLM (and the
 * UI timeline) a sanitized view of one `audit_role='evidence'` row plus its
 * current scope classification, **without** ever inlining the raw tool output
 * into the LLM context.
 *
 * Type mirror of:
 *   backend/crates/golish/src/tools/evidence.rs
 *     - SummaryLevel
 *     - EvidenceFreshness
 *     - EvidenceSummary
 *     - ReadEvidenceRequest
 *
 * Naming follows AGENTS.md §I4 `<domain>_<verb>_<object>` for the backend
 * command name, but the frontend wrapper uses idiomatic camelCase
 * (`evidenceApi.read`) per existing patterns in `frontend/lib/api/`.
 */

/** 3 摘要档 (Doc 2 §3.1). `structured` 是默认值. */
export type SummaryLevel = "headline" | "structured" | "raw";

/** 三态新鲜度 (Doc 2 §3.1). */
export type EvidenceFreshness = "fresh" | "stale" | "expired";

/** IFC scope label 三态 (Doc 1 §4.1). */
export type EvidenceScopeLabel = "in_scope" | "out_of_scope" | "derived_from_out_of_scope";

export interface ReadEvidenceRequest {
  evidence_audit_id: number;
  summary_level?: SummaryLevel;
}

export interface EvidenceSummary {
  evidence_audit_id: number;
  kind: string;
  subject: string;
  /** ISO 8601 timestamp string (`chrono::DateTime<Utc>` JSON repr). */
  as_of_timestamp: string;
  freshness: EvidenceFreshness;
  scope_label: EvidenceScopeLabel;
  /**
   * Per-kind structured parser output (Doc 2 §4.1 `parse_structured`). `null`
   * when the kind has no structural parser or `summary_level=headline`.
   */
  structured: unknown | null;
  /** Single-line headline, always sanitize-pipeline processed. */
  headline: string;
  /** Sanitized raw text. Only populated when `summary_level=raw`. */
  raw_truncated: string | null;
}

/**
 * `evidenceApi.read(request)` calls Tauri `evidence_read`.
 *
 * Errors:
 *   - `NotFound` when `evidence_audit_id` does not match an `audit_role='evidence'`
 *     row, or the row is `status='abandoned'`.
 *   - `Database` on connection / SQL failure (auto-wrapped by `GolishError`).
 */
export const evidenceApi = {
  read: (request: ReadEvidenceRequest) =>
    invoke<EvidenceSummary>("evidence_read", { request }),
};
