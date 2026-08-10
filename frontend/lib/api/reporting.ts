/** Canonical cited Reporting read model and explicit trusted publication IPC. */

import type { ReportArtifactView } from "@/lib/generated/ReportArtifactView";
import type { ReportClaimValue } from "@/lib/generated/ReportClaimValue";
import type { ReportingFinalizeRequest } from "@/lib/generated/ReportingFinalizeRequest";
import type { ReportingScopeRequest } from "@/lib/generated/ReportingScopeRequest";
import type { ReportReadModelView } from "@/lib/generated/ReportReadModelView";
import type { ReportRevisionView } from "@/lib/generated/ReportRevisionView";
import { invoke } from "./client";

export type {
  ReportArtifactView,
  ReportClaimValue,
  ReportReadModelView,
  ReportRevisionView,
  ReportingFinalizeRequest,
  ReportingScopeRequest,
};

export function getReportReadModel(
  request: ReportingScopeRequest
): Promise<ReportReadModelView | null> {
  return invoke("reporting_get_read_model", { request });
}

export function buildReportReadModel(request: ReportingScopeRequest): Promise<ReportReadModelView> {
  return invoke("reporting_build_read_model", { request });
}

export function listReportRevisions(request: ReportingScopeRequest): Promise<ReportRevisionView[]> {
  return invoke("reporting_list_revisions", { request });
}

export function getReportArtifacts(request: ReportingScopeRequest): Promise<ReportArtifactView[]> {
  return invoke("reporting_get_artifacts", { request });
}

export function finalizeReportRevision(
  request: ReportingFinalizeRequest
): Promise<ReportReadModelView> {
  return invoke("reporting_finalize_revision", { request });
}
