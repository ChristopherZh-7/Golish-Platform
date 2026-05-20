/**
 * Behavioural tests for the Warp-style RunningCommandCard. Locks in
 * the contract that:
 *  - mount does NOT steal focus (or `cargo build` would yank focus
 *    away from whatever the user was typing in another pane)
 *  - flipping `interactiveMode.active` true auto-focuses the hidden
 *    capture textarea so `y` + Enter just works at a `[Y/n]` prompt
 *  - keydown is translated into the byte sequence a terminal app
 *    expects (printable chars, Enter as `\r`, Backspace as `\x7f`,
 *    Ctrl-letter as ASCII control codes, arrows as CSI sequences)
 *  - paste ships the clipboard text in one `ptyWrite`
 *
 * The accumulator-and-flush plumbing for output bytes is already
 * covered indirectly by the `UnifiedTimeline` suite (which renders
 * this card whenever `pendingCommand` is non-null), so we don't
 * re-test it here.
 */

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitMockEvent } from "@/test/mocks/event-bus-helpers";

// `vi.mock` is hoisted to the top of the file, so the factory cannot
// close over a top-level `const`. We use `vi.hoisted` to declare the
// mock alongside the hoist so the factory sees it at module-eval time.
const { ptyWriteMock } = vi.hoisted(() => ({
  ptyWriteMock: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/lib/api/pty", () => ({
  ptyWrite: ptyWriteMock,
}));

import { useStore } from "../../store";
import { RunningCommandCard } from "./RunningCommandCard";

const SESSION_ID = "rcc-test-session";

function resetStore() {
  useStore.setState({
    sessions: {},
    activeSessionId: null,
    timelines: {},
    pendingCommand: {},
    agentStreaming: {},
    agentInitialized: {},
  });
  useStore.getState().addSession({
    id: SESSION_ID,
    name: "Test",
    workingDirectory: "/tmp",
    createdAt: new Date().toISOString(),
    mode: "terminal",
  });
}

function getCaptureTextarea(): HTMLTextAreaElement {
  // The card hosts exactly one offscreen <textarea>. Picking by role
  // would also match the bottom UnifiedInput in a fuller render, but
  // we render the card in isolation here so the lookup is unambiguous.
  const card = screen.getByTestId("running-command-card");
  const ta = card.querySelector("textarea");
  if (!ta) throw new Error("RunningCommandCard textarea not found");
  return ta as HTMLTextAreaElement;
}

describe("RunningCommandCard · Warp-style stdin routing", () => {
  beforeEach(() => {
    resetStore();
    ptyWriteMock.mockClear();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("does not steal focus on mount", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="cargo build" />);
    const ta = getCaptureTextarea();
    expect(document.activeElement).not.toBe(ta);
  });

  it("auto-focuses the capture textarea when interactiveMode flips active", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="sqlmap -u baidu.com" />);
    const ta = getCaptureTextarea();
    expect(document.activeElement).not.toBe(ta);

    act(() => {
      useStore.getState().setInteractiveMode(SESSION_ID, {
        active: true,
        command: "sqlmap -u baidu.com",
        detector: "yn_choice",
        enteredAt: Date.now(),
      });
    });

    expect(document.activeElement).toBe(ta);
  });

  it("ships printable characters straight to ptyWrite", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="sqlmap" />);
    const ta = getCaptureTextarea();

    fireEvent.keyDown(ta, { key: "y" });
    expect(ptyWriteMock).toHaveBeenCalledWith(SESSION_ID, "y");

    fireEvent.keyDown(ta, { key: "N" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "N");
  });

  it("translates Enter to CR, Backspace to DEL, Tab to TAB, Escape to ESC", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="bash" />);
    const ta = getCaptureTextarea();

    fireEvent.keyDown(ta, { key: "Enter" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\r");

    fireEvent.keyDown(ta, { key: "Backspace" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x7f");

    fireEvent.keyDown(ta, { key: "Tab" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\t");

    fireEvent.keyDown(ta, { key: "Escape" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x1b");
  });

  it("translates Ctrl-C / Ctrl-D / Ctrl-Z to ASCII control codes", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="cat" />);
    const ta = getCaptureTextarea();

    fireEvent.keyDown(ta, { key: "c", ctrlKey: true });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x03");

    fireEvent.keyDown(ta, { key: "d", ctrlKey: true });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x04");

    fireEvent.keyDown(ta, { key: "z", ctrlKey: true });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x1a");
  });

  it("translates arrow keys to ANSI CSI sequences", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="vim" />);
    const ta = getCaptureTextarea();

    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x1b[A");

    fireEvent.keyDown(ta, { key: "ArrowDown" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x1b[B");

    fireEvent.keyDown(ta, { key: "ArrowRight" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x1b[C");

    fireEvent.keyDown(ta, { key: "ArrowLeft" });
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x1b[D");
  });

  it("ignores OS-level Cmd / Alt chords (lets browser handle Cmd-C copy etc.)", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="bash" />);
    const ta = getCaptureTextarea();

    fireEvent.keyDown(ta, { key: "c", metaKey: true });
    fireEvent.keyDown(ta, { key: "v", metaKey: true });
    fireEvent.keyDown(ta, { key: "Tab", altKey: true });

    expect(ptyWriteMock).not.toHaveBeenCalled();
  });

  it("ships pasted clipboard text in a single ptyWrite", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="bash" />);
    const ta = getCaptureTextarea();

    fireEvent.paste(ta, {
      clipboardData: {
        getData: (type: string) => (type === "text" ? "echo hello\n" : ""),
      },
    });

    expect(ptyWriteMock).toHaveBeenCalledTimes(1);
    expect(ptyWriteMock).toHaveBeenCalledWith(SESSION_ID, "echo hello\n");
  });

  it("focuses the capture textarea when the visible card is clicked", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="bash" />);
    const card = screen.getByTestId("running-command-card");
    const ta = getCaptureTextarea();
    expect(document.activeElement).not.toBe(ta);

    fireEvent.click(card);
    expect(document.activeElement).toBe(ta);
  });

  it("shows the amber 'waiting for input' pill only while interactiveMode is active", () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="sqlmap" />);
    expect(screen.queryByTestId("running-command-card-waiting")).toBeNull();

    act(() => {
      useStore.getState().setInteractiveMode(SESSION_ID, {
        active: true,
        command: "sqlmap",
        detector: "yn_choice",
        enteredAt: Date.now(),
      });
    });

    const pill = screen.getByTestId("running-command-card-waiting");
    expect(pill).toBeInTheDocument();
    expect(pill).toHaveTextContent("等待输入");

    act(() => {
      useStore.getState().setInteractiveMode(SESSION_ID, null);
    });

    expect(screen.queryByTestId("running-command-card-waiting")).toBeNull();
  });

  it("local-echo backspace erases the user's `y` but stops at zero counter (won't eat tool output)", async () => {
    // The card tracks the number of characters the user has typed
    // during the current line via `interactiveInputCountRef`. Local-
    // echo backspace only fires while that counter is > 0, so once
    // it hits zero further Backspace presses no-op — preventing the
    // user from accidentally erasing the program's own prompt or
    // earlier output even on long-press / mash-the-key scenarios.
    // This is the regression the user reported: without the
    // counter, repeated backspaces ate the `Do you want to follow?
    // [Y/n] ` line itself.
    render(<RunningCommandCard sessionId={SESSION_ID} command="sqlmap" />);
    const ta = getCaptureTextarea();

    act(() => {
      emitMockEvent("terminal_output", {
        session_id: SESSION_ID,
        data: "[*] starting\n[INFO] testing\nDo you want to follow? [Y/n] ",
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("running-command-card")).toHaveTextContent("[Y/n]");
    });

    act(() => {
      useStore.getState().setInteractiveMode(SESSION_ID, {
        active: true,
        command: "sqlmap",
        detector: "yn_choice",
        enteredAt: Date.now(),
      });
    });

    // User presses `y` — handler increments the counter to 1 and
    // ships the byte; the PTY then echoes the `y` back into the
    // visible buffer.
    fireEvent.keyDown(ta, { key: "y" });
    act(() => {
      emitMockEvent("terminal_output", { session_id: SESSION_ID, data: "y" });
    });
    await waitFor(() => {
      expect(screen.getByTestId("running-command-card")).toHaveTextContent(
        "Do you want to follow? [Y/n] y"
      );
    });

    // First backspace deletes the `y` (counter 1 → 0).
    fireEvent.keyDown(ta, { key: "Backspace" });
    let card = screen.getByTestId("running-command-card");
    expect(card.textContent).toContain("Do you want to follow? [Y/n]");
    expect(card.textContent).not.toMatch(/\[Y\/n\] y/);
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x7f");

    // Five more backspaces — counter already 0, so the local pop
    // path is gated off; the prompt and prior banner must remain.
    for (let i = 0; i < 5; i++) {
      fireEvent.keyDown(ta, { key: "Backspace" });
    }
    card = screen.getByTestId("running-command-card");
    expect(card.textContent).toContain("[*] starting");
    expect(card.textContent).toContain("[INFO] testing");
    expect(card.textContent).toContain("Do you want to follow? [Y/n]");
  });

  it("resets local-echo backspace counter on every new prompt fire (different enteredAt)", async () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="sqlmap" />);
    const ta = getCaptureTextarea();

    act(() => {
      emitMockEvent("terminal_output", {
        session_id: SESSION_ID,
        data: "prompt 1 [Y/n] ",
      });
      useStore.getState().setInteractiveMode(SESSION_ID, {
        active: true,
        command: "sqlmap",
        detector: "yn_choice",
        enteredAt: 1,
      });
    });

    // First prompt: user types `y`, PTY echoes `y`, user backspaces it.
    fireEvent.keyDown(ta, { key: "y" });
    act(() => {
      emitMockEvent("terminal_output", { session_id: SESSION_ID, data: "y" });
    });
    fireEvent.keyDown(ta, { key: "Backspace" });

    // Tool output → next prompt → detector re-fires with a new
    // `enteredAt`. The counter resets so the user can start typing
    // fresh into prompt #2.
    act(() => {
      emitMockEvent("terminal_output", {
        session_id: SESSION_ID,
        data: "\nresult: ok\nprompt 2 [Y/n] ",
      });
      useStore.getState().setInteractiveMode(SESSION_ID, {
        active: true,
        command: "sqlmap",
        detector: "yn_choice",
        enteredAt: 2,
      });
    });

    // Type `n` at prompt 2 (counter 0 → 1), backspace it once
    // (counter 1 → 0). Two extra backspaces must no-op because the
    // counter is gated at 0 — they MUST NOT delete back into
    // `result: ok` or earlier output.
    fireEvent.keyDown(ta, { key: "n" });
    act(() => {
      emitMockEvent("terminal_output", { session_id: SESSION_ID, data: "n" });
    });
    fireEvent.keyDown(ta, { key: "Backspace" });
    fireEvent.keyDown(ta, { key: "Backspace" });
    fireEvent.keyDown(ta, { key: "Backspace" });

    const card = screen.getByTestId("running-command-card");
    expect(card.textContent).toContain("prompt 1 [Y/n]");
    expect(card.textContent).toContain("result: ok");
    expect(card.textContent).toContain("prompt 2 [Y/n]");
    expect(card.textContent).not.toMatch(/prompt 2 \[Y\/n\] n/);
  });

  it("does NOT local-echo backspace outside interactive mode (anchor null)", async () => {
    render(<RunningCommandCard sessionId={SESSION_ID} command="bash" />);
    const ta = getCaptureTextarea();

    act(() => {
      emitMockEvent("terminal_output", {
        session_id: SESSION_ID,
        data: "some output",
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId("running-command-card")).toHaveTextContent("some output");
    });

    fireEvent.keyDown(ta, { key: "Backspace" });

    // Buffer is untouched (interactive mode never armed), but DEL
    // still ships to the PTY for non-cooked apps that interpret it.
    expect(screen.getByTestId("running-command-card")).toHaveTextContent("some output");
    expect(ptyWriteMock).toHaveBeenLastCalledWith(SESSION_ID, "\x7f");
  });

  it("renders PTY backspace echo (`\\b \\b`) by erasing the prior visible character", async () => {
    // Regression: when sqlmap / read / python-input prompts are in
    // cooked mode and the user hits backspace, the PTY echoes back
    // `\b \b`. Without explicit erase handling those bytes show up
    // as invisible control characters in the rendered <pre> and the
    // mistyped `y` stays on screen even though the program itself
    // already cleared its input buffer.
    render(<RunningCommandCard sessionId={SESSION_ID} command="sqlmap" />);

    act(() => {
      emitMockEvent("terminal_output", {
        session_id: SESSION_ID,
        data: "Do you want to follow? [Y/n] y",
      });
    });

    await waitFor(() => {
      expect(screen.getByTestId("running-command-card")).toHaveTextContent(
        "Do you want to follow? [Y/n] y"
      );
    });

    act(() => {
      emitMockEvent("terminal_output", {
        session_id: SESSION_ID,
        data: "\b \b",
      });
    });

    await waitFor(() => {
      const card = screen.getByTestId("running-command-card");
      // The erased `y` is gone — only the prompt remains.
      expect(card.textContent).toContain("Do you want to follow? [Y/n]");
      expect(card.textContent).not.toMatch(/\[Y\/n\] y/);
    });
  });

  it("keeps the offscreen capture textarea inside the card (avoids scroll-on-focus jump)", () => {
    // Regression: an earlier revision parked the textarea at top:-9999
    // which made every keystroke scroll the whole page to the top. The
    // textarea must now sit *inside* the card's relative-positioned
    // container so `focus()` never has to scroll the viewport.
    render(<RunningCommandCard sessionId={SESSION_ID} command="bash" />);
    const card = screen.getByTestId("running-command-card");
    const ta = getCaptureTextarea();
    expect(card.contains(ta)).toBe(true);
    expect(ta.style.top).not.toContain("-9999");
    expect(ta.style.left).not.toContain("-9999");
  });
});
