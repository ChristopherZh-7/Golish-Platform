import { useCallback } from "react";
import { Input } from "@/components/ui/input";
import type { TerminalSettings as TerminalSettingsType } from "@/lib/settings";

interface TerminalSettingsProps {
  settings: TerminalSettingsType;
  onChange: (settings: TerminalSettingsType) => void;
}

export function TerminalSettings({ settings, onChange }: TerminalSettingsProps) {
  const updateField = useCallback(
    <K extends keyof TerminalSettingsType>(key: K, value: TerminalSettingsType[K]) => {
      onChange({ ...settings, [key]: value });
    },
    [settings, onChange]
  );

  return (
    <div className="space-y-6">
      {/* Shell */}
      <div className="space-y-2">
        <label htmlFor="terminal-shell" className="text-sm font-medium text-foreground">
          Shell
        </label>
        <Input
          id="terminal-shell"
          value={settings.shell || ""}
          onChange={(e) => updateField("shell", e.target.value || null)}
          placeholder="Auto-detect from environment"
        />
        <p className="text-xs text-muted-foreground">
          Override the default shell. Leave empty to auto-detect.
        </p>
      </div>

      {/* Font Family */}
      <div className="space-y-2">
        <label htmlFor="terminal-font-family" className="text-sm font-medium text-foreground">
          Font Family
        </label>
        <Input
          id="terminal-font-family"
          value={settings.font_family}
          onChange={(e) => updateField("font_family", e.target.value)}
          placeholder="SF Mono"
        />
        <p className="text-xs text-muted-foreground">Monospace font for the terminal</p>
      </div>

      {/* Font Size */}
      <div className="space-y-2">
        <label htmlFor="terminal-font-size" className="text-sm font-medium text-foreground">
          Font Size
        </label>
        <Input
          id="terminal-font-size"
          type="number"
          min={8}
          max={32}
          value={settings.font_size}
          onChange={(e) => updateField("font_size", parseInt(e.target.value, 10) || 14)}
          className="w-24"
        />
        <p className="text-xs text-muted-foreground">Font size in pixels (8-32)</p>
      </div>

      {/* Scrollback */}
      <div className="space-y-2">
        <label htmlFor="terminal-scrollback" className="text-sm font-medium text-foreground">
          Scrollback Lines
        </label>
        <Input
          id="terminal-scrollback"
          type="number"
          min={1000}
          max={100000}
          step={1000}
          value={settings.scrollback}
          onChange={(e) => updateField("scrollback", parseInt(e.target.value, 10) || 10000)}
          className="w-32"
        />
        <p className="text-xs text-muted-foreground">
          Number of lines to keep in scrollback buffer
        </p>
      </div>

      {/* GridTerminal renderer (Phase B · default since 2026-05) */}
      <div className="space-y-2 rounded-md border border-border/60 p-3">
        <label className="flex items-start gap-2 text-sm font-medium text-foreground">
          <input
            type="checkbox"
            checked={settings.use_grid_renderer !== false}
            onChange={(e) => updateField("use_grid_renderer", e.target.checked)}
            className="mt-0.5 h-4 w-4 cursor-pointer accent-current"
          />
          <span className="flex flex-col gap-1">
            <span>Use GridTerminal renderer for TUI apps</span>
            <span className="text-xs font-normal text-muted-foreground">
              Renders vim / htop / less through a Rust virtual terminal + React grid. Default since
              2026-05 — disabling it leaves alt-screen sessions in a no-renderer state (legacy
              xterm.js was removed in the same release), so toggling off is mostly a switch for
              future fallback renderers. Applies on next session creation.
            </span>
          </span>
        </label>
      </div>
    </div>
  );
}
