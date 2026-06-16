/* ── ToolManager ──────────────────────────────────────────────────────
 *
 * Pen-test tool management view. The bulk of the rendering work is now
 * delegated to a handful of focused subcomponents in this directory:
 *
 *   - ToolManagerHeader   — top bar (list-mode + edit-mode)
 *   - ToolManagerToolbar  — search / filters / sort / view-mode switch
 *   - ToolManagerList     — required/optional grid or list
 *   - ToolManagerEditor   — form / raw / skills / output panes
 *   - ToolManagerDialogs  — context menu + confirm/picker dialogs
 *
 * Hook composition stays here — it's where the shared state lives and
 * where small glue callbacks are easiest to wire up. Big JSX blocks were
 * the reason this file sat at 1044 lines before; having it broken up
 * keeps the file under the 800-line file-size budget enforced by
 * `scripts/check_file_sizes.sh`.
 */

import { X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { copyToClipboard } from "@/lib/clipboard";
import { openToolDirectory } from "@/lib/pentest/api";
import type { CtxMenuState } from "./Dialogs";
import { useGithubImport } from "./hooks/useGithubImport";
import { useSkillEditor } from "./hooks/useSkillEditor";
import { useToolData } from "./hooks/useToolData";
import { useToolEditor } from "./hooks/useToolEditor";
import { useToolInstall } from "./hooks/useToolInstall";
import type { ToolWithMeta, ViewMode } from "./OutputParserEditor";
import type { ActionButtonProps } from "./ToolCards";
import { ToolManagerDialogs } from "./ToolManagerDialogs";
import { ToolManagerEditor } from "./ToolManagerEditor";
import { ToolManagerHeader } from "./ToolManagerHeader";
import { ToolManagerList } from "./ToolManagerList";
import { ToolManagerToolbar } from "./ToolManagerToolbar";

export function ToolManager() {
  const { t } = useTranslation();

  // ── Data: loading, filtering, sorting ──
  const data = useToolData();

  // ── Editor: form/raw/skills/output mode, save, open/close ──
  const editor = useToolEditor(data.loadData, data.setError);

  // ── Install: install, uninstall, updates, permissions ──
  const install = useToolInstall(data.loadData, data.setError);

  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [optionalCollapsed, setOptionalCollapsed] = useState(true);

  // ── Skills editor (extracted hook) ──
  const skills = useSkillEditor({
    toolName: editor.editingTool?.name ?? null,
    skillsList: editor.skillsList,
    setSkillsList: editor.setSkillsList,
    skillDirty: editor.skillDirty,
    setSkillDirty: editor.setSkillDirty,
  });

  // ── GitHub import (extracted hook) ──
  const github = useGithubImport({ openEditor: editor.openEditor, setError: data.setError });

  // ── Context menu ──
  const [ctxMenu, setCtxMenu] = useState<CtxMenuState | null>(null);

  useEffect(() => {
    const dismiss = () => setCtxMenu(null);
    window.addEventListener("click", dismiss);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("wheel", dismiss, { passive: true });
    return () => {
      window.removeEventListener("click", dismiss);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("wheel", dismiss);
    };
  }, []);

  const handleContextMenu = useCallback((e: React.MouseEvent, tool: ToolWithMeta) => {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ tool, x: e.clientX, y: e.clientY });
  }, []);

  const ctxAction = useCallback(
    async (action: string) => {
      if (!ctxMenu) return;
      const tool = ctxMenu.tool;
      setCtxMenu(null);
      switch (action) {
        case "edit":
          editor.openEditor(tool);
          break;
        case "uninstall":
          install.handleUninstall(tool);
          break;
        case "install":
          install.enqueueInstall(tool);
          break;
        case "install-deps":
          install.handleInstallDeps(tool);
          break;
        case "copy-id":
          copyToClipboard(tool.id);
          break;
        case "open-dir":
          openToolDirectory({
            executable: tool.executable || tool.name,
            installMethod: tool.install?.method,
            installSource: tool.install?.source,
          }).catch((err) => data.setError(t("toolManager.openDirFailed", { error: err })));
          break;
        case "delete":
          install.setDeleteTarget(tool);
          break;
      }
    },
    [ctxMenu, editor, install, data, t]
  );

  const actionCtx: ActionButtonProps = {
    busy: install.busy,
    queuedIds: install.queuedIds,
    installProgress: install.installProgress,
    dlProgress: install.dlProgress,
    onCancel: install.handleCancelInstall,
    onUninstall: install.handleUninstall,
    onInstall: install.enqueueInstall,
    onDequeue: install.dequeueInstall,
    onFixPermission: install.handleFixExecutablePermission,
  };

  return (
    <div className="h-full flex flex-col">
      <ToolManagerHeader
        editingTool={editor.editingTool}
        editorVisible={editor.editorVisible}
        editorMode={editor.editorMode}
        editorDirty={editor.editorDirty}
        saving={editor.saving}
        closeEditor={editor.closeEditor}
        handleSwitchMode={editor.handleSwitchMode}
        handleSave={editor.handleSave}
        skillDirty={editor.skillDirty}
        skillSaving={skills.skillSaving}
        handleSaveSkill={skills.handleSaveSkill}
        toolsCount={data.tools.length}
        installedCount={data.installedCount}
        loading={data.loading}
        loadData={data.loadData}
        toolUpdates={install.toolUpdates}
        checkingUpdates={install.checkingUpdates}
        checkForUpdates={install.checkForUpdates}
        onImportGithub={github.openImportDialog}
        onAddTool={editor.handleAddTool}
      />

      {data.error && (
        <div className="mx-6 mt-3 text-[11px] text-destructive/80 bg-destructive/5 rounded-md px-3 py-2 flex items-center justify-between">
          <span>{data.error}</span>
          <button
            type="button"
            onClick={() => data.setError(null)}
            className="ml-2 text-destructive/50 hover:text-destructive"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      )}

      {editor.editingTool ? (
        <ToolManagerEditor
          editorVisible={editor.editorVisible}
          editorLoading={editor.editorLoading}
          editorMode={editor.editorMode}
          textareaRef={editor.textareaRef}
          rawJson={editor.rawJson}
          onRawChange={editor.handleRawChange}
          formData={editor.formData}
          onFormChange={editor.handleFormChange}
          onOutputChange={(output) => editor.handleFormChange("output", output)}
          categories={data.categories ?? []}
          skills={{
            skillsList: skills.skillsList,
            activeSkillId: skills.activeSkillId,
            skillContent: skills.skillContent,
            skillDirty: skills.skillDirty,
            skillSaving: skills.skillSaving,
            showNewSkill: skills.showNewSkill,
            setShowNewSkill: skills.setShowNewSkill,
            newSkillName: skills.newSkillName,
            setNewSkillName: skills.setNewSkillName,
            handleCreateSkill: skills.handleCreateSkill,
            loadSkillContent: skills.loadSkillContent,
            handleDeleteSkill: skills.handleDeleteSkill,
            handleSaveSkill: skills.handleSaveSkill,
            updateContent: skills.updateContent,
          }}
        />
      ) : (
        <>
          <ToolManagerToolbar
            search={data.search}
            setSearch={data.setSearch}
            selectedCategory={data.selectedCategory}
            setSelectedCategory={data.setSelectedCategory}
            selectedTier={data.selectedTier}
            setSelectedTier={data.setSelectedTier}
            sortKey={data.sortKey}
            setSortKey={data.setSortKey}
            allCategories={data.allCategories}
            categoryDisplayName={data.categoryDisplayName}
            viewMode={viewMode}
            setViewMode={setViewMode}
          />
          <ToolManagerList
            loading={data.loading}
            filteredTools={data.filteredTools}
            search={data.search}
            selectedTier={data.selectedTier}
            viewMode={viewMode}
            optionalCollapsed={optionalCollapsed}
            setOptionalCollapsed={setOptionalCollapsed}
            onOpenEditor={editor.openEditor}
            onContextMenu={handleContextMenu}
            onAddTool={editor.handleAddTool}
            actionCtx={actionCtx}
            installBusy={install.busy}
            batchInstalling={install.batchInstalling}
            onInstallAllRequired={install.handleInstallAllRequired}
            onCancelBatchInstall={install.cancelBatchInstall}
          />
        </>
      )}

      <ToolManagerDialogs
        ctxMenu={ctxMenu}
        ctxAction={ctxAction}
        uninstallTarget={install.uninstallTarget}
        setUninstallTarget={install.setUninstallTarget}
        confirmUninstall={install.confirmUninstall}
        depPicker={install.depPicker}
        setDepPicker={install.setDepPicker}
        doInstallDepFile={install.doInstallDepFile}
        execPicker={install.execPicker}
        setExecPicker={install.setExecPicker}
        deleteTarget={install.deleteTarget}
        setDeleteTarget={install.setDeleteTarget}
        handleDeleteTool={install.handleDeleteTool}
        showCloseConfirm={editor.showCloseConfirm}
        setShowCloseConfirm={editor.setShowCloseConfirm}
        forceCloseEditor={editor.forceCloseEditor}
        showUpdates={install.showUpdates}
        setShowUpdates={install.setShowUpdates}
        toolUpdates={install.toolUpdates}
        showGithubImport={github.showGithubImport}
        githubUrl={github.githubUrl}
        setGithubUrl={github.setGithubUrl}
        githubAnalyzing={github.githubAnalyzing}
        handleGithubImport={github.handleGithubImport}
        closeImportDialog={github.closeImportDialog}
      />
    </div>
  );
}
