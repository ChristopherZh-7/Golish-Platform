import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "../index";
import {
  clearAllSessionCaches,
  clearSessionCache,
  type SessionState,
  selectSessionState,
} from "./session";

/**
 * Unit tests for the combined session selector.
 *
 * SessionState currently includes: timeline, pendingCommand, workingDirectory.
 */

const resetStore = () => {
  clearAllSessionCaches();
  useStore.setState({
    sessions: {},
    activeSessionId: null,
    timelines: {},
    pendingCommand: {},
    agentStreaming: {},
    streamingBlocks: {},
    streamingTextOffset: {},
    agentInitialized: {},
    isAgentThinking: {},
    isAgentResponding: {},
    pendingToolApproval: {},
    processedToolRequests: {},
    activeToolCalls: {},
    thinkingContent: {},
    isThinkingExpanded: {},
    activeWorkflows: {},
    workflowHistory: {},
    activeSubAgents: {},
    contextMetrics: {},
    compactionCount: {},
    isCompacting: {},
    isSessionDead: {},
    compactionError: {},
    gitStatus: {},
    gitStatusLoading: {},
    gitCommitMessage: {},
    tabLayouts: {},
    tabHasNewActivity: {},
    sessionTokenUsage: {},
  });
};

const createSession = (sessionId: string) => {
  useStore.getState().addSession({
    id: sessionId,
    name: `Session ${sessionId}`,
    workingDirectory: `/home/${sessionId}`,
    createdAt: new Date().toISOString(),
    mode: "terminal",
  });
};

describe("selectSessionState", () => {
  beforeEach(() => {
    resetStore();
  });

  describe("Data Extraction", () => {
    it("should extract timeline from store", () => {
      createSession("session-1");
      useStore.getState().handleCommandStart("session-1", "ls");
      useStore.getState().handleCommandEnd("session-1", 0);

      const state = useStore.getState();
      const result = selectSessionState(state, "session-1");

      expect(result.timeline).toHaveLength(1);
      expect(result.timeline[0].type).toBe("command");
    });

    it("should extract pendingCommand from store", () => {
      createSession("session-1");
      useStore.getState().handleCommandStart("session-1", "long-running-cmd");

      const state = useStore.getState();
      const result = selectSessionState(state, "session-1");

      expect(result.pendingCommand).not.toBeNull();
      expect(result.pendingCommand?.command).toBe("long-running-cmd");
    });

    it("should extract workingDirectory from session", () => {
      createSession("session-1");

      const state = useStore.getState();
      const result = selectSessionState(state, "session-1");

      expect(result.workingDirectory).toBe("/home/session-1");
    });
  });

  describe("Default Values", () => {
    it("should return empty array for missing timeline", () => {
      const state = useStore.getState();
      const result = selectSessionState(state, "non-existent");

      expect(result.timeline).toEqual([]);
    });

    it("should return null for missing pendingCommand", () => {
      createSession("session-1");
      const state = useStore.getState();
      const result = selectSessionState(state, "session-1");

      expect(result.pendingCommand).toBeNull();
    });

    it("should return empty string for missing workingDirectory", () => {
      const state = useStore.getState();
      const result = selectSessionState(state, "non-existent");

      expect(result.workingDirectory).toBe("");
    });
  });

  describe("Memoization", () => {
    it("should return same reference when state unchanged", () => {
      createSession("session-1");

      const state = useStore.getState();
      const result1 = selectSessionState(state, "session-1");
      const result2 = selectSessionState(state, "session-1");

      expect(result1).toBe(result2);
    });

    it("should return new reference when timeline changes", () => {
      createSession("session-1");

      const state1 = useStore.getState();
      const result1 = selectSessionState(state1, "session-1");

      useStore.getState().handleCommandStart("session-1", "ls");
      useStore.getState().handleCommandEnd("session-1", 0);

      const state2 = useStore.getState();
      const result2 = selectSessionState(state2, "session-1");

      expect(result1).not.toBe(result2);
      expect(result2.timeline).toHaveLength(1);
    });
  });

  describe("Cross-Session Isolation", () => {
    it("changes to session-1 should not affect session-2 result", () => {
      createSession("session-1");
      createSession("session-2");

      const state1 = useStore.getState();
      const session2Result1 = selectSessionState(state1, "session-2");

      useStore.getState().updateAgentStreaming("session-1", "Hello from session 1");
      useStore.getState().setAgentThinking("session-1", true);

      const state2 = useStore.getState();
      const session2Result2 = selectSessionState(state2, "session-2");

      expect(session2Result1).toBe(session2Result2);
    });

    it("should handle many sessions efficiently", () => {
      for (let i = 0; i < 10; i++) {
        createSession(`session-${i}`);
      }

      const state = useStore.getState();

      const results: SessionState[] = [];
      for (let i = 0; i < 10; i++) {
        results.push(selectSessionState(state, `session-${i}`));
      }

      for (let i = 0; i < 10; i++) {
        const cached = selectSessionState(state, `session-${i}`);
        expect(cached).toBe(results[i]);
      }
    });
  });

  describe("Cache Management", () => {
    it("clearSessionCache should invalidate cache for specific session", () => {
      createSession("session-1");

      const state = useStore.getState();
      const result1 = selectSessionState(state, "session-1");

      clearSessionCache("session-1");

      const result2 = selectSessionState(state, "session-1");

      expect(result1).not.toBe(result2);
      expect(result1.timeline).toEqual(result2.timeline);
    });

    it("clearAllSessionCaches should invalidate all caches", () => {
      createSession("session-1");
      createSession("session-2");

      const state = useStore.getState();
      const result1a = selectSessionState(state, "session-1");
      const result2a = selectSessionState(state, "session-2");

      clearAllSessionCaches();

      const result1b = selectSessionState(state, "session-1");
      const result2b = selectSessionState(state, "session-2");

      expect(result1a).not.toBe(result1b);
      expect(result2a).not.toBe(result2b);
    });
  });
});
