/**
 * IntegrationsSettings — Settings panel for external-service credentials.
 *
 * Replaces the legacy `IntelProvidersSettings` tab. Lists every
 * schema-declared integration (ASM intel providers + tool credentials
 * like ENScan_GO cookies + GitHub Token) and renders one
 * [`IntegrationCard`] per entry with category-based filtering and
 * fuzzy search.
 *
 * Three states the parent should be aware of:
 *  - **loading**: integrations have not finished resolving server-side
 *  - **error**:   resolver call failed (likely backend not registered)
 *  - **empty**:   resolver succeeded but returned an empty list
 *                 (no `integration` blocks anywhere in the project)
 */

import { AlertCircle, Inbox, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { integrations as integrationsApi } from "@/lib/api";
import type { ResolvedIntegration } from "@/lib/api/integrations";
import { CategoryNav, matchSearch } from "./CategoryNav";
import { IntegrationCard } from "./IntegrationCard";

type LoadStatus = "loading" | "ready" | "error";

export function IntegrationsSettings() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<LoadStatus>("loading");
  const [integrations, setIntegrations] = useState<ResolvedIntegration[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [activeCategory, setActiveCategory] = useState<string>("all");
  const [search, setSearch] = useState<string>("");

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setError(null);
    integrationsApi
      .listSchemas()
      .then((list) => {
        if (cancelled) return;
        setIntegrations(list);
        setStatus("ready");
      })
      .catch((err) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
        setStatus("error");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const filtered = useMemo(() => {
    return integrations.filter((it) => {
      if (activeCategory !== "all" && it.schema.category !== activeCategory) {
        return false;
      }
      return matchSearch(it, search);
    });
  }, [integrations, activeCategory, search]);

  return (
    <div className="space-y-4">
      <header className="space-y-1">
        <h2 className="text-sm font-medium text-foreground">{t("integrations.title")}</h2>
        <p className="text-[11px] text-muted-foreground/70 leading-relaxed">
          {t("integrations.headerDesc")}
        </p>
      </header>

      {status === "loading" && <LoadingSkeleton />}

      {status === "error" && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2.5 text-[11px] text-red-400 flex items-start gap-2">
          <AlertCircle className="w-3 h-3 flex-shrink-0 mt-0.5" />
          <div className="space-y-1">
            <div className="font-medium">{t("integrations.loadFailed")}</div>
            <div className="text-red-400/80 break-all">{error}</div>
          </div>
        </div>
      )}

      {status === "ready" && integrations.length === 0 && (
        <div className="flex flex-col items-center justify-center py-16 text-muted-foreground/30">
          <Inbox className="w-8 h-8 mb-3" />
          <p className="text-[12px] font-medium text-muted-foreground/70">
            {t("integrations.empty")}
          </p>
          <p className="text-[10px] text-muted-foreground/40 mt-1 max-w-sm text-center leading-relaxed">
            {t("integrations.emptyHint")}
          </p>
        </div>
      )}

      {status === "ready" && integrations.length > 0 && (
        <div className="grid grid-cols-[160px_1fr] gap-4">
          <aside>
            <CategoryNav
              integrations={integrations}
              activeCategory={activeCategory}
              onCategoryChange={setActiveCategory}
              search={search}
              onSearchChange={setSearch}
            />
          </aside>
          <section className="space-y-1.5 min-w-0">
            {filtered.length === 0 ? (
              <div className="text-[11px] text-muted-foreground/60 py-8 text-center">
                {t("integrations.noMatch")}
              </div>
            ) : (
              filtered.map((it) => (
                <IntegrationCard
                  key={it.tool_id}
                  integration={it}
                  defaultOpen={Boolean(search.trim()) && filtered.length <= 2}
                />
              ))
            )}
          </section>
        </div>
      )}
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div className="flex items-center gap-2 text-[11px] text-muted-foreground/60 py-8 justify-center">
      <Loader2 className="w-3 h-3 animate-spin" />
    </div>
  );
}
