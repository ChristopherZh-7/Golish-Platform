import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CustomSelect } from "@/components/ui/custom-select";
import { Switch } from "@/components/ui/switch";
import type { AdvancedSettings as AdvancedSettingsType, PrivacySettings } from "@/lib/settings";
import { DispatchInflightSection } from "./DispatchInflightSection";
import { KnowledgeGraphSection } from "./KnowledgeGraphSection";

interface AdvancedSettingsProps {
  settings: AdvancedSettingsType;
  privacy: PrivacySettings;
  onChange: (settings: AdvancedSettingsType) => void;
  onPrivacyChange: (privacy: PrivacySettings) => void;
}

function SimpleSelect({
  value,
  onValueChange,
  options,
}: {
  id?: string;
  value: string;
  onValueChange: (value: string) => void;
  options: { value: string; label: string }[];
}) {
  return <CustomSelect value={value} onChange={onValueChange} options={options} />;
}

export function AdvancedSettings({
  settings,
  privacy,
  onChange,
  onPrivacyChange,
}: AdvancedSettingsProps) {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string>("...");

  useEffect(() => {
    if (import.meta.env.DEV) {
      setVersion("dev");
    } else {
      getVersion()
        .then(setVersion)
        .catch(() => setVersion("unknown"));
    }
  }, []);

  const logLevelOptions = [
    { value: "error", label: t("advancedSettings.logLevels.error") },
    { value: "warn", label: t("advancedSettings.logLevels.warn") },
    { value: "info", label: t("advancedSettings.logLevels.info") },
    { value: "debug", label: t("advancedSettings.logLevels.debug") },
    { value: "trace", label: t("advancedSettings.logLevels.trace") },
  ];

  return (
    <div className="space-y-6">
      {/* Log Level */}
      <div className="space-y-2">
        <label htmlFor="advanced-log-level" className="text-sm font-medium text-foreground">
          {t("advancedSettings.logLevel")}
        </label>
        <SimpleSelect
          id="advanced-log-level"
          value={settings.log_level}
          onValueChange={(value) =>
            onChange({ ...settings, log_level: value as AdvancedSettingsType["log_level"] })
          }
          options={logLevelOptions}
        />
        <p className="text-xs text-muted-foreground">{t("advancedSettings.logLevelDesc")}</p>
      </div>

      {/* Experimental Features */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <label htmlFor="advanced-experimental" className="text-sm font-medium text-foreground">
            {t("advancedSettings.experimental")}
          </label>
          <p className="text-xs text-muted-foreground">{t("advancedSettings.experimentalDesc")}</p>
        </div>
        <Switch
          id="advanced-experimental"
          checked={settings.enable_experimental}
          onCheckedChange={(checked) => onChange({ ...settings, enable_experimental: checked })}
        />
      </div>

      {/* LLM API Logs */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <label htmlFor="advanced-llm-api-logs" className="text-sm font-medium text-foreground">
            {t("advancedSettings.llmApiLogs")}
          </label>
          <p className="text-xs text-muted-foreground">{t("advancedSettings.llmApiLogsDesc")}</p>
        </div>
        <Switch
          id="advanced-llm-api-logs"
          checked={settings.enable_llm_api_logs}
          onCheckedChange={(checked) => onChange({ ...settings, enable_llm_api_logs: checked })}
        />
      </div>

      {/* Extract Raw SSE */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <label htmlFor="advanced-extract-raw-sse" className="text-sm font-medium text-foreground">
            {t("advancedSettings.extractRawSse")}
          </label>
          <p className="text-xs text-muted-foreground">{t("advancedSettings.extractRawSseDesc")}</p>
        </div>
        <Switch
          id="advanced-extract-raw-sse"
          checked={settings.extract_raw_sse}
          onCheckedChange={(checked) => onChange({ ...settings, extract_raw_sse: checked })}
        />
      </div>

      {/* Privacy Section */}
      <div className="space-y-4 p-4 rounded-lg bg-muted border border-[var(--border-medium)]">
        <h4 className="text-sm font-medium text-accent">{t("advancedSettings.privacy")}</h4>

        {/* Usage Statistics */}
        <div className="flex items-center justify-between">
          <div className="space-y-1">
            <label htmlFor="privacy-usage-stats" className="text-sm text-foreground">
              {t("advancedSettings.usageStats")}
            </label>
            <p className="text-xs text-muted-foreground">{t("advancedSettings.usageStatsDesc")}</p>
          </div>
          <Switch
            id="privacy-usage-stats"
            checked={privacy.usage_statistics}
            onCheckedChange={(checked) =>
              onPrivacyChange({ ...privacy, usage_statistics: checked })
            }
          />
        </div>

        {/* Log Prompts */}
        <div className="flex items-center justify-between">
          <div className="space-y-1">
            <label htmlFor="privacy-log-prompts" className="text-sm text-foreground">
              {t("advancedSettings.logPrompts")}
            </label>
            <p className="text-xs text-muted-foreground">{t("advancedSettings.logPromptsDesc")}</p>
          </div>
          <Switch
            id="privacy-log-prompts"
            checked={privacy.log_prompts}
            onCheckedChange={(checked) => onPrivacyChange({ ...privacy, log_prompts: checked })}
          />
        </div>
      </div>

      {/* Knowledge Graph snapshot */}
      <div className="pt-4 border-t border-[var(--border-medium)]">
        <KnowledgeGraphSection />
      </div>

      {/* Sub-agent dispatch monitor */}
      <div className="pt-4 border-t border-[var(--border-medium)]">
        <DispatchInflightSection />
      </div>

      {/* Version */}
      <div className="pt-4 border-t border-[var(--border-medium)]">
        <div className="flex items-center justify-between">
          <span className="text-sm text-muted-foreground">{t("advancedSettings.version")}</span>
          <span className="text-sm font-mono text-muted-foreground">{version}</span>
        </div>
      </div>
    </div>
  );
}
