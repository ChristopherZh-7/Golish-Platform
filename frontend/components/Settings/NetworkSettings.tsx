import { Input } from "@/components/ui/input";
import type { NetworkSettings as NetworkSettingsType } from "@/lib/settings";

interface NetworkSettingsProps {
  settings: NetworkSettingsType;
  onChange: (settings: NetworkSettingsType) => void;
}

export function NetworkSettings({ settings, onChange }: NetworkSettingsProps) {
  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <label htmlFor="proxy-url" className="text-sm font-medium text-foreground">
          Proxy URL
        </label>
        <Input
          id="proxy-url"
          value={settings.proxy_url || ""}
          onChange={(e) =>
            onChange({ ...settings, proxy_url: e.target.value || null })
          }
          placeholder="http://127.0.0.1:7890"
        />
        <p className="text-xs text-muted-foreground">
          HTTP, HTTPS, or SOCKS5 proxy for all outgoing requests (LLM APIs, web search, content
          fetching)
        </p>
      </div>

      <div className="space-y-2">
        <label htmlFor="no-proxy" className="text-sm font-medium text-foreground">
          No Proxy
        </label>
        <Input
          id="no-proxy"
          value={settings.no_proxy || ""}
          onChange={(e) =>
            onChange({ ...settings, no_proxy: e.target.value || null })
          }
          placeholder="localhost,127.0.0.1,.local"
        />
        <p className="text-xs text-muted-foreground">
          Comma-separated list of hosts or domains that bypass the proxy
        </p>
      </div>

      <p className="text-xs text-muted-foreground border-t border-[var(--border-medium)] pt-4">
        Proxy changes take effect after restarting the application.
      </p>
    </div>
  );
}
