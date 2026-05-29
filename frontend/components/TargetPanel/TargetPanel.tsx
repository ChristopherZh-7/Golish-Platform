import { Building2, Crosshair, GitFork } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TargetGraphView } from "@/components/TargetPanel/TargetGraphView";
import { cn } from "@/lib/utils";
import { useTargetData } from "./hooks/useTargetData";
import { TargetGroupedView } from "./TargetGroupedView";

/**
 * The Target Manager is now centered on one primary org tree + selected target
 * workbench, while keeping the existing topology entry visible for the upcoming
 * relationship graph redesign.
 */
type TargetViewMode = "tree" | "graph";

export function TargetPanel() {
  const { t } = useTranslation();
  const [viewMode, setViewMode] = useState<TargetViewMode>("tree");

  const {
    safeTargets,
    stats,
    handleAdd,
    handleBatchAdd,
    handleDelete,
    handleToggleScope,
    handleUpdateNotes,
  } = useTargetData();

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border/50">
        <div className="flex items-center gap-1">
          <div className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium text-foreground">
            <Crosshair className="w-3.5 h-3.5 text-blue-400" />
            {t("targets.title")}
            <span className="text-[10px] text-muted-foreground/70 tabular-nums">{stats.total}</span>
          </div>
          <div className="ml-2 flex items-center overflow-hidden rounded-md border border-border/30 bg-background/20">
            <ViewModeButton
              active={viewMode === "tree"}
              icon={<Building2 className="w-3 h-3" />}
              label="Tree"
              onClick={() => setViewMode("tree")}
              title={t("targets.treeView")}
            />
            <ViewModeButton
              active={viewMode === "graph"}
              icon={<GitFork className="w-3 h-3" />}
              label="Topology"
              onClick={() => setViewMode("graph")}
              title={t("targets.graphView")}
            />
          </div>
        </div>
      </div>

      {viewMode === "graph" ? (
        <div className="flex-1 min-h-0">
          <TargetGraphView targets={safeTargets} />
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-hidden">
          <TargetGroupedView
            targets={safeTargets}
            t={t}
            onAdd={handleAdd}
            onBatchAdd={handleBatchAdd}
            onDelete={handleDelete}
            onToggleScope={handleToggleScope}
            onUpdateNotes={handleUpdateNotes}
          />
        </div>
      )}
    </div>
  );
}

function ViewModeButton({
  active,
  icon,
  label,
  onClick,
  title,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex h-7 items-center gap-1 border-r border-border/25 px-2 text-[10px] transition-colors last:border-r-0",
        active
          ? "bg-muted/30 text-foreground"
          : "text-muted-foreground hover:bg-muted/20 hover:text-foreground"
      )}
      onClick={onClick}
      title={title}
      aria-pressed={active}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
