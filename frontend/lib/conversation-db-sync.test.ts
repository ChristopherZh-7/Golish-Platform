import { describe, expect, it } from "vitest";
import type { ChatMessageRow } from "@/lib/api/conversation-db";
import type { ChatMessage, ThinkingSegment } from "@/store/slices/conversation";
import {
  chatMessageToDbRow,
  dbMsgToChatMessage,
  isPersistableMessage,
  type LoadedTerminalData,
  loadedToPersistedTerminalData,
} from "./conversation-db-sync";

function chatMessage(overrides: Partial<ChatMessage>): ChatMessage {
  return { id: "m", role: "assistant", content: "", timestamp: 0, ...overrides };
}

function chatRow(overrides: Partial<ChatMessageRow>): ChatMessageRow {
  return {
    id: "m1",
    conversationId: "c1",
    role: "assistant",
    content: "",
    thinking: null,
    error: null,
    toolCalls: null,
    toolCallsContentOffset: null,
    toolCallOffsets: null,
    thinkingSegments: null,
    sortOrder: 0,
    createdAt: 0,
    ...overrides,
  };
}

/**
 * Guards the thinking-segment persistence contract: interleaved reasoning bursts
 * must round-trip through the DB row so restored history keeps multiple Thought
 * blocks instead of collapsing into the single merged `thinking` string.
 */
const SEGMENTS: ThinkingSegment[] = [
  { content: "first", startedAt: 1, endedAt: 2, contentOffset: 0, toolIndex: 0 },
  { content: "second", startedAt: 3, endedAt: 4, contentOffset: 5, toolIndex: 1 },
];

describe("thinking-segment persistence round-trip", () => {
  it("persists thinkingSegments onto the DB row", () => {
    const msg: ChatMessage = {
      id: "m1",
      role: "assistant",
      content: "hello world",
      timestamp: 100,
      thinking: "firstsecond",
      thinkingSegments: SEGMENTS,
    };
    const row = chatMessageToDbRow(msg, "conv-1", 0);
    expect(row.thinkingSegments).toEqual(SEGMENTS);
  });

  it("restores thinkingSegments from the DB row", () => {
    const row: ChatMessageRow = {
      id: "m1",
      conversationId: "conv-1",
      role: "assistant",
      content: "hello world",
      thinking: "firstsecond",
      error: null,
      toolCalls: null,
      toolCallsContentOffset: null,
      toolCallOffsets: null,
      thinkingSegments: SEGMENTS,
      sortOrder: 0,
      createdAt: 100,
    };
    const restored = dbMsgToChatMessage(row);
    expect(restored.thinkingSegments).toEqual(SEGMENTS);
    expect(restored.thinking).toBe("firstsecond");
  });

  it("survives a full save → load round-trip", () => {
    const original: ChatMessage = {
      id: "m1",
      role: "assistant",
      content: "hi",
      timestamp: 7,
      thinking: "ab",
      thinkingSegments: SEGMENTS,
    };
    const restored = dbMsgToChatMessage(chatMessageToDbRow(original, "c", 0));
    expect(restored.thinkingSegments).toEqual(SEGMENTS);
  });

  it("falls back to undefined for legacy rows without segments", () => {
    const row: ChatMessageRow = {
      id: "m1",
      conversationId: "conv-1",
      role: "assistant",
      content: "hello",
      thinking: "merged thought",
      error: null,
      toolCalls: null,
      toolCallsContentOffset: null,
      toolCallOffsets: null,
      thinkingSegments: null,
      sortOrder: 0,
      createdAt: 100,
    };
    const restored = dbMsgToChatMessage(row);
    expect(restored.thinkingSegments).toBeUndefined();
    expect(restored.thinking).toBe("merged thought");
  });

  it("writes null (not an empty array) when there are no segments", () => {
    const msg: ChatMessage = {
      id: "m1",
      role: "assistant",
      content: "hello",
      timestamp: 100,
      thinking: "merged",
      thinkingSegments: [],
    };
    expect(chatMessageToDbRow(msg, "conv-1", 0).thinkingSegments).toBeNull();
  });
});

describe("isPersistableMessage", () => {
  it("persists an error-only bubble (so the turn survives a restart)", () => {
    expect(isPersistableMessage(chatMessage({ content: "", error: "boom" }))).toBe(true);
  });

  it("persists messages with content or tool calls", () => {
    expect(isPersistableMessage(chatMessage({ content: "hi" }))).toBe(true);
    expect(
      isPersistableMessage(
        chatMessage({ content: "", toolCalls: [{ name: "grep", args: "{}" }] })
      )
    ).toBe(true);
  });

  it("drops empty messages and runtime-only system dividers", () => {
    expect(isPersistableMessage(chatMessage({ content: "" }))).toBe(false);
    expect(isPersistableMessage(chatMessage({ role: "system", content: "Stage complete" }))).toBe(
      false
    );
  });
});

describe("error severity re-derivation on restore", () => {
  it("restores a soft planner refusal as a warning", () => {
    const restored = dbMsgToChatMessage(
      chatRow({ error: "Generator failed: declined to produce a plan" })
    );
    expect(restored.errorSeverity).toBe("warning");
  });

  it("restores a real failure as a hard error", () => {
    const restored = dbMsgToChatMessage(chatRow({ error: "Network error: ECONNREFUSED" }));
    expect(restored.errorSeverity).toBe("error");
  });

  it("leaves severity undefined when there is no error", () => {
    const restored = dbMsgToChatMessage(chatRow({ content: "hi" }));
    expect(restored.errorSeverity).toBeUndefined();
  });
});

function loadedTerminal(overrides: Partial<LoadedTerminalData> = {}): LoadedTerminalData {
  return {
    sessionId: "logical-1",
    workingDirectory: "/proj",
    scrollback: "",
    customName: null,
    timelineBlocks: [],
    planJson: null,
    executionMode: null,
    retiredPlansJson: null,
    planMessageId: null,
    stageRunJson: null,
    ...overrides,
  };
}

describe("loadedToPersistedTerminalData", () => {
  // Regression: the boot-time restore path used to drop stageRunJson, so a
  // persisted stage_run came back as a bare "Expired" card (no progress bar /
  // per-org rows) after reopening the app. The mapper is the single source of
  // truth both restore paths go through, so this guards against re-introducing
  // the field-drop.
  it("carries the persisted stage_run snapshot into the restore shape", () => {
    const stageRun = {
      requestId: "req-7",
      rows: [{ id: "org-1", name: "Acme", status: "passed" }],
      summary: { total: 1, covered: 1, active: 0, queued: 0, blocked: 0 },
      stageLabel: "target_intel",
      roleLabel: "recon",
      coverageAxis: ["dns"],
    };
    const persisted = loadedToPersistedTerminalData(loadedTerminal({ stageRunJson: stageRun }));
    expect(persisted.stageRunJson).toEqual(stageRun);
    expect(persisted.logicalTerminalId).toBe("logical-1");
  });

  it("maps logicalTerminalId from the loaded sessionId and preserves core fields", () => {
    const persisted = loadedToPersistedTerminalData(
      loadedTerminal({
        sessionId: "sess-abc",
        workingDirectory: "/x",
        executionMode: "task",
        planJson: { steps: [] },
      })
    );
    expect(persisted.logicalTerminalId).toBe("sess-abc");
    expect(persisted.workingDirectory).toBe("/x");
    expect(persisted.executionMode).toBe("task");
    expect(persisted.planJson).toEqual({ steps: [] });
  });

  // Regression: planMessageId was dropped by BOTH restore paths, so a retired
  // plan card lost its anchor message on restart. Carried through the mapper.
  it("carries planMessageId so a retired plan re-anchors after restart", () => {
    const persisted = loadedToPersistedTerminalData(loadedTerminal({ planMessageId: "msg-42" }));
    expect(persisted.planMessageId).toBe("msg-42");
  });

  it("normalizes a missing stage_run to undefined (legacy rows restore cleanly)", () => {
    expect(loadedToPersistedTerminalData(loadedTerminal()).stageRunJson).toBeUndefined();
  });
});
