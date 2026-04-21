import { useEffect, useMemo, useState } from "react";
import {
  ChevronRight, Loader2, ShieldAlert, ShieldCheck, ShieldX,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { zapGetAlerts, zapGetAlertCount } from "@/lib/pentest/zap-api";
import type { ZapAlert } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { AlertCard } from "./ScannerPanel";

export function ScanResultsView({ alerts }: { alerts: ZapAlert[] }) {
  const { t } = useTranslation();
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const riskColor = (risk: string) => {
    const c: Record<string, string> = {
      High: "text-red-400 bg-red-500/10",
      Medium: "text-orange-400 bg-orange-500/10",
      Low: "text-yellow-400 bg-yellow-500/10",
      Informational: "text-blue-400 bg-blue-500/10",
    };
    return c[risk] || "text-muted-foreground bg-muted/20";
  };

  const riskIcon = (risk: string) => {
    if (risk === "High") return <ShieldX className="w-3 h-3" />;
    if (risk === "Medium") return <ShieldAlert className="w-3 h-3" />;
    return <ShieldCheck className="w-3 h-3" />;
  };

  if (alerts.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground/20">
        <div className="flex flex-col items-center gap-2">
          <ShieldCheck className="w-8 h-8" />
          <p className="text-[12px]">{t("security.scanNoResults")}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {alerts.map((a) => (
        <div key={a.id} className="border-b border-border/5">
          <button
            type="button"
            onClick={() => setExpandedId(expandedId === a.id ? null : a.id)}
            className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-[var(--bg-hover)]/20 transition-colors"
          >
            <ChevronRight className={cn("w-3 h-3 transition-transform text-muted-foreground/30", expandedId === a.id && "rotate-90")} />
            <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(a.risk))}>
              {riskIcon(a.risk)}
            </span>
            <span className="text-[11px] font-medium flex-1 truncate">{a.name}</span>
            <span className="text-[9px] text-muted-foreground/30 font-mono truncate max-w-[120px]">
              {a.param && `${t("security.param")}: ${a.param}`}
            </span>
            <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(a.risk))}>
              {a.risk}
            </span>
          </button>
          {expandedId === a.id && (
            <div className="px-8 pb-3 space-y-2">
              <p className="text-[10px] text-foreground/60 leading-relaxed">{a.description}</p>
              {a.param && (
                <div>
                  <span className="text-[9px] text-muted-foreground/40 font-medium">{t("security.param")}:</span>
                  <span className="text-[10px] text-orange-400 ml-1 font-mono">{a.param}</span>
                </div>
              )}
              {a.evidence && (
                <div>
                  <span className="text-[9px] text-muted-foreground/40 font-medium">{t("security.evidence")}:</span>
                  <pre className="text-[10px] text-foreground/50 font-mono mt-0.5 bg-muted/10 rounded p-1.5 overflow-x-auto">{a.evidence}</pre>
                </div>
              )}
              {a.solution && (
                <div>
                  <span className="text-[9px] text-muted-foreground/40 font-medium">{t("security.solution")}:</span>
                  <p className="text-[10px] text-green-400/60 mt-0.5 leading-relaxed">{a.solution}</p>
                </div>
              )}
              <div className="flex items-center gap-3 text-[9px] text-muted-foreground/30">
                <span>CWE-{a.cweid}</span>
                <span>{a.method} {a.url}</span>
              </div>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

// ── Alerts Panel ──

export function AlertsPanel() {
  const { t } = useTranslation();
  const [alerts, setAlerts] = useState<ZapAlert[]>([]);
  const [loading, setLoading] = useState(true);
  const [count, setCount] = useState(0);

  useEffect(() => {
    async function load() {
      setLoading(true);
      try {
        const [items, total] = await Promise.all([
          zapGetAlerts(undefined, 0, 500),
          zapGetAlertCount(),
        ]);
        setAlerts(items);
        setCount(total);
      } catch { /* ignore */ }
      setLoading(false);
    }
    load();
  }, []);

  const riskColor = (risk: string) => {
    const c: Record<string, string> = {
      High: "text-red-400 bg-red-500/10",
      Medium: "text-orange-400 bg-orange-500/10",
      Low: "text-yellow-400 bg-yellow-500/10",
      Informational: "text-blue-400 bg-blue-500/10",
    };
    return c[risk] || "text-muted-foreground bg-muted/20";
  };

  const riskIcon = (risk: string) => {
    if (risk === "High") return <ShieldX className="w-3 h-3" />;
    if (risk === "Medium") return <ShieldAlert className="w-3 h-3" />;
    return <ShieldCheck className="w-3 h-3" />;
  };

  const grouped = useMemo(() => {
    const groups: Record<string, ZapAlert[]> = {};
    for (const a of alerts) {
      (groups[a.risk] ??= []).push(a);
    }
    return groups;
  }, [alerts]);

  const riskOrder = ["High", "Medium", "Low", "Informational"];

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-4 py-2 border-b border-border/10 flex-shrink-0">
        <span className="text-[11px] text-muted-foreground/40">
          {count} {t("security.alertsTotal")}
        </span>
        <div className="flex items-center gap-2">
          {riskOrder.map((risk) => {
            const c = grouped[risk]?.length || 0;
            return c > 0 ? (
              <span key={risk} className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(risk))}>
                {risk}: {c}
              </span>
            ) : null;
          })}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {alerts.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <ShieldCheck className="w-12 h-12" />
            <p className="text-[13px] font-medium">{t("security.noAlerts")}</p>
          </div>
        ) : (
          <div className="space-y-2">
            {riskOrder.map((risk) =>
              (grouped[risk] || []).map((alert) => (
                <AlertCard
                  key={`${alert.id}-${alert.url}`}
                  alert={alert}
                  riskColor={riskColor}
                  riskIcon={riskIcon}
                />
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}

