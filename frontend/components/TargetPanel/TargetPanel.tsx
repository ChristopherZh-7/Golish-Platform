import { Building2, Crosshair, GitFork, Loader2, Shield } from "lucide-react";
import { lazy, Suspense, useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { TargetGraphView } from "@/components/TargetPanel/TargetGraphView";
import { cn } from "@/lib/utils";
import { useTargetData } from "./hooks/useTargetData";
import { TargetGroupedView } from "./TargetGroupedView";

const SecurityViewLazy = lazy(() =>
  import("@/components/SecurityView/SecurityView").then((m) => ({ default: m.SecurityView }))
);

type TargetTab = "targets" | "security";

/**
 * Two views after the Schema E "unified panel" refactor (2026-05-17):
 *
 * - `tree`  — primary view. Organizations form the spine and every target
 *             attaches to one org node. Per-node hover actions handle the
 *             full lifecycle (create / edit / delete org and target).
 * - `graph` — node-link visualisation, useful for quick relationship checks.
 *
 * The old `list` view (flat target table) was retired — its only differentiating
 * features (search / batch import / domain grouping) can be reintroduced inside
 * the tree's top bar later if users miss them.
 */
type TargetViewMode = "tree" | "graph";

const ALL_VIEW_MODES: TargetViewMode[] = ["tree", "graph"];

export function TargetPanel() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TargetTab>("targets");
  const [viewMode, setViewMode] = useState<TargetViewMode>("tree");
  const [scanTarget, setScanTarget] = useState<{ id: string; value: string } | null>(null);

  const { safeTargets, stats, handleAdd, handleDelete, handleToggleScope, handleUpdateNotes } =
    useTargetData();

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
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40"
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
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40"
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
                {ALL_VIEW_MODES.map((mode) => (
                  <ViewModeButton
                    key={mode}
                    active={viewMode === mode}
                    onClick={() => setViewMode(mode)}
                    title={t(`targets.${mode}View`)}
                  >
                    {mode === "tree" ? (
                      <Building2 className="w-3.5 h-3.5" />
                    ) : (
                      <GitFork className="w-3.5 h-3.5" />
                    )}
                  </ViewModeButton>
                ))}
              </div>
            </>
          )}
        </div>
      </div>

      {activeTab === "security" ? (
        <div className="flex-1 min-h-0 overflow-hidden">
          <Suspense
            fallback={
              <div className="h-full flex items-center justify-center">
                <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" />
              </div>
            }
          >
            <SecurityViewLazy initialScanTarget={scanTarget ?? undefined} />
          </Suspense>
        </div>
      ) : viewMode === "graph" ? (
        <div className="flex-1 min-h-0">
          <TargetGraphView targets={safeTargets} />
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-hidden">
          <TargetGroupedView
            targets={safeTargets}
            t={t}
            onAdd={handleAdd}
            onDelete={handleDelete}
            onToggleScope={handleToggleScope}
            onUpdateNotes={handleUpdateNotes}
            onScan={openScanTools}
          />
        </div>
      )}
    </div>
  );
}

interface ViewModeButtonProps {
  active: boolean;
  onClick: () => void;
  title: string;
  children: React.ReactNode;
}

function ViewModeButton({ active, onClick, title, children }: ViewModeButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        "p-1.5 transition-colors",
        active
          ? "bg-accent/15 text-accent"
          : "text-muted-foreground hover:text-foreground hover:bg-muted/30"
      )}
      onClick={onClick}
      title={title}
      aria-pressed={active}
    >
      {children}
    </button>
  );
}
