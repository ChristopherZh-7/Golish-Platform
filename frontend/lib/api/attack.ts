/** Durable Candidate review and execution-attempt IPC. */

import type { AttackCandidateResumeRequest } from "@/lib/generated/AttackCandidateResumeRequest";
import type { AttackCandidateReviewRequest } from "@/lib/generated/AttackCandidateReviewRequest";
import type { AttackCandidateReviewResponse } from "@/lib/generated/AttackCandidateReviewResponse";
import type { AttackCandidateReviewScopeRequest } from "@/lib/generated/AttackCandidateReviewScopeRequest";
import type { AttackCandidateReviewState } from "@/lib/generated/AttackCandidateReviewState";
import type { CandidateAttemptRow } from "@/lib/generated/CandidateAttemptRow";
import { invoke } from "./client";

export type {
  AttackCandidateResumeRequest,
  AttackCandidateReviewRequest,
  AttackCandidateReviewResponse,
  AttackCandidateReviewScopeRequest,
  AttackCandidateReviewState,
  CandidateAttemptRow,
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
