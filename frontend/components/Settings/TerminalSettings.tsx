import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import type { TerminalSettings as TerminalSettingsType } from "@/lib/settings";

interface TerminalSettingsProps {
  settings: TerminalSettingsType;
  onChange: (settings: TerminalSettingsType) => void;
}

export function TerminalSettings({ settings, onChange }: TerminalSettingsProps) {
  const { t } = useTranslation();
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
          {t("terminal.shell")}
        </label>
        <Input
          id="terminal-shell"
          value={settings.shell || ""}
          onChange={(e) => updateField("shell", e.target.value || null)}
          placeholder={t("terminal.shellPlaceholder")}
        />
        <p className="text-xs text-muted-foreground">{t("terminal.shellDesc")}</p>
      </div>

      {/* Font Family */}
      <div className="space-y-2">
        <label htmlFor="terminal-font-family" className="text-sm font-medium text-foreground">
          {t("terminal.fontFamily")}
        </label>
        <Input
          id="terminal-font-family"
          value={settings.font_family}
          onChange={(e) => updateField("font_family", e.target.value)}
          placeholder="SF Mono"
        />
        <p className="text-xs text-muted-foreground">{t("terminal.fontFamilyDesc")}</p>
      </div>

      {/* Font Size */}
      <div className="space-y-2">
        <label htmlFor="terminal-font-size" className="text-sm font-medium text-foreground">
          {t("terminal.fontSize")}
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
        <p className="text-xs text-muted-foreground">{t("terminal.fontSizeDesc")}</p>
      </div>

      {/* Scrollback */}
      <div className="space-y-2">
        <label htmlFor="terminal-scrollback" className="text-sm font-medium text-foreground">
          {t("terminal.scrollbackLines")}
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
        <p className="text-xs text-muted-foreground">{t("terminal.scrollbackDesc")}</p>
      </div>
    </div>
  );
}
