import { invoke } from "@tauri-apps/api/core";
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invalidateSettingsCache } from "@/lib/settings";
import { useStore } from "../store";
import { useCreateTerminalTab } from "./useCreateTerminalTab";

// Track invoke call timing to verify parallel execution
let invokeCallTimes: { command: string; time: number }[] = [];

// Helper to flush all pending background promises
async function flushBackgroundWork() {
  // Wait for multiple microtask cycles to let background promises settle
  for (let i = 0; i < 10; i++) {
    await new Promise((resolve) => setTimeout(resolve, 15));
  }
}

describe("useCreateTerminalTab", () => {
  beforeEach(() => {
    // Reset store state
    useStore.setState({
      sessions: {},
      activeSessionId: null,
      timelines: {},
      pendingCommand: {},
      agentStreaming: {},
      agentInitialized: {},
      pendingToolApproval: {},
      processedToolRequests: {},
    });

    // Clear settings cache to ensure fresh state
    invalidateSettingsCache();

    // Reset invoke call tracking
    invokeCallTimes = [];

    // Enhanced mock that tracks call timing
    vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
      const callTime = Date.now();
      invokeCallTimes.push({ command, time: callTime });

      // Simulate some latency for each call
      await new Promise((resolve) => setTimeout(resolve, 10));

      const argsObj = args as Record<string, unknown> | undefined;
      switch (command) {
        case "pty_create":
          return {
            id: "test-session-id",
            working_directory: argsObj?.working_directory ?? "/test/dir",
          };
        case "get_settings":
          return {
            version: 1,
            terminal: { fullterm_commands: [] },
            ai: {
              default_provider: "anthropic",
              default_model: "claude-3-5-sonnet-20241022",
              openrouter: { api_key: null, show_in_selector: false },
              openai: { api_key: null, show_in_selector: false },
              anthropic: { api_key: "test-key", show_in_selector: true },
              ollama: { show_in_selector: false },
              gemini: { api_key: null, show_in_selector: false },
              groq: { api_key: null, show_in_selector: false },
              xai: { api_key: null, show_in_selector: false },
              zai_sdk: { api_key: null, show_in_selector: false },
              nvidia: { api_key: null, base_url: null, show_in_selector: false },
              vertex_ai: {
                credentials_path: null,
                project_id: null,
                location: null,
                show_in_selector: false,
              },
              vertex_gemini: {
                credentials_path: null,
                project_id: null,
                location: null,
                show_in_selector: false,
              },
            },
          };
        case "get_project_settings":
          return { provider: null, model: null, agent_mode: null };
        case "init_ai_session":
          return undefined;
        case "build_provider_config":
          return { provider: "anthropic", model: "claude-3-5-sonnet-20241022" };
        default:
          return undefined;
      }
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("should create a terminal tab successfully", async () => {
    const { result } = renderHook(() => useCreateTerminalTab());

    let sessionId: string | null = null;
    await act(async () => {
      sessionId = await result.current.createTerminalTab("/test/path");
    });

    expect(sessionId).toBe("test-session-id");
    expect(useStore.getState().sessions["test-session-id"]).toBeDefined();
  });

  describe("startup performance optimization", () => {
    // SKIP block: AI initialisation has been removed from useCreateTerminalTab
    // (it lives in AIChatPanel now — see the source file's comment "AI is
    // managed by the right-side AI chat panel, not per-terminal"). The four
    // assertions below cover behaviour that no longer exists in this hook:
    //   - aiConfig.status transitions ("initializing" → "ready")
    //   - get_settings / get_project_settings parallel calls
    //   - settings caching across tabs
    // They should be re-homed to the AIChatPanel test suite once that work
    // is scheduled. See docs/risks/d1-vitest-react19.md.
    it.skip("should return immediately after pty_create without waiting for settings or AI", async () => {
      const { result } = renderHook(() => useCreateTerminalTab());

      await act(async () => {
        await result.current.createTerminalTab("/test/path");
      });

      const ptyCreateCall = invokeCallTimes.find((c) => c.command === "pty_create");
      expect(ptyCreateCall).toBeDefined();

      const session = useStore.getState().sessions["test-session-id"];
      expect(session).toBeDefined();
      expect(session.aiConfig?.status).toBe("initializing");
    });

    it.skip("should fetch settings and project settings in parallel in background", async () => {
      const { result } = renderHook(() => useCreateTerminalTab());

      await act(async () => {
        await result.current.createTerminalTab("/test/path");
      });

      await act(async () => {
        await flushBackgroundWork();
      });

      const getSettingsCall = invokeCallTimes.find((c) => c.command === "get_settings");
      const getProjectSettingsCall = invokeCallTimes.find(
        (c) => c.command === "get_project_settings"
      );

      expect(getSettingsCall).toBeDefined();
      expect(getProjectSettingsCall).toBeDefined();

      if (getSettingsCall && getProjectSettingsCall) {
        const timeDiff = Math.abs(getSettingsCall.time - getProjectSettingsCall.time);
        expect(timeDiff).toBeLessThan(5);
      }
    });

    it.skip("should use cached settings on subsequent tab creations", async () => {
      const { result } = renderHook(() => useCreateTerminalTab());

      invokeCallTimes = [];

      await act(async () => {
        await result.current.createTerminalTab("/test/path1");
      });
      await act(async () => {
        await flushBackgroundWork();
      });

      const firstSettingsCallCount = invokeCallTimes.filter(
        (c) => c.command === "get_settings"
      ).length;
      expect(firstSettingsCallCount).toBe(1);

      await act(async () => {
        await result.current.createTerminalTab("/test/path2");
      });
      await act(async () => {
        await flushBackgroundWork();
      });

      const totalSettingsCallCount = invokeCallTimes.filter(
        (c) => c.command === "get_settings"
      ).length;
      expect(totalSettingsCallCount).toBe(1);
    });

    it.skip("should eventually update AI status to ready after background init", async () => {
      const { result } = renderHook(() => useCreateTerminalTab());

      await act(async () => {
        await result.current.createTerminalTab("/test/path");
      });

      // Skipped: AI status transitions have moved to AIChatPanel.
      expect(useStore.getState().sessions["test-session-id"]?.aiConfig?.status).toBe(
        "initializing"
      );

      await act(async () => {
        await flushBackgroundWork();
      });

      expect(useStore.getState().sessions["test-session-id"]?.aiConfig?.status).toBe("ready");
    });
  });
});
