import { Input } from "@/components/ui/input";
import type { NetworkSettings as NetworkSettingsType } from "@/lib/settings";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";

const PROXY_PRESETS = [
  { label: "Surge HTTP", url: "http://127.0.0.1:6152" },
  { label: "Surge SOCKS", url: "socks5://127.0.0.1:6153" },
  { label: "Clash", url: "http://127.0.0.1:7890" },
  { label: "Clash SOCKS", url: "socks5://127.0.0.1:7891" },
  { label: "V2Ray", url: "http://127.0.0.1:10809" },
  { label: "SS", url: "socks5://127.0.0.1:1080" },
] as const;

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
        <div className="flex flex-wrap gap-1.5 pt-1">
          {PROXY_PRESETS.map((p) => (
            <button
              key={p.url}
              type="button"
              className={cn(
                "rounded-md border px-2 py-0.5 text-[11px] transition-colors",
                settings.proxy_url === p.url
                  ? "border-accent/40 bg-accent/10 text-accent"
                  : "border-border/40 bg-muted/20 text-muted-foreground hover:border-border/60 hover:text-foreground",
              )}
              onClick={() => onChange({ ...settings, proxy_url: p.url })}
            >
              {p.label}
            </button>
          ))}
        </div>
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
