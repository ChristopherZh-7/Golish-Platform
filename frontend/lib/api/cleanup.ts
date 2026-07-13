/** DB-authoritative Cleanup closeout and trusted residual-waiver IPC. */

import type { CleanupCloseoutGateView } from "@/lib/generated/CleanupCloseoutGateView";
import type { CleanupObligationListRequest } from "@/lib/generated/CleanupObligationListRequest";
import type { CleanupObligationView } from "@/lib/generated/CleanupObligationView";
import type { CleanupWaiverSubmitRequest } from "@/lib/generated/CleanupWaiverSubmitRequest";
import { invoke } from "./client";

export type {
  CleanupCloseoutGateView,
  CleanupObligationListRequest,
  CleanupObligationView,
  CleanupWaiverSubmitRequest,
};

export function listCleanupObligations(
  request: CleanupObligationListRequest
): Promise<CleanupObligationView[]> {
  return invoke("cleanup_list_obligations", { request });
}

export function getCleanupCloseoutGate(
  request: CleanupObligationListRequest
): Promise<CleanupCloseoutGateView> {
  return invoke("cleanup_get_closeout_gate", { request });
}

export function waiveCleanupObligation(
  request: CleanupWaiverSubmitRequest
): Promise<CleanupObligationView> {
  return invoke("cleanup_waive_obligation", { request });
}
