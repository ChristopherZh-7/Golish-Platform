import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_CARET_SETTINGS,
  type TerminalSettings as TerminalSettingsType,
} from "@/lib/settings";

import { TerminalSettings } from "./TerminalSettings";

const baseSettings: TerminalSettingsType = {
  shell: null,
  font_family: "JetBrains Mono",
  font_size: 14,
  scrollback: 10000,
  fullterm_commands: [],
  caret: { ...DEFAULT_CARET_SETTINGS },
};

describe("TerminalSettings", () => {
  it("should render shell input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("terminal.shell")).toBeInTheDocument();
  });

  it("should render font family input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("terminal.fontFamily")).toBeInTheDocument();
  });

  it("should render font size input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("terminal.fontSize")).toBeInTheDocument();
  });

  it("should render scrollback input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("terminal.scrollbackLines")).toBeInTheDocument();
  });

  it("should not render theme or caret settings", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.queryByText("Theme")).not.toBeInTheDocument();
    expect(screen.queryByText("Input Caret")).not.toBeInTheDocument();
  });
});
