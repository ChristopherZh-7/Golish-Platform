import { useCallback, useState } from "react";
import { HexColorPicker } from "react-colorful";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import {
  type CaretSettings,
  DEFAULT_CARET_SETTINGS,
  type TerminalSettings as TerminalSettingsType,
} from "@/lib/settings";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import {
  type DisplaySettings,
  defaultDisplaySettings,
  selectDisplaySettings,
} from "@/store/slices";
import { CaretPreview } from "./CaretPreview";
import { ThemePicker } from "./ThemePicker";

interface ToggleRowProps {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  /** Gray out when a parent toggle makes this irrelevant */
  dimmed?: boolean;
}

function ToggleRow({ id, label, description, checked, onCheckedChange, dimmed }: ToggleRowProps) {
  return (
    <div
      className={cn(
        "flex items-center justify-between",
        dimmed && "opacity-40 pointer-events-none"
      )}
    >
      <div className="space-y-1">
        <label htmlFor={id} className="text-sm font-medium text-foreground cursor-pointer">
          {label}
        </label>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  );
}

interface AppearanceSettingsProps {
  terminalSettings?: TerminalSettingsType;
  onTerminalChange?: (settings: TerminalSettingsType) => void;
}

export function AppearanceSettings({
  terminalSettings,
  onTerminalChange,
}: AppearanceSettingsProps) {
  const displaySettings = useStore(selectDisplaySettings);
  const setDisplaySettings = useStore((state) => state.setDisplaySettings);

  // Caret settings (from terminal settings in settings.toml)
  const caret: CaretSettings = terminalSettings?.caret ?? DEFAULT_CARET_SETTINGS;

  const updateTerminalField = useCallback(
    <K extends keyof TerminalSettingsType>(key: K, value: TerminalSettingsType[K]) => {
      if (terminalSettings && onTerminalChange) {
        onTerminalChange({ ...terminalSettings, [key]: value });
      }
    },
    [terminalSettings, onTerminalChange]
  );

  const updateCaret = useCallback(
    <K extends keyof CaretSettings>(key: K, value: CaretSettings[K]) => {
      updateTerminalField("caret", { ...caret, [key]: value });
    },
    [caret, updateTerminalField]
  );

  const [showColorPicker, setShowColorPicker] = useState(false);

  const update = (patch: Partial<DisplaySettings>) => {
    setDisplaySettings({ ...displaySettings, ...patch });
  };

  const visibilityKeys: Array<keyof DisplaySettings> = [
    "showTabBar",
    "showHomeTab",
    "showFileEditorButton",
    "showHistoryButton",
    "showSettingsButton",
    "showNotificationBell",
    "showTerminalContext",
    "showWorkingDirectory",
    "showGitBranch",
    "showStatusBar",
    "showInputModeToggle",
    "showStatusBadge",
    "showAgentModeSelector",
    "showContextUsage",
    "showMcpBadge",
  ];
  const allShown = visibilityKeys.every((k) => displaySettings[k]);
  const allHidden = visibilityKeys.every((k) => !displaySettings[k]);

  const tabBarSubOptions: Array<keyof DisplaySettings> = [
    "showHomeTab",
    "showFileEditorButton",
    "showHistoryButton",
    "showSettingsButton",
    "showNotificationBell",
  ];

  const contextBarSubOptions: Array<keyof DisplaySettings> = [
    "showWorkingDirectory",
    "showGitBranch",
  ];

  const statusBarSubOptions: Array<keyof DisplaySettings> = [
    "showInputModeToggle",
    "showStatusBadge",
    "showAgentModeSelector",
    "showContextUsage",
    "showMcpBadge",
  ];

  const tabBarParentOn =
    displaySettings.showTabBar || tabBarSubOptions.some((k) => displaySettings[k]);
  const contextBarParentOn =
    displaySettings.showTerminalContext || contextBarSubOptions.some((k) => displaySettings[k]);
  const statusBarParentOn =
    displaySettings.showStatusBar || statusBarSubOptions.some((k) => displaySettings[k]);

  return (
    <div className="space-y-8">
      {/* Theme */}
      <div className="space-y-2">
        <h3 className="text-sm font-medium text-foreground mb-4">Theme</h3>
        <ThemePicker />
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* UI Scale */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">UI Scale</h3>
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-foreground">Zoom Level</span>
            <span className="text-xs text-muted-foreground tabular-nums">
              {Math.round((displaySettings.uiScale ?? 1.1) * 100)}%
            </span>
          </div>
          <Slider
            value={[(displaySettings.uiScale ?? 1.1) * 100]}
            onValueChange={([v]: number[]) => update({ uiScale: Math.round(v) / 100 })}
            min={75}
            max={150}
            step={5}
            className="w-full"
          />
          <div className="flex items-center justify-between">
            <p className="text-xs text-muted-foreground">
              Scale the entire UI (75% – 150%). Useful if text or buttons feel too small.
            </p>
            {(displaySettings.uiScale ?? 1.1) !== 1.1 && (
              <button
                type="button"
                className="text-xs text-accent hover:underline"
                onClick={() => update({ uiScale: 1.1 })}
              >
                Reset to 110%
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Input Caret */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">Input Caret</h3>

        {/* Preview */}
        <CaretPreview settings={caret} />

        {/* Style selector */}
        <div className="space-y-2">
          <span className="text-sm font-medium text-foreground">Style</span>
          <Select
            value={caret.style}
            onValueChange={(value: "block" | "default") => updateCaret("style", value)}
          >
            <SelectTrigger className="w-40">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="default">Default</SelectItem>
              <SelectItem value="block">Block</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Default uses the native browser text caret. Block renders a customizable overlay.
          </p>
        </div>

        {/* Block-specific settings */}
        {caret.style === "block" && (
          <div className="space-y-4 pl-2 border-l-2 border-[var(--border-subtle)]">
            {/* Width */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-foreground">Width</span>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {caret.width.toFixed(1)}ch
                </span>
              </div>
              <Slider
                value={[caret.width]}
                onValueChange={([v]: number[]) => updateCaret("width", Math.round(v * 10) / 10)}
                min={0.1}
                max={3.0}
                step={0.1}
                className="w-full"
              />
              <p className="text-xs text-muted-foreground">
                Caret width in character units (0.1–3.0)
              </p>
            </div>

            {/* Color */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-foreground">Color</span>
                {caret.color && (
                  <button
                    type="button"
                    className="text-xs text-accent hover:underline"
                    onClick={() => updateCaret("color", null)}
                  >
                    Reset to theme default
                  </button>
                )}
              </div>
              <div className="flex items-center gap-3">
                <Popover open={showColorPicker} onOpenChange={setShowColorPicker}>
                  <PopoverTrigger asChild>
                    <button
                      type="button"
                      className="h-8 w-8 rounded-md border border-[var(--border-subtle)] shrink-0"
                      style={{ backgroundColor: caret.color ?? "var(--foreground)" }}
                      aria-label="Pick caret color"
                    />
                  </PopoverTrigger>
                  <PopoverContent align="start" className="w-auto p-3">
                    <HexColorPicker
                      color={caret.color ?? "#ffffff"}
                      onChange={(color: string) => updateCaret("color", color)}
                    />
                  </PopoverContent>
                </Popover>
                <Input
                  value={caret.color ?? ""}
                  onChange={(e) => {
                    const val = e.target.value;
                    if (val === "") {
                      updateCaret("color", null);
                    } else {
                      updateCaret("color", val);
                    }
                  }}
                  placeholder="Theme default"
                  className="w-32 font-mono text-xs"
                />
              </div>
              <p className="text-xs text-muted-foreground">
                Hex color for the caret. Leave empty to use the theme foreground color.
              </p>
            </div>

            {/* Blink Speed */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-foreground">Blink Speed</span>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {caret.blink_speed === 0 ? "No blink" : `${caret.blink_speed}ms`}
                </span>
              </div>
              <Slider
                value={[caret.blink_speed]}
                onValueChange={([v]: number[]) =>
                  updateCaret("blink_speed", Math.round(v / 10) * 10)
                }
                min={0}
                max={2000}
                step={10}
                className="w-full"
              />
              <p className="text-xs text-muted-foreground">
                Blink cycle duration in milliseconds. Set to 0 to disable blinking.
              </p>
            </div>

            {/* Opacity */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-foreground">Opacity</span>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {Math.round(caret.opacity * 100)}%
                </span>
              </div>
              <Slider
                value={[caret.opacity]}
                onValueChange={([v]: number[]) => updateCaret("opacity", Math.round(v * 100) / 100)}
                min={0}
                max={1.0}
                step={0.01}
                className="w-full"
              />
              <p className="text-xs text-muted-foreground">Caret opacity (0%–100%)</p>
            </div>
          </div>
        )}
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Section header */}
      <div className="space-y-1">
        <h2 className="text-base font-semibold text-foreground">UI Customization</h2>
        <p className="text-sm text-muted-foreground">
          Fine-grained customization of UI elements and components
        </p>
      </div>

      {/* General */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">General</h3>
        <ToggleRow
          id="hide-ai-settings-in-shell-mode"
          label="Hide AI Settings in Shell Mode"
          description="Hide model badge, token usage, agent mode, and MCP badge when in shell mode"
          checked={displaySettings.hideAiSettingsInShellMode}
          onCheckedChange={(checked) => update({ hideAiSettingsInShellMode: checked })}
        />
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Tab Bar */}
      <div className="space-y-4">
        <ToggleRow
          id="show-tab-bar-buttons"
          label="Tab Bar"
          description="Show application icons in the top tab bar"
          checked={tabBarParentOn}
          onCheckedChange={(checked) => {
            if (checked) {
              update({ showTabBar: true });
            } else {
              update({
                showTabBar: false,
                ...Object.fromEntries(tabBarSubOptions.map((k) => [k, false])),
              });
            }
          }}
        />
        <div
          className={cn(
            "space-y-4 pl-4 border-l-2 border-[var(--border-subtle)]",
            !tabBarParentOn && "opacity-40 pointer-events-none"
          )}
        >
          <ToggleRow
            id="show-home-tab"
            label="Home Tab"
            description="Show the home tab in the tab bar"
            checked={displaySettings.showHomeTab}
            onCheckedChange={(checked) => update({ showHomeTab: checked })}
          />
          <ToggleRow
            id="show-file-editor-button"
            label="File Editor"
            description="Show the file editor panel button"
            checked={displaySettings.showFileEditorButton}
            onCheckedChange={(checked) => update({ showFileEditorButton: checked })}
          />
          <ToggleRow
            id="show-history-button"
            label="Session History"
            description="Show the session history button"
            checked={displaySettings.showHistoryButton}
            onCheckedChange={(checked) => update({ showHistoryButton: checked })}
          />
          <ToggleRow
            id="show-settings-button"
            label="Settings"
            description="Show the settings button"
            checked={displaySettings.showSettingsButton}
            onCheckedChange={(checked) => update({ showSettingsButton: checked })}
          />
          <ToggleRow
            id="show-notification-bell"
            label="Notification Bell"
            description="Show the notification bell"
            checked={displaySettings.showNotificationBell}
            onCheckedChange={(checked) => update({ showNotificationBell: checked })}
          />
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Terminal Context */}
      <div className="space-y-4">
        <ToggleRow
          id="show-terminal-context"
          label="Context Bar"
          description="Show context information above the terminal input"
          checked={contextBarParentOn}
          onCheckedChange={(checked) => {
            if (checked) {
              update({ showTerminalContext: true });
            } else {
              update({
                showTerminalContext: false,
                ...Object.fromEntries(contextBarSubOptions.map((k) => [k, false])),
              });
            }
          }}
        />
        <div
          className={cn(
            "space-y-4 pl-4 border-l-2 border-[var(--border-subtle)]",
            !contextBarParentOn && "opacity-40 pointer-events-none"
          )}
        >
          <ToggleRow
            id="show-working-directory"
            label="Working Directory"
            description="Show the current working directory path badge"
            checked={displaySettings.showWorkingDirectory}
            onCheckedChange={(checked) => update({ showWorkingDirectory: checked })}
          />
          <ToggleRow
            id="show-git-branch"
            label="Git Branch"
            description="Show the git branch and diff stats badge"
            checked={displaySettings.showGitBranch}
            onCheckedChange={(checked) => update({ showGitBranch: checked })}
          />
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Status Bar */}
      <div className="space-y-4">
        <ToggleRow
          id="show-status-bar"
          label="Status Bar"
          description="Show the bottom status bar"
          checked={statusBarParentOn}
          onCheckedChange={(checked) => {
            if (checked) {
              update({ showStatusBar: true });
            } else {
              update({
                showStatusBar: false,
                ...Object.fromEntries(statusBarSubOptions.map((k) => [k, false])),
              });
            }
          }}
        />
        <div
          className={cn(
            "space-y-4 pl-4 border-l-2 border-[var(--border-subtle)]",
            !statusBarParentOn && "opacity-40 pointer-events-none"
          )}
        >
          <ToggleRow
            id="show-input-mode-toggle"
            label="Input Mode Toggle"
            description="Show the full Terminal / AI segmented toggle instead of collapsing it"
            checked={displaySettings.showInputModeToggle}
            onCheckedChange={(checked) => update({ showInputModeToggle: checked })}
          />
          <ToggleRow
            id="show-status-badge"
            label="Status & Model Badge"
            description="Show the connection status and active model name badge"
            checked={displaySettings.showStatusBadge}
            onCheckedChange={(checked) => update({ showStatusBadge: checked })}
          />
          <ToggleRow
            id="show-agent-mode-selector"
            label="Agent Mode Selector"
            description="Show the agent mode (Auto / Plan / etc.) dropdown"
            checked={displaySettings.showAgentModeSelector}
            onCheckedChange={(checked) => update({ showAgentModeSelector: checked })}
          />
          <ToggleRow
            id="show-context-usage"
            label="Token Usage"
            description="Show the context window / token usage percentage badge"
            checked={displaySettings.showContextUsage}
            onCheckedChange={(checked) => update({ showContextUsage: checked })}
          />
          <ToggleRow
            id="show-mcp-badge"
            label="MCP Servers Badge"
            description="Show the MCP servers connected indicator"
            checked={displaySettings.showMcpBadge}
            onCheckedChange={(checked) => update({ showMcpBadge: checked })}
          />
        </div>
      </div>
      {/* Quick actions */}
      <div className="flex items-center gap-3">
        <p className="text-xs text-muted-foreground">Choose which UI elements are visible.</p>
        <span className="text-xs text-muted-foreground/50">·</span>
        <button
          type="button"
          disabled={allShown}
          onClick={() =>
            setDisplaySettings({
              showTabBar: true,
              showHomeTab: true,
              showFileEditorButton: true,
              showHistoryButton: true,
              showSettingsButton: true,
              showNotificationBell: true,
              showTerminalContext: true,
              showWorkingDirectory: true,
              showGitBranch: true,
              showStatusBar: true,
              showInputModeToggle: true,
              showStatusBadge: true,
              showAgentModeSelector: true,
              showContextUsage: true,
              showMcpBadge: true,
              hideAiSettingsInShellMode: displaySettings.hideAiSettingsInShellMode,
              uiScale: displaySettings.uiScale ?? 1.1,
            })
          }
          className="text-xs text-accent hover:underline disabled:opacity-40 disabled:no-underline disabled:cursor-not-allowed"
        >
          Show all
        </button>
        <span className="text-xs text-muted-foreground/50">·</span>
        <button
          type="button"
          disabled={allHidden}
          onClick={() =>
            setDisplaySettings({
              showTabBar: false,
              showHomeTab: false,
              showFileEditorButton: false,
              showHistoryButton: false,
              showSettingsButton: false,
              showNotificationBell: false,
              showTerminalContext: false,
              showWorkingDirectory: false,
              showGitBranch: false,
              showStatusBar: false,
              showInputModeToggle: false,
              showStatusBadge: false,
              showAgentModeSelector: false,
              showContextUsage: false,
              showMcpBadge: false,
              hideAiSettingsInShellMode: displaySettings.hideAiSettingsInShellMode,
              uiScale: displaySettings.uiScale ?? 1.1,
            })
          }
          className="text-xs text-accent hover:underline disabled:opacity-40 disabled:no-underline disabled:cursor-not-allowed"
        >
          Hide all
        </button>
        <span className="text-xs text-muted-foreground/50">·</span>
        <button
          type="button"
          onClick={() => setDisplaySettings({ ...defaultDisplaySettings })}
          className="text-xs text-accent hover:underline"
        >
          Reset to defaults
        </button>
      </div>
    </div>
  );
}
