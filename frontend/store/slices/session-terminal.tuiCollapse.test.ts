import { beforeEach, describe, expect, it } from "vitest";
import { selectCommandBlocksFromTimeline } from "@/lib/timeline/selectors";
import { useStore } from "@/store";

const SESSION_ID = "test-session-tui";

function resetStore() {
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
}

function lastBlock() {
  const state = useStore.getState();
  const blocks = selectCommandBlocksFromTimeline(state.timelines[SESSION_ID]);
  return blocks[blocks.length - 1];
}

describe("session-terminal · Phase C TUI block folding", () => {
  beforeEach(() => {
    resetStore();
  });

  it("creates a non-collapsed block for normal shell commands", () => {
    const api = useStore.getState();
    api.handleCommandStart(SESSION_ID, "ls -la");
    api.appendOutput(SESSION_ID, "foo\nbar\n");
    api.handleCommandEnd(SESSION_ID, 0);

    const block = lastBlock();
    expect(block.command).toBe("ls -la");
    expect(block.isCollapsed).toBe(false);
  });

  it.each([
    ["vim foo.txt", true],
    ["nvim ~/.config/nvim/init.lua", true],
    ["htop", true],
    ["less LICENSE", true],
    ["nano README", true],
    ["tmux new -s work", true],
    ["sudo htop", true],
    ["/usr/bin/vim file", true],
    // Process detector strips leading env vars / paths but `cargo test`
    // should NOT be flagged.
    ["cargo test", false],
    ["npm install", false],
    ["echo vim", false],
  ])("collapse vs not — %s → %s", (command, expectedCollapsed) => {
    const api = useStore.getState();
    api.handleCommandStart(SESSION_ID, command);
    api.handleCommandEnd(SESSION_ID, 0);

    const block = lastBlock();
    expect(block.command).toBe(command);
    expect(block.isCollapsed).toBe(expectedCollapsed);
  });

  it("preserves the duration / exit_code metadata on collapsed TUI blocks", () => {
    const api = useStore.getState();
    api.handleCommandStart(SESSION_ID, "vim foo.txt");
    api.handleCommandEnd(SESSION_ID, 0, Date.now() + 12_345);

    const block = lastBlock();
    expect(block.isCollapsed).toBe(true);
    expect(block.exitCode).toBe(0);
    expect(block.durationMs).toBeGreaterThan(0);
  });
});
