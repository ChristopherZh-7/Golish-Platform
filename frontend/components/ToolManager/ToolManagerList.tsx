/* ── Tool list body ───────────────────────────────────────────────────
 *
 * Renders the tool grid/list in the non-editor view, with the
 * Required/Optional grouping and the "Install All Required" button strip.
 * Pulled out of `ToolManager.tsx` to keep the main file under the
 * 800-line file-size budget.
 */

import { ChevronDown, Download, Loader2, Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { isAutoInstallMethod } from "./hooks/useToolInstall";
import type { ToolWithMeta, ViewMode } from "./OutputParserEditor";
import { type ActionButtonProps, GridCard, ListRow } from "./ToolCards";

export interface ToolManagerListProps {
  loading: boolean;
  filteredTools: ToolWithMeta[];
  search: string;
  selectedTier: string | null;
  viewMode: ViewMode;
  optionalCollapsed: boolean;
  setOptionalCollapsed: (v: boolean | ((prev: boolean) => boolean)) => void;
  onOpenEditor: (tool: ToolWithMeta) => void;
  onContextMenu: (e: React.MouseEvent, tool: ToolWithMeta) => void;
  onAddTool: () => void;
  actionCtx: ActionButtonProps;
  installBusy: string | null;
  batchInstalling: boolean;
  onInstallAllRequired: (tools: ToolWithMeta[]) => void;
  onCancelBatchInstall: () => void;
}

/** Renders the inner tool collection (grid or list) given an array of
 *  tools. Used both for the flat case and inside Required/Optional
 *  buckets. */
function ToolCollection({
  tools,
  viewMode,
  onOpenEditor,
  onContextMenu,
  actionCtx,
}: {
  tools: ToolWithMeta[];
  viewMode: ViewMode;
  onOpenEditor: ToolManagerListProps["onOpenEditor"];
  onContextMenu: ToolManagerListProps["onContextMenu"];
  actionCtx: ActionButtonProps;
}) {
  if (viewMode === "grid") {
    return (
      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {tools.map((tool) => (
          <GridCard
            key={tool.id}
            tool={tool}
            onOpen={onOpenEditor}
            onContextMenu={onContextMenu}
            actionCtx={actionCtx}
          />
        ))}
      </div>
    );
  }
  return (
    <div className="space-y-1">
      {tools.map((tool) => (
        <ListRow
          key={tool.id}
          tool={tool}
          onOpen={onOpenEditor}
          onContextMenu={onContextMenu}
          actionCtx={actionCtx}
        />
      ))}
    </div>
  );
}

export function ToolManagerList(props: ToolManagerListProps) {
  const {
    loading,
    filteredTools,
    search,
    selectedTier,
    viewMode,
    optionalCollapsed,
    setOptionalCollapsed,
    onOpenEditor,
    onContextMenu,
    onAddTool,
    actionCtx,
    installBusy,
    batchInstalling,
    onInstallAllRequired,
    onCancelBatchInstall,
  } = props;
  const { t } = useTranslation();

  if (loading) {
    return (
      <div className="flex-1 overflow-y-auto px-6 py-4">
        <div key="tm-loading" className="flex items-center justify-center h-32">
          <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/50" />
        </div>
      </div>
    );
  }

  if (filteredTools.length === 0) {
    return (
      <div className="flex-1 overflow-y-auto px-6 py-4">
        <div
          key="tm-empty"
          className="flex flex-col items-center justify-center h-32 gap-2 overflow-hidden"
        >
          <span className="text-[12px] text-muted-foreground/60">
            {search.trim() ? t("toolManager.noMatch") : t("toolManager.noTools")}
          </span>
          {!search.trim() && (
            <button
              type="button"
              onClick={onAddTool}
              className="text-[11px] text-accent/60 hover:text-accent transition-colors flex items-center gap-1"
            >
              <Plus className="w-3 h-3" /> {t("toolManager.addFirstTool")}
            </button>
          )}
        </div>
      </div>
    );
  }

  const shouldGroup = !selectedTier && !search.trim();
  if (!shouldGroup) {
    return (
      <div className="flex-1 overflow-y-auto px-6 py-4">
        <ToolCollection
          tools={filteredTools}
          viewMode={viewMode}
          onOpenEditor={onOpenEditor}
          onContextMenu={onContextMenu}
          actionCtx={actionCtx}
        />
      </div>
    );
  }

  const requiredTools = filteredTools.filter(
    (tool) => tool.tier === "essential" || tool.tier === "recommended"
  );
  const optionalTools = filteredTools.filter((tool) => !tool.tier || tool.tier === "optional");
  const uninstalledRequired = requiredTools.filter(
    (tool) => !tool.installed && isAutoInstallMethod(tool.install?.method)
  );

  return (
    <div className="flex-1 overflow-y-auto px-6 py-4">
      <div className="space-y-6">
        {requiredTools.length > 0 && (
          <div>
            <div className="flex items-center gap-2 mb-3">
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-red-500/10 border border-red-500/20">
                <div className="w-1.5 h-1.5 rounded-full bg-red-500" />
                <span className="text-[11px] font-medium text-red-400">
                  {t("toolManager.requiredSection")}
                </span>
                <span className="text-[10px] text-red-400/50">{requiredTools.length}</span>
              </div>
              <span className="text-[10px] text-muted-foreground/60">
                {t("toolManager.requiredHint")}
              </span>
              {uninstalledRequired.length > 0 && (
                <div className="ml-auto flex items-center gap-1.5">
                  {batchInstalling ? (
                    <button
                      type="button"
                      onClick={onCancelBatchInstall}
                      className="flex items-center gap-1.5 px-3 py-1 rounded-md text-[10px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors"
                    >
                      <X className="w-3 h-3" />
                      {t("common.cancel")}
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onInstallAllRequired(filteredTools)}
                      disabled={!!installBusy}
                      className="flex items-center gap-1.5 px-3 py-1 rounded-md text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                    >
                      <Download className="w-3 h-3" />
                      {t("toolManager.installAllRequired")}
                      <span className="text-[9px] px-1.5 py-px rounded-full bg-accent/15">
                        {uninstalledRequired.length}
                      </span>
                    </button>
                  )}
                </div>
              )}
            </div>
            <ToolCollection
              tools={requiredTools}
              viewMode={viewMode}
              onOpenEditor={onOpenEditor}
              onContextMenu={onContextMenu}
              actionCtx={actionCtx}
            />
          </div>
        )}
        {optionalTools.length > 0 && (
          <div>
            <button
              type="button"
              onClick={() => setOptionalCollapsed((v) => !v)}
              className="flex items-center gap-2 mb-3 w-full group cursor-pointer"
            >
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-muted/40 border border-border/30 group-hover:bg-muted/60 transition-colors">
                <ChevronDown
                  className={cn(
                    "w-3 h-3 text-muted-foreground/50 transition-transform duration-200",
                    optionalCollapsed && "-rotate-90"
                  )}
                />
                <span className="text-[11px] font-medium text-muted-foreground/70">
                  {t("toolManager.optionalSection")}
                </span>
                <span className="text-[10px] text-muted-foreground/50">{optionalTools.length}</span>
              </div>
              <span className="text-[10px] text-muted-foreground/60">
                {t("toolManager.optionalHint")}
              </span>
            </button>
            {!optionalCollapsed && (
              <ToolCollection
                tools={optionalTools}
                viewMode={viewMode}
                onOpenEditor={onOpenEditor}
                onContextMenu={onContextMenu}
                actionCtx={actionCtx}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}
