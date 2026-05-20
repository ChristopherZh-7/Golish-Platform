/**
 * IntelProvidersSettings — Settings panel for ASM intel platforms.
 *
 * Lists every registered provider (0.zone / FOFA / Quake / Hunter / Shodan)
 * and lets the user configure an API key per provider via vault storage.
 *
 * Architecture:
 * - Backend exposes `intel_list_providers` / `intel_test_connection` /
 *   `intel_query_provider`. Keys are stored as `vault_entries` rows with
 *   name=<provider_id> + entry_type=`api_key`.
 * - This component pulls the provider list on mount and renders one
 *   `ProviderCard` per entry.
 */

import { Loader2, Network } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { intel } from "@/lib/api";
import type { ProviderMeta } from "@/lib/api/intel";
import { notify } from "@/lib/notify";
import { ProviderCard } from "./ProviderCard";

export function IntelProvidersSettings() {
  const [providers, setProviders] = useState<ProviderMeta[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setIsLoading(true);
    intel
      .listProviders()
      .then((list) => {
        if (!cancelled) setProviders(list);
      })
      .catch((err) => {
        console.error("Failed to load intel providers:", err);
        if (!cancelled) notify.error("Failed to load intel providers");
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const refresh = useCallback(() => setRefreshKey((k) => k + 1), []);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="w-5 h-5 text-muted-foreground animate-spin" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <header>
        <div className="flex items-center gap-2 mb-1">
          <Network className="w-4 h-4 text-accent" />
          <h2 className="text-base font-semibold text-foreground">情报源（Intel Providers）</h2>
        </div>
        <p className="text-sm text-muted-foreground">
          ASM 网络空间测绘平台 API 配置。配好 key 后即可在 organizations
          详情页一键拉取域名/IP/邮箱/证书等情报，自动入库。
        </p>
      </header>

      <div className="space-y-3">
        {providers.map((p) => (
          <ProviderCard key={`${p.id}-${refreshKey}`} meta={p} onChanged={refresh} />
        ))}
      </div>

      {providers.length === 0 && (
        <div className="text-sm text-muted-foreground py-8 text-center">
          暂无 intel provider 注册。请检查后端 commands_registry 是否包含 intel_list_providers。
        </div>
      )}
    </div>
  );
}
