/**
 * Pane slice for the Zustand store.
 *
 * Owns the per-tab pane tree (`tabLayouts`) and the in-progress pane-move
 * picker state (`paneMoveState`). Actions defined here only mutate pane-shape
 * data; cross-cutting cleanup (deleting session/streaming/AI keys when a pane
 * is closed) is performed via untyped writes so this slice never imports
 * other slices.
 */

import { logger } from "@/lib/logger";
import {
  countLeafPanes,
  findPaneById,
  getFirstLeafPane,
  getPaneNeighbor,
  insertPaneAtPosition,
  type PaneId,
  removePaneNode,
  type SplitDirection,
  type TabLayout,
  splitPaneNode,
  updatePaneRatio,
} from "@/lib/pane-utils";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import type { SliceCreator } from "./types";

export interface PaneState {
  /** Per-tab pane tree. Keyed by the tab's root session id. */
  tabLayouts: Record<string, TabLayout>;
  /** Active "drag a pane to another pane" picker state, or null when idle. */
  paneMoveState: {
    tabId: string;
    sourcePaneId: PaneId;
    sourceSessionId: string;
  } | null;
}

export interface PaneActions {
  splitPane: (
    tabId: string,
    paneId: PaneId,
    direction: SplitDirection,
    newPaneId: PaneId,
    newSessionId: string,
  ) => void;
  closePane: (tabId: string, paneId: PaneId) => void;
  focusPane: (tabId: string, paneId: PaneId) => void;
  resizePane: (tabId: string, splitPaneId: PaneId, ratio: number) => void;
  navigatePane: (tabId: string, direction: "up" | "down" | "left" | "right") => void;
  /** Start move mode: user picks a drop zone on another pane */
  startPaneMove: (tabId: string, paneId: PaneId, sessionId: string) => void;
  /** Cancel move mode */
  cancelPaneMove: () => void;
  /** Complete a pane move: relocate source pane relative to target pane */
  completePaneMove: (
    targetPaneId: PaneId,
    direction: "top" | "right" | "bottom" | "left",
  ) => void;
  /** Extract a pane into its own new tab (preserving its session) */
  movePaneToNewTab: (tabId: string, paneId: PaneId) => void;
}

export interface PaneSlice extends PaneState, PaneActions {}

export const initialPaneState: PaneState = {
  tabLayouts: {},
  paneMoveState: null,
};

/**
 * Drop every per-session field associated with the given session id. Called
 * by `closePane` during pane teardown. Uses `(state as any)` writes so this
 * slice doesn't depend on the shape of other slices.
 */
function purgeSessionStateInDraft(state: unknown, sessionId: string): void {
  const s = state as Record<string, Record<string, unknown> | undefined>;
  for (const key of [
    "sessions",
    "timelines",
    "pendingCommand",
    "lastSentCommand",
    "agentStreamingBuffer",
    "agentStreaming",
    "streamingBlocks",
    "streamingTextOffset",
    "streamingBlockRevision",
    "agentInitialized",
    "isAgentThinking",
    "isAgentResponding",
    "pendingToolApproval",
    "pendingAskHuman",
    "processedToolRequests",
    "activeToolCalls",
    "thinkingContent",
    "isThinkingExpanded",
    "gitStatus",
    "gitStatusLoading",
    "gitCommitMessage",
    "contextMetrics",
    "compactionCount",
    "isCompacting",
    "isSessionDead",
    "compactionError",
    "tabHasNewActivity",
  ]) {
    const bucket = s[key];
    if (bucket && typeof bucket === "object") {
      delete bucket[sessionId];
    }
  }
}

export const createPaneSlice: SliceCreator<PaneSlice> = (set, get) => ({
  ...initialPaneState,

  splitPane: (tabId, paneId, direction, newPaneId, newSessionId) =>
    set((state: any) => {
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      const rootSession = state.sessions?.[tabId];
      const tabType = rootSession?.tabType ?? "terminal";
      if (tabType !== "terminal") {
        logger.warn(`[store] splitPane: Cannot split ${tabType} tabs`);
        return;
      }

      const currentCount = countLeafPanes(layout.root);
      if (currentCount >= 4) {
        logger.warn("[store] splitPane: Maximum pane limit (4) reached");
        return;
      }

      state.tabLayouts[tabId].root = splitPaneNode(
        layout.root,
        paneId,
        direction,
        newPaneId,
        newSessionId,
      );

      // activeSessionId stays as the tab's root; only focusedPaneId moves.
      state.tabLayouts[tabId].focusedPaneId = newPaneId;
    }),

  closePane: (tabId, paneId) => {
    const currentState = get() as any;
    const layout = currentState.tabLayouts[tabId];
    if (!layout) return;

    const paneNode = findPaneById(layout.root, paneId);
    if (!paneNode || paneNode.type !== "leaf") return;

    const sessionIdToRemove = paneNode.sessionId;

    TerminalInstanceManager.dispose(sessionIdToRemove);

    import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
      resetSessionSequence(sessionIdToRemove);
    });

    set((state: any) => {
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      const newRoot = removePaneNode(layout.root, paneId);

      if (newRoot === null) {
        delete state.tabLayouts[tabId];
        return;
      }

      state.tabLayouts[tabId].root = newRoot;

      if (layout.focusedPaneId === paneId) {
        const newFocusId = getFirstLeafPane(newRoot);
        state.tabLayouts[tabId].focusedPaneId = newFocusId;
      }

      purgeSessionStateInDraft(state, sessionIdToRemove);
    });
  },

  focusPane: (tabId, paneId) =>
    set((state) => {
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      const paneNode = findPaneById(layout.root, paneId);
      if (!paneNode || paneNode.type !== "leaf") return;

      state.tabLayouts[tabId].focusedPaneId = paneId;
    }),

  resizePane: (tabId, splitPaneId, ratio) =>
    set((state) => {
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      state.tabLayouts[tabId].root = updatePaneRatio(layout.root, splitPaneId, ratio);
    }),

  navigatePane: (tabId, direction) =>
    set((state) => {
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      const neighborId = getPaneNeighbor(layout.root, layout.focusedPaneId, direction);
      if (!neighborId) return;

      state.tabLayouts[tabId].focusedPaneId = neighborId;
    }),

  startPaneMove: (tabId, paneId, sessionId) =>
    set((state) => {
      state.paneMoveState = { tabId, sourcePaneId: paneId, sourceSessionId: sessionId };
    }),

  cancelPaneMove: () =>
    set((state) => {
      state.paneMoveState = null;
    }),

  completePaneMove: (targetPaneId, direction) =>
    set((state) => {
      const moveState = state.paneMoveState;
      if (!moveState) return;

      const { tabId, sourcePaneId, sourceSessionId } = moveState;
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      if (sourcePaneId === targetPaneId) {
        state.paneMoveState = null;
        return;
      }

      const treeAfterRemove = removePaneNode(layout.root, sourcePaneId);
      if (!treeAfterRemove) {
        state.paneMoveState = null;
        return;
      }

      const sourceLeaf = {
        type: "leaf" as const,
        id: sourcePaneId,
        sessionId: sourceSessionId,
      };
      const newRoot = insertPaneAtPosition(
        treeAfterRemove,
        targetPaneId,
        direction,
        sourceLeaf,
      );

      state.tabLayouts[tabId].root = newRoot;
      state.tabLayouts[tabId].focusedPaneId = sourcePaneId;
      state.paneMoveState = null;
    }),

  movePaneToNewTab: (tabId, paneId) =>
    set((state: any) => {
      const layout = state.tabLayouts[tabId];
      if (!layout) return;

      if (countLeafPanes(layout.root) <= 1) return;

      const paneNode = findPaneById(layout.root, paneId);
      if (!paneNode || paneNode.type !== "leaf") return;

      const sessionId = paneNode.sessionId;

      const newRoot = removePaneNode(layout.root, paneId);
      if (!newRoot) return;

      state.tabLayouts[tabId].root = newRoot;

      if (layout.focusedPaneId === paneId) {
        state.tabLayouts[tabId].focusedPaneId = getFirstLeafPane(newRoot);
      }

      state.tabLayouts[sessionId] = {
        root: { type: "leaf", id: sessionId, sessionId },
        focusedPaneId: sessionId,
      };
      if (state.tabHasNewActivity) {
        state.tabHasNewActivity[sessionId] = false;
      }
      if (Array.isArray(state.tabOrder)) {
        state.tabOrder.push(sessionId);
      }
      state.activeSessionId = sessionId;
      if (Array.isArray(state.tabActivationHistory)) {
        const histIdx = state.tabActivationHistory.indexOf(sessionId);
        if (histIdx !== -1) {
          state.tabActivationHistory.splice(histIdx, 1);
        }
        state.tabActivationHistory.push(sessionId);
      }
    }),
});

export const selectTabLayout = <T extends PaneState>(
  state: T,
  tabId: string,
): TabLayout | undefined => state.tabLayouts[tabId];

export const selectPaneMoveState = <T extends PaneState>(state: T) => state.paneMoveState;
