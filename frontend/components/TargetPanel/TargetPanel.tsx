import { useCallback, useState } from "react";
import {
  Crosshair, GitFork, LayoutList, Loader2, Shield,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { TargetGraphView } from "@/components/TargetPanel/TargetGraphView";
import { TargetListView } from "./TargetListView";
import { useTargetData } from "./hooks/useTargetData";

import { lazy, Suspense } from "react";
const SecurityViewLazy = lazy(() =>
  import("@/components/SecurityView/SecurityView").then((m) => ({ default: m.SecurityView }))
);

type TargetTab = "targets" | "security";
type TargetViewMode = "list" | "graph";

export function TargetPanel() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TargetTab>("targets");
  const [viewMode, setViewMode] = useState<TargetViewMode>("list");
  const [scanTarget, setScanTarget] = useState<{ id: string; value: string } | null>(null);

  const {
    safeTargets, stats,
    handleAdd, handleBatchAdd, handleDelete,
    handleToggleScope, handleUpdateNotes, handleClearAll,
  } = useTargetData();

  const openScanTools = useCallback((target: { id: string; value: string }) => {
    setScanTarget({ id: target.id, value: target.value });
    setActiveTab("security");
  }, []);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border/50">
        <div className="flex items-center gap-1">
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "targets"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("targets")}
          >
            <Crosshair className="w-3.5 h-3.5" />
            {t("targets.title")}
            <span className="text-[10px] text-muted-foreground/60 tabular-nums">{stats.total}</span>
          </button>
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "security"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("security")}
          >
            <Shield className="w-3.5 h-3.5" />
            {t("security.title", "Security")}
          </button>

          {activeTab === "targets" && (
            <>
              <div className="w-px h-4 bg-border/30 mx-1" />
              <div className="flex items-center rounded-md border border-border/30 overflow-hidden">
                <button
                  type="button"
                  className={cn(
                    "p-1.5 transition-colors",
                    viewMode === "list" ? "bg-accent/15 text-accent" : "text-muted-foreground hover:text-foreground hover:bg-muted/30",
                  )}
                  onClick={() => setViewMode("list")}
                  title={t("targets.listView", "List View")}
                >
                  <LayoutList className="w-3.5 h-3.5" />
                </button>
                <button
                  type="button"
                  className={cn(
                    "p-1.5 transition-colors",
                    viewMode === "graph" ? "bg-accent/15 text-accent" : "text-muted-foreground hover:text-foreground hover:bg-muted/30",
                  )}
                  onClick={() => setViewMode("graph")}
                  title={t("targets.graphView", "Graph View")}
                >
                  <GitFork className="w-3.5 h-3.5" />
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      {activeTab === "targets" && viewMode === "graph" ? (
        <div className="flex-1 min-h-0">
          <TargetGraphView targets={safeTargets} />
        </div>
      ) : activeTab === "security" ? (
        <div className="flex-1 min-h-0 overflow-hidden">
          <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" /></div>}>
            <SecurityViewLazy
              initialScanTarget={scanTarget ?? undefined}
            />
          </Suspense>
        </div>
      ) : (
        <TargetListView
          targets={safeTargets}
          stats={stats}
          t={t}
          onAdd={handleAdd}
          onBatchAdd={handleBatchAdd}
          onDelete={handleDelete}
          onToggleScope={handleToggleScope}
          onUpdateNotes={handleUpdateNotes}
          onClearAll={handleClearAll}
          onScan={openScanTools}
        />
      )}
    </div>
  );
}
