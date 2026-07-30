/** Read-only Hypothesis Registry audit API. */

import type { InvestigationHypothesisDetailView } from "@/lib/generated/InvestigationHypothesisDetailView";
import type { InvestigationHypothesisGetRequest } from "@/lib/generated/InvestigationHypothesisGetRequest";
import type { InvestigationHypothesisListRequest } from "@/lib/generated/InvestigationHypothesisListRequest";
import type { InvestigationHypothesisListView } from "@/lib/generated/InvestigationHypothesisListView";
import type { InvestigationScopeRequest } from "@/lib/generated/InvestigationScopeRequest";
import type { InvestigationSummaryView } from "@/lib/generated/InvestigationSummaryView";
import { invoke } from "./client";

export type {
  InvestigationHypothesisDetailView,
  InvestigationHypothesisGetRequest,
  InvestigationHypothesisListRequest,
  InvestigationHypothesisListView,
  InvestigationScopeRequest,
  InvestigationSummaryView,
};

export const getInvestigationSummary = (
  request: InvestigationScopeRequest
): Promise<InvestigationSummaryView> =>
  invoke<InvestigationSummaryView>("investigation_get_summary", { request });

export const listInvestigationHypotheses = (
  request: InvestigationHypothesisListRequest
): Promise<InvestigationHypothesisListView> =>
  invoke<InvestigationHypothesisListView>("investigation_list_hypotheses", { request });

export const getInvestigationHypothesis = (
  request: InvestigationHypothesisGetRequest
): Promise<InvestigationHypothesisDetailView> =>
  invoke<InvestigationHypothesisDetailView>("investigation_get_hypothesis", { request });
