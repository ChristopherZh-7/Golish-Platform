import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "@/store";

const SESSION_ID = "test-session-interactive";

describe("session-core: setInteractiveMode", () => {
  beforeEach(() => {
    useStore.setState({
      sessions: {},
      activeSessionId: null,
      tabOrder: [],
      tabActivationHistory: [],
      tabHasNewActivity: {},
      tabLayouts: {},
      timelines: {},
      streamingBlocks: {},
      streamingTextOffset: {},
      pendingCommand: {},
      lastSentCommand: {},
      streamingBlockRevision: {},
    });

    useStore.getState().addSession({
      id: SESSION_ID,
      name: "Test",
      workingDirectory: "/tmp",
      createdAt: new Date().toISOString(),
      mode: "terminal",
    });
  });

  it("enters interactive mode for a session", () => {
    useStore.getState().setInteractiveMode(SESSION_ID, {
      active: true,
      command: "bash select.sh",
      detector: "yn_choice",
      enteredAt: 1_700_000_000_000,
    });

    const mode = useStore.getState().sessions[SESSION_ID]?.interactiveMode;
    expect(mode).toEqual({
      active: true,
      command: "bash select.sh",
      detector: "yn_choice",
      enteredAt: 1_700_000_000_000,
    });
  });

  it("clears interactive mode when called with null", () => {
    const api = useStore.getState();
    api.setInteractiveMode(SESSION_ID, {
      active: true,
      command: "npm init",
      detector: "generic_prompt",
      enteredAt: 1,
    });
    api.setInteractiveMode(SESSION_ID, null);

    const mode = useStore.getState().sessions[SESSION_ID]?.interactiveMode;
    expect(mode).toBeNull();
  });

  it("is a no-op when the new mode is equivalent to the existing one", () => {
    const api = useStore.getState();
    const next = {
      active: true,
      command: "git push",
      detector: "yn_choice" as const,
      enteredAt: 42,
    };
    api.setInteractiveMode(SESSION_ID, next);
    const refBefore = useStore.getState().sessions[SESSION_ID]?.interactiveMode;

    // Same shape, fresh `enteredAt` — exercises the identity-stable
    // short-circuit path in the action so memoised selectors don't
    // re-fire.
    api.setInteractiveMode(SESSION_ID, { ...next, enteredAt: 99 });
    const refAfter = useStore.getState().sessions[SESSION_ID]?.interactiveMode;

    expect(refAfter).toBe(refBefore);
  });

  it("updates when the detector kind changes", () => {
    const api = useStore.getState();
    api.setInteractiveMode(SESSION_ID, {
      active: true,
      command: "ssh remote",
      detector: "generic_prompt",
      enteredAt: 1,
    });
    api.setInteractiveMode(SESSION_ID, {
      active: true,
      command: "ssh remote",
      detector: "password",
      enteredAt: 2,
    });

    const mode = useStore.getState().sessions[SESSION_ID]?.interactiveMode;
    expect(mode?.detector).toBe("password");
    expect(mode?.enteredAt).toBe(2);
  });

  it("ignores writes for unknown sessions", () => {
    expect(() =>
      useStore.getState().setInteractiveMode("nonexistent-session", {
        active: true,
        command: null,
        detector: "yn_choice",
        enteredAt: 0,
      })
    ).not.toThrow();
  });
});
