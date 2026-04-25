/**
 * Human-in-the-loop (HITL) slice for the Zustand store.
 *
 * Owns the global approval mode plus the per-session pending tool-approval
 * and ask-human queues. Components that prompt the user before a destructive
 * tool runs (or that ask for credentials/choices mid-stream) read from here.
 */

import type { AskHumanRequest, ToolCall } from "../store-types";
import type { SliceCreator } from "./types";

export interface HitlState {
  /** Global approval policy for tool execution: "ask" | "auto-approve" | etc. */
  approvalMode: string;
  /** One-at-a-time pending tool approval modal data, keyed by sessionId. */
  pendingToolApproval: Record<string, ToolCall | null>;
  /** Pending ask-human prompt (credentials/choice/freetext) keyed by sessionId. */
  pendingAskHuman: Record<string, AskHumanRequest | null>;
}

export interface HitlActions {
  setApprovalMode: (mode: string) => void;
  setPendingToolApproval: (sessionId: string, tool: ToolCall | null) => void;
  setPendingAskHuman: (sessionId: string, request: AskHumanRequest) => void;
  clearPendingAskHuman: (sessionId: string) => void;
}

export interface HitlSlice extends HitlState, HitlActions {}

export const initialHitlState: HitlState = {
  approvalMode: "ask",
  pendingToolApproval: {},
  pendingAskHuman: {},
};

export const createHitlSlice: SliceCreator<HitlSlice> = (set) => ({
  ...initialHitlState,

  setApprovalMode: (mode) =>
    set((state) => {
      state.approvalMode = mode;
    }),

  setPendingToolApproval: (sessionId, tool) =>
    set((state) => {
      state.pendingToolApproval[sessionId] = tool;
    }),

  setPendingAskHuman: (sessionId, request) =>
    set((state) => {
      state.pendingAskHuman[sessionId] = request;
    }),

  clearPendingAskHuman: (sessionId) =>
    set((state) => {
      state.pendingAskHuman[sessionId] = null;
    }),
});

export const selectApprovalMode = <T extends HitlState>(state: T): string => state.approvalMode;

export const selectPendingToolApproval = <T extends HitlState>(
  state: T,
  sessionId: string,
): ToolCall | null => state.pendingToolApproval[sessionId] ?? null;

export const selectPendingAskHuman = <T extends HitlState>(
  state: T,
  sessionId: string,
): AskHumanRequest | null => state.pendingAskHuman[sessionId] ?? null;
