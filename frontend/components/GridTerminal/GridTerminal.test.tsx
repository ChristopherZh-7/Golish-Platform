import { render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearTerminalAutoFocusSuppression,
  suppressTerminalAutoFocus,
} from "@/lib/terminal/terminalAutoFocus";
import { GridTerminal } from "./GridTerminal";

vi.mock("@/lib/api/shell", () => ({
  imeGetSource: vi.fn(() => Promise.resolve("com.apple.keylayout.ABC")),
  imeSetSource: vi.fn(() => Promise.resolve()),
}));

vi.mock("./useGridKeyboard", () => ({
  useGridKeyboard: vi.fn(),
}));

vi.mock("./useGridResize", () => ({
  useGridResize: vi.fn(),
}));

vi.mock("./useGridState", () => ({
  useGridState: vi.fn(() => ({
    appCursorMode: false,
    rows: [],
    rowCount: 0,
  })),
}));

describe("GridTerminal auto-focus", () => {
  afterEach(() => {
    clearTerminalAutoFocusSuppression("grid-s1");
    vi.restoreAllMocks();
  });

  it("auto-focuses normally when suppression is not active", async () => {
    const focusSpy = vi.spyOn(HTMLElement.prototype, "focus").mockImplementation(() => {});

    render(<GridTerminal sessionId="grid-s1" />);

    await waitFor(() => expect(focusSpy).toHaveBeenCalled());
  });

  it("does not auto-focus during a chat-first suppression window", async () => {
    const focusSpy = vi.spyOn(HTMLElement.prototype, "focus").mockImplementation(() => {});
    suppressTerminalAutoFocus("grid-s1", 5000);

    render(<GridTerminal sessionId="grid-s1" />);

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(focusSpy).not.toHaveBeenCalled();
  });
});
