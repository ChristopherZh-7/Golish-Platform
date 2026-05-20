import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../../store";
import { clearAllSessionCaches } from "../../store/selectors/session";

// happy-dom (our test environment) does not always populate `window.localStorage`
// with a real Storage instance — sometimes it is just `{}`. Install a tiny
// in-memory polyfill so `useInputResize` can read/write the dragged height.
function installLocalStoragePolyfill() {
  const mem: Record<string, string> = {};
  const polyfill = {
    getItem: (k: string) => (k in mem ? mem[k] : null),
    setItem: (k: string, v: string) => {
      mem[k] = String(v);
    },
    removeItem: (k: string) => {
      delete mem[k];
    },
    clear: () => {
      for (const k of Object.keys(mem)) delete mem[k];
    },
    key: (i: number) => Object.keys(mem)[i] ?? null,
    get length() {
      return Object.keys(mem).length;
    },
  };
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: polyfill,
  });
  return polyfill;
}

/**
 * Regression test for: "主页 timeline 在输入命令时往上跳一格"
 *
 * Root cause: `adjustTextareaHeight` in `useUnifiedInputState.ts` previously
 * set `textarea.style.height = "auto"` on every input change to measure
 * scrollHeight. When the user had dragged the input panel to a fixed
 * minimum (e.g. 80px), this caused a brief shrink to ~26px (single line)
 * before snapping back, which propagated up the parent flex column and
 * forced the browser to clamp `scrollTop` on the timeline above.
 *
 * Fix: when `desired > 0`, set the textarea to `desired` first and only
 * grow if `scrollHeight` actually exceeds it. The legacy auto-grow path
 * (no fixed desired height) is unchanged.
 */

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/lib/ai", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ai")>();
  return {
    ...actual,
    sendPromptSession: vi.fn(() => Promise.resolve()),
    sendPromptWithAttachments: vi.fn(() => Promise.resolve()),
    getVisionCapabilities: vi.fn(() => Promise.resolve({ supports_vision: false })),
  };
});

vi.mock("@/lib/api/pty", () => ({
  ptyWrite: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/lib/api/files", () => ({
  readPrompt: vi.fn(() => Promise.resolve("prompt content")),
  readSkillBody: vi.fn(() => Promise.resolve("skill content")),
  readFileAsBase64: vi.fn(() => Promise.resolve("base64data")),
  listWorkspaceFiles: vi.fn().mockResolvedValue([]),
}));
vi.mock("@/lib/api/shell", () => ({
  imeGetSource: vi.fn(() => Promise.resolve("com.apple.keylayout.ABC")),
  imeSetSource: vi.fn(() => Promise.resolve()),
}));
vi.mock("@/lib/notify", () => ({
  notify: {
    error: vi.fn(),
    warning: vi.fn(),
    success: vi.fn(),
    info: vi.fn(),
  },
}));
vi.mock("@/lib/logger", () => ({
  logger: {
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  },
}));
vi.mock("@/hooks/useSlashCommands", () => ({
  useSlashCommands: vi.fn(() => ({ commands: [] })),
}));
vi.mock("@/hooks/useFileCommands", () => ({
  useFileCommands: vi.fn(() => ({ files: [] })),
}));
vi.mock("@/hooks/usePathCompletion", () => ({
  usePathCompletion: vi.fn(() => ({ completions: [], totalCount: 0 })),
}));
vi.mock("@/hooks/useHistorySearch", () => ({
  useHistorySearch: vi.fn(() => ({ matches: [] })),
}));
vi.mock("@/hooks/useCommandHistory", () => ({
  useCommandHistory: vi.fn(() => ({
    history: [],
    add: vi.fn(),
    navigateUp: vi.fn(),
    navigateDown: vi.fn(),
    reset: vi.fn(),
  })),
}));

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
    inputMode: "agent",
  });
};

const STORAGE_KEY = "golish.unifiedInput.desiredHeight";

/**
 * Install a global spy on CSSStyleDeclaration.prototype.height setter.
 * Returns a tuple of (setterCalls, restore). The spy records every value
 * assigned to `.style.height` on any element, which is precisely what
 * we want to assert: did the buggy "auto" intermediate ever land?
 */
function spyOnStyleHeightSetter(): [string[], () => void] {
  const calls: string[] = [];
  const proto = CSSStyleDeclaration.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(proto, "height");
  if (!descriptor?.set || !descriptor?.get) {
    throw new Error("CSSStyleDeclaration.prototype.height descriptor unavailable");
  }
  const origGet = descriptor.get;
  const origSet = descriptor.set;
  Object.defineProperty(proto, "height", {
    configurable: true,
    get() {
      return origGet.call(this);
    },
    set(value: string) {
      calls.push(value);
      origSet.call(this, value);
    },
  });
  const restore = () => {
    Object.defineProperty(proto, "height", {
      configurable: true,
      get: origGet,
      set: origSet,
    });
  };
  return [calls, restore];
}

const flushFrames = async (n = 2) => {
  for (let i = 0; i < n; i++) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  }
};

describe("UnifiedInput textarea height adjustment", () => {
  beforeEach(() => {
    resetStore();
    vi.clearAllMocks();
    installLocalStoragePolyfill();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  describe("desired > 0 (user dragged the resize handle)", () => {
    it("never sets textarea.style.height to 'auto' while typing", async () => {
      window.localStorage.setItem(STORAGE_KEY, "80");
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");
      const [heightWrites, restore] = spyOnStyleHeightSetter();
      try {
        render(<UnifiedInput sessionId="session-1" />);
        await flushFrames();
        const textarea = screen.getByTestId("unified-input") as HTMLTextAreaElement;

        // Clear writes from initial mount; we only care about writes that
        // happen DURING typing (where the bug manifests).
        heightWrites.length = 0;

        await userEvent.type(textarea, "ls -la");
        await flushFrames(3);

        // The buggy code path would have written "auto" multiple times here
        // (once per keystroke), producing visible layout jumps. After the
        // fix, only "80px" should ever be written (no shrink-then-restore).
        expect(heightWrites).not.toContain("auto");
        for (const v of heightWrites) {
          expect(v).toMatch(/^\d+px$/);
        }
      } finally {
        restore();
      }
    });

    it("settles at exactly `desired` px for short single-line input", async () => {
      window.localStorage.setItem(STORAGE_KEY, "80");
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");
      render(<UnifiedInput sessionId="session-1" />);
      await flushFrames();
      const textarea = screen.getByTestId("unified-input") as HTMLTextAreaElement;

      await userEvent.type(textarea, "ls");
      await flushFrames(3);

      expect(textarea.style.height).toBe("80px");
    });
  });

  describe("desired === 0 (legacy auto-grow path)", () => {
    it("still uses 'auto' so the textarea can shrink when content is deleted", async () => {
      // No localStorage entry → desiredHeight stays at 0 (UNSET).
      createSession("session-1");

      const { UnifiedInput } = await import("./UnifiedInput");
      const [heightWrites, restore] = spyOnStyleHeightSetter();
      try {
        render(<UnifiedInput sessionId="session-1" />);
        await flushFrames();
        const textarea = screen.getByTestId("unified-input") as HTMLTextAreaElement;

        heightWrites.length = 0;
        await userEvent.type(textarea, "x");
        await flushFrames(3);

        // Legacy path intentionally still uses "auto" because there's no
        // fixed minimum to fall back to — auto-shrink is required.
        expect(heightWrites).toContain("auto");
      } finally {
        restore();
      }
    });
  });
});
