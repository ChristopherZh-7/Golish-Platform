import { describe, expect, it } from "vitest";
import type { ChatMessageRow } from "@/lib/api/conversation-db";
import type { UnifiedBlock } from "@/store";
import type { ChatMessage, ThinkingSegment } from "@/store/slices/conversation";
import type { SessionStageRun } from "@/store/store-types";
import {
  chatMessageToDbRow,
  dbMsgToChatMessage,
  isPersistableMessage,
  type LoadedTerminalData,
  loadedToPersistedTerminalData,
  normalizePersistedStageRunState,
  serializeStageRunState,
  timelineBlocksFingerprint,
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

describe("timelineBlocksFingerprint", () => {
  it("changes when an existing sub-agent block receives tool output or a resume boundary", () => {
    const blocks: UnifiedBlock[] = [
      {
        id: "sub-agent-stage-run::org::org-1",
        type: "sub_agent_activity",
        timestamp: "2026-06-28T10:00:00.000Z",
        data: {
          agentId: "recon",
          agentName: "Recon",
          parentRequestId: "stage-run::org::org-1",
          task: "Collect org intel",
          depth: 1,
          status: "running",
          toolCalls: [
            {
              id: "tool-1",
              name: "recon_map_assets",
              args: { org: "Acme" },
              status: "running",
              startedAt: "2026-06-28T10:00:01.000Z",
            },
          ],
          entries: [{ kind: "tool_call", toolCallId: "tool-1" }],
          startedAt: "2026-06-28T10:00:00.000Z",
        },
      },
      {
        id: "tool-exec-stage-run",
        type: "ai_tool_execution",
        timestamp: "2026-06-28T10:00:00.000Z",
        data: {
          requestId: "stage-run",
          toolName: "stage_run",
          args: {},
          status: "running",
          startedAt: "2026-06-28T10:00:00.000Z",
        },
      },
    ];

    const before = timelineBlocksFingerprint(blocks);
    const subAgentBlock = blocks[0];
    if (subAgentBlock.type !== "sub_agent_activity") {
      throw new Error("expected sub-agent block");
    }
    subAgentBlock.data.toolCalls[0].result = { stdout: "mapped 12 assets" };
    subAgentBlock.data.toolCalls[0].status = "completed";

    const afterToolOutput = timelineBlocksFingerprint(blocks);
    expect(afterToolOutput).not.toBe(before);

    subAgentBlock.data.attemptEntryStart = 1;
    expect(timelineBlocksFingerprint(blocks)).not.toBe(afterToolOutput);
  });

  it("changes when a non-last tool execution appends streaming output", () => {
    const blocks: UnifiedBlock[] = [
      {
        id: "tool-exec-httpx",
        type: "ai_tool_execution",
        timestamp: "2026-06-28T10:00:00.000Z",
        data: {
          requestId: "httpx-1",
          toolName: "pentest_run",
          args: { tool_name: "httpx" },
          status: "running",
          startedAt: "2026-06-28T10:00:00.000Z",
          streamingOutput: "first line",
        },
      },
      {
        id: "tool-exec-stage-run",
        type: "ai_tool_execution",
        timestamp: "2026-06-28T10:00:02.000Z",
        data: {
          requestId: "stage-run",
          toolName: "stage_run",
          args: {},
          status: "running",
          startedAt: "2026-06-28T10:00:02.000Z",
        },
      },
    ];

    const before = timelineBlocksFingerprint(blocks);
    const toolBlock = blocks[0];
    if (toolBlock.type !== "ai_tool_execution") {
      throw new Error("expected tool block");
    }
    toolBlock.data.streamingOutput += "\nsecond line";

    expect(timelineBlocksFingerprint(blocks)).not.toBe(before);
  });
});

describe("stage_run persistence shape", () => {
  const current: SessionStageRun = {
    requestId: "T2",
    rows: [
      {
        id: "org-2",
        name: "Beta",
        ownershipPercent: null,
        status: "running",
        evidenceCount: 0,
        coverage: {},
      },
    ],
    summary: { total: 1, covered: 0, active: 1, queued: 0, blocked: 0 },
    stageLabel: "External Attack Surface",
    roleLabel: "Prober",
    coverageAxis: ["LIVE"],
  };
  const previous: SessionStageRun = {
    requestId: "T1",
    rows: [
      {
        id: "org-1",
        name: "Acme",
        ownershipPercent: null,
        status: "passed",
        evidenceCount: 1,
        coverage: {},
      },
    ],
    summary: { total: 1, covered: 1, active: 0, queued: 0, blocked: 0 },
    stageLabel: "Target Intel",
    roleLabel: "Recon",
    coverageAxis: ["DNS"],
  };

  it("serializes current and request-scoped stage_run snapshots", () => {
    const persisted = serializeStageRunState({
      stageRun: current,
      stageRuns: { T1: previous },
    });

    expect(persisted?.current).toBe(current);
    expect(persisted?.byRequestId.T1).toBe(previous);
    expect(persisted?.byRequestId.T2).toBe(current);
  });

  it("normalizes legacy single-stage_run JSON into the request map", () => {
    const normalized = normalizePersistedStageRunState(previous);

    expect(normalized?.current).toBe(previous);
    expect(normalized?.byRequestId.T1).toBe(previous);
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
