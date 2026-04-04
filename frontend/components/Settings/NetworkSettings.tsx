import { Input } from "@/components/ui/input";
import type { NetworkSettings as NetworkSettingsType } from "@/lib/settings";
import { useTranslation } from "react-i18next";

interface NetworkSettingsProps {
  settings: NetworkSettingsType;
  onChange: (settings: NetworkSettingsType) => void;
}

export function NetworkSettings({ settings, onChange }: NetworkSettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <label htmlFor="proxy-url" className="text-sm font-medium text-foreground">
          {t("network.proxyUrl")}
        </label>
        <Input
          id="proxy-url"
          value={settings.proxy_url || ""}
          onChange={(e) =>
            onChange({ ...settings, proxy_url: e.target.value || null })
          }
          placeholder={t("network.proxyPlaceholder")}
        />
        <p className="text-xs text-muted-foreground">
          {t("network.proxyHint")}
        </p>
      </div>

      <div className="space-y-2">
        <label htmlFor="no-proxy" className="text-sm font-medium text-foreground">
          {t("network.noProxy")}
        </label>
        <Input
          id="no-proxy"
          value={settings.no_proxy || ""}
          onChange={(e) =>
            onChange({ ...settings, no_proxy: e.target.value || null })
          }
          placeholder={t("network.noProxyPlaceholder")}
        />
        <p className="text-xs text-muted-foreground">
          {t("network.noProxyHint")}
        </p>
      </div>

      <div className="space-y-2 border-t border-[var(--border-medium)] pt-4">
        <label htmlFor="github-token" className="text-sm font-medium text-foreground">
          {t("network.githubToken")}
        </label>
        <Input
          id="github-token"
          type="password"
          value={settings.github_token || ""}
          onChange={(e) =>
            onChange({ ...settings, github_token: e.target.value || null })
          }
          placeholder={t("network.githubTokenPlaceholder")}
        />
        <p className="text-xs text-muted-foreground">
          {t("network.githubTokenHint")}
          <a href="https://github.com/settings/tokens" target="_blank" rel="noreferrer"
            className="text-accent hover:underline ml-1">{t("network.generateToken")}</a>
        </p>
      </div>

      <p className="text-xs text-muted-foreground border-t border-[var(--border-medium)] pt-4">
        {t("network.proxyRestart")}
      </p>
    </div>
  );
}
