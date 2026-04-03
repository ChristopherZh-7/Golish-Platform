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
    expect(screen.getByLabelText("Shell")).toBeInTheDocument();
  });

  it("should render font family input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("Font Family")).toBeInTheDocument();
  });

  it("should render font size input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("Font Size")).toBeInTheDocument();
  });

  it("should render scrollback input", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.getByLabelText("Scrollback Lines")).toBeInTheDocument();
  });

  it("should not render theme or caret settings", () => {
    const onChange = vi.fn();
    render(<TerminalSettings settings={baseSettings} onChange={onChange} />);
    expect(screen.queryByText("Theme")).not.toBeInTheDocument();
    expect(screen.queryByText("Input Caret")).not.toBeInTheDocument();
  });
});
