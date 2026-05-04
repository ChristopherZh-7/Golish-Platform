/* ── ToolManager top bar ──────────────────────────────────────────────
 *
 * Renders the top bar for both states:
 *   - "list" mode: title + count + import/add/updates/refresh buttons.
 *   - "edit" mode: back arrow + tool icon/name + dirty dot + mode tabs +
 *     save button (skills mode has its own save action).
 *
 * Pulled out of `ToolManager.tsx` to keep the main file under the
 * 800-line file-size budget. Owns no state — it consumes the editor /
 * data / install / github / skill hook outputs as props.
 */

import {
  ArrowLeft,
  ArrowUpCircle,
  ArrowUpDown,
  BookOpen,
  Code2,
  FileText,
  GitFork,
  Loader2,
  Plus,
  RefreshCw,
  Save,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { ToolWithMeta } from "./OutputParserEditor";

interface UpdateInfo {
  has_update: boolean;
}

export interface ToolManagerHeaderProps {
  editingTool: ToolWithMeta | null;
  editorVisible: boolean;
  editorMode: "form" | "raw" | "skills" | "output";
  editorDirty: boolean;
  saving: boolean;
  closeEditor: () => void;
  handleSwitchMode: (mode: "form" | "raw" | "skills" | "output") => void;
  handleSave: () => void;

  skillDirty: boolean;
  skillSaving: boolean;
  handleSaveSkill: () => void;

  toolsCount: number;
  installedCount: number;
  loading: boolean;
  loadData: () => void;

  toolUpdates: UpdateInfo[];
  checkingUpdates: boolean;
  checkForUpdates: () => void;

  onImportGithub: () => void;
  onAddTool: () => void;
}

export function ToolManagerHeader(props: ToolManagerHeaderProps) {
  const { t } = useTranslation();
  const {
    editingTool,
    editorVisible,
    editorMode,
    editorDirty,
    saving,
    closeEditor,
    handleSwitchMode,
    handleSave,
    skillDirty,
    skillSaving,
    handleSaveSkill,
    toolsCount,
    installedCount,
    loading,
    loadData,
    toolUpdates,
    checkingUpdates,
    checkForUpdates,
    onImportGithub,
    onAddTool,
  } = props;

  return (
    <div className="flex items-center justify-between px-6 py-4 border-b border-border/15 flex-shrink-0">
      {editingTool ? (
        <>
          <div
            className={cn(
              "flex items-center gap-3 transition-all duration-[180ms] ease-out",
              editorVisible ? "opacity-100 translate-x-0" : "opacity-0 translate-x-2"
            )}
          >
            <button
              type="button"
              onClick={closeEditor}
              className="p-1.5 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
            >
              <ArrowLeft className="w-4 h-4" />
            </button>
            <div>
              <div className="flex items-center gap-2">
                {editingTool.icon && <span className="text-[14px]">{editingTool.icon}</span>}
                <h1 className="text-[16px] font-semibold text-foreground">{editingTool.name}</h1>
                {(editorDirty || skillDirty) && (
                  <span
                    className="w-2 h-2 rounded-full bg-accent/60 flex-shrink-0"
                    title={t("toolManager.unsavedChanges")}
                  />
                )}
              </div>
              <p className="text-[11px] text-muted-foreground/50 mt-0.5">
                {t("toolManager.editToolConfig")}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <div className="flex items-center rounded-lg border border-border/15 overflow-hidden">
              {(["form", "skills", "output", "raw"] as const).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  onClick={() => handleSwitchMode(mode)}
                  className={cn(
                    "flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                    editorMode === mode
                      ? "bg-accent/15 text-accent"
                      : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]"
                  )}
                >
                  {mode === "form" && (
                    <>
                      <FileText className="w-3 h-3" /> {t("toolManager.form")}
                    </>
                  )}
                  {mode === "skills" && (
                    <>
                      <BookOpen className="w-3 h-3" /> Skills
                    </>
                  )}
                  {mode === "output" && (
                    <>
                      <ArrowUpDown className="w-3 h-3" /> Output
                    </>
                  )}
                  {mode === "raw" && (
                    <>
                      <Code2 className="w-3 h-3" /> {t("toolManager.json")}
                    </>
                  )}
                </button>
              ))}
            </div>
            {editorMode === "skills" ? (
              <button
                type="button"
                onClick={handleSaveSkill}
                disabled={skillSaving || !skillDirty}
                className={cn(
                  "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                  skillDirty
                    ? "bg-accent text-accent-foreground hover:bg-accent/90"
                    : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed"
                )}
              >
                {skillSaving ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Save className="w-3 h-3" />
                )}{" "}
                {t("common.save")}
              </button>
            ) : (
              <button
                type="button"
                onClick={handleSave}
                disabled={saving || !editorDirty}
                className={cn(
                  "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                  editorDirty
                    ? "bg-accent text-accent-foreground hover:bg-accent/90"
                    : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed"
                )}
              >
                {saving ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Save className="w-3 h-3" />
                )}{" "}
                {t("common.save")}
              </button>
            )}
          </div>
        </>
      ) : (
        <>
          <div>
            <h1 className="text-[16px] font-semibold text-foreground">{t("toolManager.title")}</h1>
            <p className="text-[11px] text-muted-foreground/50 mt-0.5">
              {t("toolManager.toolCount", { count: toolsCount, installed: installedCount })}
            </p>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={onImportGithub}
              title={t("toolManager.importGithub")}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
            >
              <GitFork className="w-3.5 h-3.5" /> {t("toolManager.importGithub")}
            </button>
            <button
              type="button"
              onClick={onAddTool}
              title={t("toolManager.addTool")}
              className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors"
            >
              <Plus className="w-4 h-4" />
            </button>
            <button
              type="button"
              onClick={checkForUpdates}
              disabled={checkingUpdates}
              title="Check for Updates"
              className={cn(
                "p-2 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-30",
                toolUpdates.some((u) => u.has_update) && "text-amber-400"
              )}
            >
              <ArrowUpCircle className={cn("w-4 h-4", checkingUpdates && "animate-spin")} />
            </button>
            <button
              type="button"
              onClick={loadData}
              disabled={loading}
              title={t("common.refresh")}
              className="p-2 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-30"
            >
              <RefreshCw className={cn("w-4 h-4", loading && "animate-spin")} />
            </button>
          </div>
        </>
      )}
    </div>
  );
}
