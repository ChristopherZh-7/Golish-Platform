import { AlertTriangle, BookOpen, Code, Shield } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TopTab } from "./types";
import { useTranslation } from "react-i18next";

export function VulnKbTopBar({ activeTab, onTabChange }: { activeTab: TopTab; onTabChange: (tab: TopTab) => void }) {
  const { t } = useTranslation();
  const tabs: { id: TopTab; icon: typeof Shield; label: string }[] = [
    { id: "intel", icon: Shield, label: t("vulnKb.intelTab", "Intel") },
    { id: "wiki", icon: BookOpen, label: t("vulnKb.wikiTab", "Wiki") },
    { id: "poc-library", icon: Code, label: t("vulnKb.pocTab", "PoC Library") },
  ];

  return (
    <div className="flex items-center gap-1 px-3 py-2 border-b border-border/20 flex-shrink-0">
      <AlertTriangle className="w-3.5 h-3.5 text-accent/70 mr-1" />
      <span className="text-[11px] font-medium mr-3">{t("vulnKb.title", "Vulnerability KB")}</span>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          className={cn(
            "flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors",
            activeTab === tab.id ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground hover:bg-muted/10"
          )}
        >
          <tab.icon className="w-3 h-3" />
          {tab.label}
        </button>
      ))}
    </div>
  );
}

