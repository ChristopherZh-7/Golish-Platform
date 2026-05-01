import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, List, ShieldAlert, ShieldCheck, ShieldX } from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import type { ZapAlert } from "@/lib/pentest/types";

export function riskColor(risk: string) {
  const c: Record<string, string> = { High: "text-red-400 bg-red-500/10", Medium: "text-orange-400 bg-orange-500/10", Low: "text-yellow-400 bg-yellow-500/10", Informational: "text-blue-400 bg-blue-500/10" };
  return c[risk] || "text-muted-foreground bg-muted/20";
}

export function riskIcon(risk: string) {
  if (risk === "High") return <ShieldX className="w-3 h-3" />;
  if (risk === "Medium") return <ShieldAlert className="w-3 h-3" />;
  return <ShieldCheck className="w-3 h-3" />;
}

export function AlertCard({
  alert,
}: {
  alert: ZapAlert;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="rounded-xl border border-border/10 bg-[var(--bg-hover)]/15 overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-start gap-3 p-3 text-left hover:bg-[var(--bg-hover)]/30 transition-colors"
      >
        <span className={cn("p-1 rounded-md flex-shrink-0 mt-0.5", riskColor(alert.risk))}>
          {riskIcon(alert.risk)}
        </span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[12px] font-medium text-foreground">{alert.name}</span>
            <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(alert.risk))}>
              {alert.risk}
            </span>
          </div>
          <p className="text-[10px] text-muted-foreground/40 truncate mt-0.5 font-mono">
            {alert.method} {alert.url}
          </p>
        </div>
        {expanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/30 mt-1" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/30 mt-1" />}
      </button>
      {expanded && (
        <div className="px-3 pb-3 space-y-2 text-[11px]">
          {alert.description && (
            <div>
              <span className="text-muted-foreground/40 text-[10px] font-medium">Description</span>
              <p className="text-foreground/70 mt-0.5">{alert.description}</p>
            </div>
          )}
          {alert.solution && (
            <div>
              <span className="text-muted-foreground/40 text-[10px] font-medium">Solution</span>
              <p className="text-foreground/70 mt-0.5">{alert.solution}</p>
            </div>
          )}
          {alert.evidence && (
            <div>
              <span className="text-muted-foreground/40 text-[10px] font-medium">Evidence</span>
              <pre className="text-foreground/50 mt-0.5 font-mono text-[10px] bg-[var(--bg-hover)]/30 p-2 rounded-lg overflow-x-auto">
                {alert.evidence}
              </pre>
            </div>
          )}
          {alert.param && (
            <div className="flex gap-2">
              <span className="text-muted-foreground/40 text-[10px] font-medium">Parameter:</span>
              <code className="text-accent/70 text-[10px]">{alert.param}</code>
            </div>
          )}
          <div className="flex gap-3 text-[9px] text-muted-foreground/30">
            {alert.cweid !== "-1" && <span>CWE-{alert.cweid}</span>}
            {alert.wascid !== "-1" && <span>WASC-{alert.wascid}</span>}
            <span>Plugin: {alert.pluginId}</span>
          </div>
        </div>
      )}
    </div>
  );
}

function ScanLogRow({ log }: { log: any }) {
  const [expanded, setExpanded] = useState(false);
  const isVuln = log.result === "vulnerable" || log.result === "potential";
  const detail = typeof log.detail === "string" ? JSON.parse(log.detail || "{}") : (log.detail || {});

  return (
    <div className={cn("rounded-lg border transition-colors", isVuln ? "border-red-500/20 bg-red-500/5" : "border-border/5 bg-transparent hover:bg-[var(--bg-hover)]/20")}>
      <button type="button" onClick={() => setExpanded(!expanded)} className="w-full flex items-center gap-2 px-3 py-1.5 text-left">
        <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0", isVuln ? "bg-red-400" : "bg-muted-foreground/15")} />
        <span className="text-[10px] font-medium text-muted-foreground/50 w-[80px] flex-shrink-0 truncate">{log.test_type || "unknown"}</span>
        <span className="text-[10px] font-mono text-foreground/50 flex-1 truncate">{log.parameter || "-"}</span>
        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", isVuln ? "bg-red-500/10 text-red-400" : "text-muted-foreground/30 bg-muted/10")}>
          {log.result}
        </span>
        {detail.status_code && (
          <span className="text-[9px] text-muted-foreground/30 font-mono">{detail.status_code}</span>
        )}
        {detail.response_time_ms != null && (
          <span className="text-[9px] text-muted-foreground/20 font-mono w-[40px] text-right">{detail.response_time_ms}ms</span>
        )}
        {expanded ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/20 flex-shrink-0" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20 flex-shrink-0" />}
      </button>
      {expanded && (
        <div className="px-3 pb-2 space-y-1.5 border-t border-border/5">
          {log.payload && (
            <div className="mt-1.5">
              <span className="text-[9px] text-muted-foreground/40 font-medium">Payload</span>
              <pre className="text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/30 p-2 rounded-lg overflow-x-auto mt-0.5 whitespace-pre-wrap break-all">{log.payload}</pre>
            </div>
          )}
          {log.url && (
            <div>
              <span className="text-[9px] text-muted-foreground/40 font-medium">URL</span>
              <p className="text-[10px] font-mono text-foreground/40 truncate">{log.url}</p>
            </div>
          )}
          {log.evidence && (
            <div>
              <span className="text-[9px] text-muted-foreground/40 font-medium">Evidence</span>
              <pre className="text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/30 p-2 rounded-lg overflow-x-auto mt-0.5 whitespace-pre-wrap break-all">{log.evidence}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function ScanDetailTabs({ alerts, scanLogs }: {
  alerts: ZapAlert[];
  scanLogs: any[];
}) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"alerts" | "tests">(alerts.length > 0 ? "alerts" : "tests");
  const vulnCount = scanLogs.filter((l) => l.result === "vulnerable" || l.result === "potential").length;
  const testTypes = useMemo(() => {
    const map = new Map<string, number>();
    for (const l of scanLogs) {
      map.set(l.test_type || "unknown", (map.get(l.test_type || "unknown") || 0) + 1);
    }
    return [...map.entries()].sort((a, b) => b[1] - a[1]);
  }, [scanLogs]);

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="flex items-center gap-1 px-4 py-1.5 border-b border-border/10 flex-shrink-0">
        <button className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "alerts" ? "bg-accent/10 text-accent" : "text-muted-foreground/40 hover:text-muted-foreground/70")} onClick={() => setTab("alerts")}>
          {t("security.alertsTab", "Alerts")} ({alerts.length})
        </button>
        <button className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "tests" ? "bg-accent/10 text-accent" : "text-muted-foreground/40 hover:text-muted-foreground/70")} onClick={() => setTab("tests")}>
          {t("security.testsTab", "Test Details")} ({scanLogs.length})
        </button>
        {tab === "tests" && vulnCount > 0 && (
          <span className="text-[9px] text-red-400 font-medium ml-1">{vulnCount} vulnerable</span>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {tab === "alerts" ? (
          alerts.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20">
              <ShieldCheck className="w-10 h-10" />
              <p className="text-[12px]">{t("security.scanNoResults")}</p>
            </div>
          ) : (
            <div className="px-4 py-3 space-y-2">
              <div className="flex items-center gap-2 mb-2">
                <span className="text-[11px] font-medium text-foreground/60">{alerts.length} {t("security.alertsTotal")}</span>
                {(() => {
                  const vulnTypes = new Set(alerts.map((a) => a.name));
                  return <span className="text-[10px] text-muted-foreground/30">{vulnTypes.size} {t("security.uniqueVulnTypes")}</span>;
                })()}
              </div>
              {alerts.map((alert) => (
                <AlertCard key={`${alert.id}-${alert.url}`} alert={alert} />
              ))}
            </div>
          )
        ) : (
          scanLogs.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20">
              <List className="w-10 h-10" />
              <p className="text-[12px]">{t("security.noTestLogs", "No test logs available")}</p>
            </div>
          ) : (
            <div className="px-4 py-3 space-y-1">
              {testTypes.length > 1 && (
                <div className="flex flex-wrap gap-1.5 mb-3">
                  {testTypes.map(([type, count]) => (
                    <span key={type} className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/20 text-muted-foreground/50">
                      {type} ({count})
                    </span>
                  ))}
                </div>
              )}
              <div className="space-y-0.5">
                {scanLogs.map((log) => (
                  <ScanLogRow key={log.id} log={log} />
                ))}
              </div>
            </div>
          )
        )}
      </div>
    </div>
  );
}
