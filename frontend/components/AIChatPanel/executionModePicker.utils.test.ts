import { afterEach, describe, expect, it } from "vitest";
import type { ExecutionModeDescriptor } from "@/lib/ai";
import {
  DEFAULT_PROFILE_ID,
  LAST_MODE_STORAGE_KEY,
  LAST_PROFILE_STORAGE_KEY,
  normalizeExecutionModeId,
  pickTaskProfile,
  readLastExecutionMode,
  readLastProfile,
  resolveEngine,
  splitModes,
  writeLastExecutionMode,
  writeLastProfile,
} from "./executionModePicker.utils";

function mode(id: string, extra: Partial<ExecutionModeDescriptor> = {}): ExecutionModeDescriptor {
  return {
    id,
    displayName: id,
    icon: "Zap",
    badgeColor: "muted",
    description: "",
    allowsSubAgents: id !== "chat",
    ...extra,
  };
}

const PROFILES = [mode("assessment"), mode("pentest"), mode("red_team")];

afterEach(() => {
  try {
    globalThis.localStorage?.clear();
  } catch {
    // ignore
  }
});

describe("resolveEngine", () => {
  it("treats chat as the Chat engine and every other id as Task", () => {
    expect(resolveEngine("chat")).toBe("chat");
    expect(resolveEngine("assessment")).toBe("task");
    expect(resolveEngine("red_team")).toBe("task");
  });
});

describe("splitModes", () => {
  it("separates the chat entry from the task profiles, preserving order", () => {
    const { chat, profiles } = splitModes([mode("chat", { allowsSubAgents: false }), ...PROFILES]);
    expect(chat?.id).toBe("chat");
    expect(profiles.map((p) => p.id)).toEqual(["assessment", "pentest", "red_team"]);
  });

  it("drops the legacy bare task engine id from the profile list", () => {
    const { profiles } = splitModes([mode("chat"), mode("task"), mode("pentest")]);
    expect(profiles.map((p) => p.id)).toEqual(["pentest"]);
  });

  it("returns a null chat entry when one is absent", () => {
    expect(splitModes(PROFILES).chat).toBeNull();
  });
});

describe("pickTaskProfile", () => {
  it("prefers the remembered profile when still available", () => {
    expect(pickTaskProfile("pentest", PROFILES)).toBe("pentest");
  });

  it("falls back to the default profile when the remembered one is gone", () => {
    expect(pickTaskProfile("does-not-exist", PROFILES)).toBe(DEFAULT_PROFILE_ID);
  });

  it("does not treat the legacy bare task id as a profile", () => {
    expect(pickTaskProfile("task", PROFILES)).toBe(DEFAULT_PROFILE_ID);
  });

  it("falls back to the first profile when neither remembered nor default exist", () => {
    const noDefault = [mode("pentest"), mode("red_team")];
    expect(pickTaskProfile(null, noDefault)).toBe("pentest");
  });

  it("returns null when there are no profiles", () => {
    expect(pickTaskProfile("pentest", [])).toBeNull();
  });
});

describe("last-profile memory", () => {
  it("round-trips the last Task profile through localStorage", () => {
    expect(readLastProfile()).toBeNull();
    writeLastProfile("red_team");
    expect(readLastProfile()).toBe("red_team");
    expect(globalThis.localStorage?.getItem(LAST_PROFILE_STORAGE_KEY)).toBe("red_team");
  });

  it("ignores legacy bare task if it was previously written as the profile", () => {
    globalThis.localStorage?.setItem(LAST_PROFILE_STORAGE_KEY, "task");
    expect(readLastProfile()).toBeNull();
  });
});

describe("last-execution-mode memory", () => {
  it("defaults to chat when nothing has been remembered", () => {
    expect(readLastExecutionMode()).toBe("chat");
  });

  it("round-trips the last engine choice so new tabs reopen in it", () => {
    writeLastExecutionMode("pentest");
    expect(readLastExecutionMode()).toBe("pentest");
    expect(globalThis.localStorage?.getItem(LAST_MODE_STORAGE_KEY)).toBe("pentest");
  });

  it("normalizes legacy bare task to the remembered profile", () => {
    writeLastProfile("red_team");
    globalThis.localStorage?.setItem(LAST_MODE_STORAGE_KEY, "task");
    expect(readLastExecutionMode()).toBe("red_team");
    expect(globalThis.localStorage?.getItem(LAST_MODE_STORAGE_KEY)).toBe("red_team");
    expect(normalizeExecutionModeId("task")).toBe("red_team");
  });

  it("stores a concrete default profile instead of bare task", () => {
    writeLastExecutionMode("task");
    expect(readLastExecutionMode()).toBe(DEFAULT_PROFILE_ID);
    expect(globalThis.localStorage?.getItem(LAST_MODE_STORAGE_KEY)).toBe(DEFAULT_PROFILE_ID);
  });

  it("remembers an explicit switch back to chat", () => {
    writeLastExecutionMode("red_team");
    writeLastExecutionMode("chat");
    expect(readLastExecutionMode()).toBe("chat");
  });
});
