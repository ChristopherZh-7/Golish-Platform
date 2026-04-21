import { History } from "lucide-react";
import { cn } from "@/lib/utils";
import type { VulnLink, ScanHistoryEntry } from "./types";
import { useTranslation } from "react-i18next";

export function HistoryTab({ link }: { link: VulnLink }) {
  const { t } = useTranslation();

  const resultBadge = (result: ScanHistoryEntry["result"]) => {
    const map: Record<string, { cls: string; label: string }> = {
      vulnerable: { cls: "text-red-400 bg-red-500/10", label: t("vulnIntel.vulnerable", "Vulnerable") },
      not_vulnerable: { cls: "text-green-400 bg-green-500/10", label: t("vulnIntel.notVulnerable", "Not Vulnerable") },
      error: { cls: "text-yellow-400 bg-yellow-500/10", label: t("common.error") },
      pending: { cls: "text-zinc-400 bg-zinc-500/10", label: t("vulnIntel.pending", "Pending") },
    };
    const m = map[result] || map.pending;
    return <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full font-medium", m.cls)}>{m.label}</span>;
  };

  if (link.scanHistory.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-6 gap-2 text-muted-foreground/20">
        <History className="w-8 h-8" />
        <p className="text-[10px]">{t("vulnIntel.noHistory", "No scan history")}</p>
        <p className="text-[9px] text-muted-foreground/15">{t("vulnIntel.historyHint", "Scan history will appear here when you test targets with this vulnerability's PoC")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-1">
      {link.scanHistory.map((entry, i) => (
        <div key={`${entry.target}-${entry.date}-${i}`} className="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-muted/5 transition-colors">
          {resultBadge(entry.result)}
          <span className="text-[10px] font-mono text-foreground/60 truncate flex-1">{entry.target}</span>
          <span className="text-[8px] text-muted-foreground/20">{new Date(entry.date).toLocaleDateString()}</span>
          {entry.details && <span className="text-[8px] text-muted-foreground/30 max-w-[150px] truncate">{entry.details}</span>}
        </div>
      ))}
    </div>
  );
}

