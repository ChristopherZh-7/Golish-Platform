import { useCallback, useState } from "react";
import { HexColorPicker } from "react-colorful";
import { useTranslation } from "react-i18next";
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
import { getLanguagePreference, type LanguagePreference, setLanguagePreference } from "@/lib/i18n";
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
  const { t } = useTranslation();
  const displaySettings = useStore(selectDisplaySettings);
  const setDisplaySettings = useStore((state) => state.setDisplaySettings);
  const [langPref, setLangPref] = useState<LanguagePreference>(getLanguagePreference);

  const handleLanguageChange = useCallback(async (next: LanguagePreference) => {
    setLangPref(next);
    try {
      await setLanguagePreference(next);
    } catch (e) {
      // Surface failures instead of silently no-op'ing.
      // eslint-disable-next-line no-console
      console.error("[i18n] setLanguagePreference failed:", e);
    }
  }, []);

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

  // Only flags that drive a real UI element are listed here. Defunct toggles
  // (file editor / history / settings / notification-bell buttons, status bar
  // row, model badge, git branch badge) were removed in 2026-05 — keeping
  // them in the bulk Show/Hide actions would silently flip dead state.
  const visibilityKeys: Array<keyof DisplaySettings> = [
    "showHomeTab",
    "showTerminalContext",
    "showWorkingDirectory",
    "showInputModeToggle",
    "showContextUsage",
    "showMcpBadge",
  ];
  const allShown = visibilityKeys.every((k) => displaySettings[k]);
  const allHidden = visibilityKeys.every((k) => !displaySettings[k]);

  const contextBarSubOptions: Array<keyof DisplaySettings> = ["showWorkingDirectory"];
  const contextBarParentOn =
    displaySettings.showTerminalContext || contextBarSubOptions.some((k) => displaySettings[k]);

  return (
    <div className="space-y-8">
      {/* Language */}
      <div className="space-y-2">
        <h3 className="text-sm font-medium text-foreground mb-4">{t("settings.language")}</h3>
        <div className="flex items-start justify-between gap-4">
          <p className="text-xs text-muted-foreground flex-1">{t("settings.languageHint")}</p>
          <Select
            value={langPref}
            onValueChange={(v: LanguagePreference) => void handleLanguageChange(v)}
          >
            <SelectTrigger className="w-44 shrink-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="system">{t("settings.languageSystem")}</SelectItem>
              <SelectItem value="zh-CN">{t("settings.languageZh")}</SelectItem>
              <SelectItem value="en">{t("settings.languageEn")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

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
          description="Hide token usage and MCP badge when in shell mode"
          checked={displaySettings.hideAiSettingsInShellMode}
          onCheckedChange={(checked) => update({ hideAiSettingsInShellMode: checked })}
        />
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Tab Bar */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">Tab Bar</h3>
        <ToggleRow
          id="show-home-tab"
          label="Home Tab"
          description="Show the home tab in the tab bar"
          checked={displaySettings.showHomeTab}
          onCheckedChange={(checked) => update({ showHomeTab: checked })}
        />
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
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Input Status Indicators */}
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">Status Indicators</h3>
        <ToggleRow
          id="show-input-mode-toggle"
          label="Input Mode Toggle"
          description="Show the full Terminal / AI segmented toggle instead of collapsing it"
          checked={displaySettings.showInputModeToggle}
          onCheckedChange={(checked) => update({ showInputModeToggle: checked })}
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

      {/* Quick actions */}
      <div className="flex items-center gap-3">
        <p className="text-xs text-muted-foreground">Choose which UI elements are visible.</p>
        <span className="text-xs text-muted-foreground/50">·</span>
        <button
          type="button"
          disabled={allShown}
          onClick={() =>
            setDisplaySettings({
              showHomeTab: true,
              showTerminalContext: true,
              showWorkingDirectory: true,
              showInputModeToggle: true,
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
              showHomeTab: false,
              showTerminalContext: false,
              showWorkingDirectory: false,
              showInputModeToggle: false,
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
