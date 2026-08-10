/** Exact unified Investigation projection and explicit stage-stop API. */

import type { InvestigationCampaignDetailRequest } from "@/lib/generated/InvestigationCampaignDetailRequest";
import type { InvestigationCampaignDetailResponse } from "@/lib/generated/InvestigationCampaignDetailResponse";
import type { InvestigationCampaignListRequest } from "@/lib/generated/InvestigationCampaignListRequest";
import type { InvestigationCampaignPageResponse } from "@/lib/generated/InvestigationCampaignPageResponse";
import type { InvestigationHypothesisDetailView } from "@/lib/generated/InvestigationHypothesisDetailView";
import type { InvestigationHypothesisGetRequest } from "@/lib/generated/InvestigationHypothesisGetRequest";
import type { InvestigationHypothesisListRequest } from "@/lib/generated/InvestigationHypothesisListRequest";
import type { InvestigationHypothesisListView } from "@/lib/generated/InvestigationHypothesisListView";
import type { InvestigationRequestStopRequest } from "@/lib/generated/InvestigationRequestStopRequest";
import type { InvestigationRequestStopResponse } from "@/lib/generated/InvestigationRequestStopResponse";
import type { InvestigationScopeRequest } from "@/lib/generated/InvestigationScopeRequest";
import type { InvestigationSummaryView } from "@/lib/generated/InvestigationSummaryView";
import type { InvestigationTimelineListRequest } from "@/lib/generated/InvestigationTimelineListRequest";
import type { InvestigationTimelinePageResponse } from "@/lib/generated/InvestigationTimelinePageResponse";
import { invoke } from "./client";

export type {
  InvestigationHypothesisDetailView,
  InvestigationHypothesisGetRequest,
  InvestigationHypothesisListRequest,
  InvestigationHypothesisListView,
  InvestigationCampaignDetailRequest,
  InvestigationCampaignDetailResponse,
  InvestigationCampaignListRequest,
  InvestigationCampaignPageResponse,
  InvestigationScopeRequest,
  InvestigationSummaryView,
  InvestigationTimelineListRequest,
  InvestigationTimelinePageResponse,
  InvestigationRequestStopRequest,
  InvestigationRequestStopResponse,
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

export const investigationListCampaigns = (
  request: InvestigationCampaignListRequest
): Promise<InvestigationCampaignPageResponse> =>
  invoke<InvestigationCampaignPageResponse>("investigation_list_campaigns", { request });

export const investigationGetCampaign = (
  request: InvestigationCampaignDetailRequest
): Promise<InvestigationCampaignDetailResponse> =>
  invoke<InvestigationCampaignDetailResponse>("investigation_get_campaign", { request });

export const investigationListTimeline = (
  request: InvestigationTimelineListRequest
): Promise<InvestigationTimelinePageResponse> =>
  invoke<InvestigationTimelinePageResponse>("investigation_list_timeline", { request });

export const investigationRequestStop = (
  request: InvestigationRequestStopRequest
): Promise<InvestigationRequestStopResponse> =>
  invoke<InvestigationRequestStopResponse>("investigation_request_stop", { request });
