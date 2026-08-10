/** Durable Candidate review and execution-attempt IPC. */

import type { AttackCandidateRecoveryResolveRequest } from "@/lib/generated/AttackCandidateRecoveryResolveRequest";
import type { AttackCandidateRecoveryResolveResponse } from "@/lib/generated/AttackCandidateRecoveryResolveResponse";
import type { AttackCandidateResumeRequest } from "@/lib/generated/AttackCandidateResumeRequest";
import type { AttackCandidateReviewRequest } from "@/lib/generated/AttackCandidateReviewRequest";
import type { AttackCandidateReviewResponse } from "@/lib/generated/AttackCandidateReviewResponse";
import type { AttackCandidateReviewScopeRequest } from "@/lib/generated/AttackCandidateReviewScopeRequest";
import type { AttackCandidateReviewState } from "@/lib/generated/AttackCandidateReviewState";
import type { AttackPreparedActionDecision } from "@/lib/generated/AttackPreparedActionDecision";
import type { AttackPreparedActionDecisionRequest } from "@/lib/generated/AttackPreparedActionDecisionRequest";
import type { AttackPreparedActionDecisionResponse } from "@/lib/generated/AttackPreparedActionDecisionResponse";
import type { AttackPreparedActionReviewItem } from "@/lib/generated/AttackPreparedActionReviewItem";
import type { AttackPreparedActionScopeRequest } from "@/lib/generated/AttackPreparedActionScopeRequest";
import type { AttackVerificationPendingEnrichmentView } from "@/lib/generated/AttackVerificationPendingEnrichmentView";
import type { AttackVerificationQueueState } from "@/lib/generated/AttackVerificationQueueState";
import type { CandidateAttemptRow } from "@/lib/generated/CandidateAttemptRow";
import { invoke } from "./client";

export type {
  AttackCandidateResumeRequest,
  AttackCandidateReviewRequest,
  AttackCandidateReviewResponse,
  AttackCandidateReviewScopeRequest,
  AttackCandidateReviewState,
  AttackCandidateRecoveryResolveRequest,
  AttackCandidateRecoveryResolveResponse,
  AttackVerificationPendingEnrichmentView,
  AttackVerificationQueueState,
  CandidateAttemptRow,
  AttackPreparedActionDecision,
  AttackPreparedActionDecisionRequest,
  AttackPreparedActionDecisionResponse,
  AttackPreparedActionReviewItem,
  AttackPreparedActionScopeRequest,
};

export function listCandidateReviews(
  request: AttackCandidateReviewScopeRequest
): Promise<AttackCandidateReviewState> {
  return invoke("attack_list_candidate_reviews", { request });
}

export function reviewCandidates(
  request: AttackCandidateReviewRequest
): Promise<AttackCandidateReviewResponse> {
  return invoke("attack_review_candidates", { request });
}

export function resumeCandidateReview(
  request: AttackCandidateResumeRequest
): Promise<AttackCandidateReviewResponse> {
  return invoke("attack_resume_candidate_review", { request });
}

export function listCandidateAttempts(
  request: AttackCandidateReviewScopeRequest
): Promise<CandidateAttemptRow[]> {
  return invoke("attack_list_candidate_attempts", { request });
}

export function listVerificationQueue(
  request: AttackCandidateReviewScopeRequest
): Promise<AttackVerificationQueueState> {
  return invoke("attack_list_verification_queue", { request });
}

export function resolveCandidateRecovery(
  request: AttackCandidateRecoveryResolveRequest
): Promise<AttackCandidateRecoveryResolveResponse> {
  return invoke("attack_resolve_candidate_recovery", { request });
}

export function listPendingPreparedActions(
  request: AttackPreparedActionScopeRequest
): Promise<AttackPreparedActionReviewItem[]> {
  return invoke("attack_list_pending_prepared_actions", { request });
}

export function decidePreparedAction(
  request: AttackPreparedActionDecisionRequest
): Promise<AttackPreparedActionDecisionResponse> {
  return invoke("attack_decide_prepared_action", { request });
}
