import {
  ArrowLeft,
  ArrowUpCircle,
  ArrowUpDown,
  BookOpen,
  Check,
  ChevronDown,
  Code2,
  Download,
  FileText,
  FolderOpen,
  Github,
  Grid3X3,
  List,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { copyToClipboard } from "@/lib/clipboard";
import { openToolDirectory } from "@/lib/pentest/api";
import { cn } from "@/lib/utils";
import {
  CloseConfirmDialog,
  ContextMenu,
  type CtxMenuState,
  DeleteConfirmDialog,
  DepPickerDialog,
  ExecPickerDialog,
  GitHubImportDialog,
  UninstallConfirmDialog,
  UpdatesDialog,
} from "./Dialogs";
import {
  type EditorFieldsContext,
  FieldRow,
  InstallFieldRow,
  ParamsEditor,
  RUNTIME_VERSION_MAP,
} from "./EditorFields";
import { useToolData } from "./hooks/useToolData";
import { useToolEditor } from "./hooks/useToolEditor";
import { isAutoInstallMethod, useToolInstall } from "./hooks/useToolInstall";
import { useSkillEditor } from "./hooks/useSkillEditor";
import { useGithubImport } from "./hooks/useGithubImport";
import { OutputParserEditor, type ToolWithMeta, type ViewMode } from "./OutputParserEditor";
import { type ActionButtonProps, GridCard, ListRow } from "./ToolCards";

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


  // ── Context menu ──

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
          install.handleInstall(tool);
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

  const fieldCtx: EditorFieldsContext = {
    formData: editor.formData,
    handleFormChange: editor.handleFormChange,
  };
  const actionCtx: ActionButtonProps = {
    busy: install.busy,
    installProgress: install.installProgress,
    dlProgress: install.dlProgress,
    onCancel: install.handleCancelInstall,
    onUninstall: install.handleUninstall,
    onInstall: install.handleInstall,
    onFixPermission: install.handleFixExecutablePermission,
  };

  const renderToolList = (items: ToolWithMeta[]) =>
    viewMode === "grid" ? (
      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {items.map((tool) => (
          <GridCard
            key={tool.id}
            tool={tool}
            onOpen={editor.openEditor}
            onContextMenu={handleContextMenu}
            actionCtx={actionCtx}
          />
        ))}
      </div>
    ) : (
      <div className="space-y-1">
        {items.map((tool) => (
          <ListRow
            key={tool.id}
            tool={tool}
            onOpen={editor.openEditor}
            onContextMenu={handleContextMenu}
            actionCtx={actionCtx}
          />
        ))}
      </div>
    );

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-border/15 flex-shrink-0">
        {editor.editingTool ? (
          <>
            <div
              className={cn(
                "flex items-center gap-3 transition-all duration-[180ms] ease-out",
                editor.editorVisible ? "opacity-100 translate-x-0" : "opacity-0 translate-x-2"
              )}
            >
              <button
                type="button"
                onClick={editor.closeEditor}
                className="p-1.5 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
              >
                <ArrowLeft className="w-4 h-4" />
              </button>
              <div>
                <div className="flex items-center gap-2">
                  {editor.editingTool.icon && (
                    <span className="text-[14px]">{editor.editingTool.icon}</span>
                  )}
                  <h1 className="text-[16px] font-semibold text-foreground">
                    {editor.editingTool.name}
                  </h1>
                  {(editor.editorDirty || editor.skillDirty) && (
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
                    onClick={() => editor.handleSwitchMode(mode)}
                    className={cn(
                      "flex items-center gap-1.5 px-3 py-1.5 text-[11px] transition-colors",
                      editor.editorMode === mode
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
              {editor.editorMode === "skills" ? (
                <button
                  type="button"
                  onClick={skills.handleSaveSkill}
                  disabled={skills.skillSaving || !editor.skillDirty}
                  className={cn(
                    "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                    editor.skillDirty
                      ? "bg-accent text-accent-foreground hover:bg-accent/90"
                      : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed"
                  )}
                >
                  {skills.skillSaving ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : (
                    <Save className="w-3 h-3" />
                  )}{" "}
                  {t("common.save")}
                </button>
              ) : (
                <button
                  type="button"
                  onClick={editor.handleSave}
                  disabled={editor.saving || !editor.editorDirty}
                  className={cn(
                    "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors",
                    editor.editorDirty
                      ? "bg-accent text-accent-foreground hover:bg-accent/90"
                      : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed"
                  )}
                >
                  {editor.saving ? (
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
              <h1 className="text-[16px] font-semibold text-foreground">
                {t("toolManager.title")}
              </h1>
              <p className="text-[11px] text-muted-foreground/50 mt-0.5">
                {t("toolManager.toolCount", {
                  count: data.tools.length,
                  installed: data.installedCount,
                })}
              </p>
            </div>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={github.openImportDialog}
                title={t("toolManager.importGithub")}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
              >
                <Github className="w-3.5 h-3.5" /> {t("toolManager.importGithub")}
              </button>
              <button
                type="button"
                onClick={editor.handleAddTool}
                title={t("toolManager.addTool")}
                className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors"
              >
                <Plus className="w-4 h-4" />
              </button>
              <button
                type="button"
                onClick={install.checkForUpdates}
                disabled={install.checkingUpdates}
                title="Check for Updates"
                className={cn(
                  "p-2 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-30",
                  install.toolUpdates.some((u) => u.has_update) && "text-amber-400"
                )}
              >
                <ArrowUpCircle
                  className={cn("w-4 h-4", install.checkingUpdates && "animate-spin")}
                />
              </button>
              <button
                type="button"
                onClick={() => data.loadData()}
                disabled={data.loading}
                title={t("common.refresh")}
                className="p-2 rounded-lg text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-30"
              >
                <RefreshCw className={cn("w-4 h-4", data.loading && "animate-spin")} />
              </button>
            </div>
          </>
        )}
      </div>

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

      {/* Editor view */}
      {editor.editingTool ? (
        <div
          className={cn(
            "flex-1 overflow-y-auto px-6 py-4 transition-all duration-[180ms] ease-out",
            editor.editorVisible ? "opacity-100 translate-x-0" : "opacity-0 translate-x-3"
          )}
        >
          {editor.editorLoading ? (
            <div className="flex items-center justify-center h-32">
              <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
            </div>
          ) : editor.editorMode === "raw" ? (
            <textarea
              ref={editor.textareaRef}
              value={editor.rawJson}
              onChange={(e) => editor.handleRawChange(e.target.value)}
              spellCheck={false}
              className="w-full h-full min-h-[400px] px-4 py-3 text-[11px] font-mono leading-[1.6] rounded-lg border border-border/10 bg-[var(--bg-hover)]/20 text-foreground outline-none focus:border-accent/30 transition-colors resize-none"
              style={{ tabSize: 2 }}
            />
          ) : editor.editorMode === "skills" ? (
            <div className="flex gap-4 h-full min-h-[400px]">
              <div className="w-[220px] flex-shrink-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
                  <span className="text-[11px] font-medium text-muted-foreground/60">Skills</span>
                  <button
                    type="button"
                    onClick={() => skills.setShowNewSkill(true)}
                    className="p-1 rounded text-muted-foreground/40 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors"
                  >
                    <Plus className="w-3 h-3" />
                  </button>
                </div>
                {skills.showNewSkill && (
                  <div className="px-2 py-2 border-b border-border/8 flex gap-1.5">
                    <input
                      value={skills.newSkillName}
                      onChange={(e) => skills.setNewSkillName(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") skills.handleCreateSkill();
                        if (e.key === "Escape") {
                          skills.setShowNewSkill(false);
                          skills.setNewSkillName("");
                        }
                      }}
                      placeholder={t("toolManager.newSkillName", "Skill name...")}
                      autoFocus
                      className="flex-1 px-2 py-1 text-[11px] rounded bg-background border border-border/20 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
                    />
                    <button
                      type="button"
                      onClick={skills.handleCreateSkill}
                      disabled={!skills.newSkillName.trim()}
                      className="p-1 rounded text-accent hover:bg-accent/10 disabled:opacity-30 transition-colors"
                    >
                      <Check className="w-3 h-3" />
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        skills.setShowNewSkill(false);
                        skills.setNewSkillName("");
                      }}
                      className="p-1 rounded text-muted-foreground/40 hover:text-foreground transition-colors"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                )}
                <div className="flex-1 overflow-y-auto">
                  {skills.skillsList.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-24 gap-2">
                      <BookOpen className="w-4 h-4 text-muted-foreground/40" />
                      <span className="text-[11px] text-muted-foreground/50">
                        {t("toolManager.noSkills", "No skills yet")}
                      </span>
                    </div>
                  ) : (
                    skills.skillsList.map((skill) => (
                      <div
                        key={skill.id}
                        className={cn(
                          "group flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors",
                          skills.activeSkillId === skill.id
                            ? "bg-accent/10 text-accent"
                            : "text-foreground/70 hover:bg-[var(--bg-hover)]"
                        )}
                        onClick={() => skills.loadSkillContent(skill.id)}
                      >
                        <BookOpen className="w-3 h-3 flex-shrink-0 opacity-40" />
                        <span className="flex-1 text-[12px] truncate">{skill.name}</span>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            skills.handleDeleteSkill(skill.id);
                          }}
                          className="p-0.5 rounded opacity-0 group-hover:opacity-40 hover:!opacity-100 hover:text-destructive transition-all"
                        >
                          <Trash2 className="w-2.5 h-2.5" />
                        </button>
                      </div>
                    ))
                  )}
                </div>
              </div>
              <div className="flex-1 min-w-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                {skills.activeSkillId ? (
                  <>
                    <div className="px-3 py-2 border-b border-border/8 flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        <Pencil className="w-3 h-3 text-muted-foreground/50" />
                        <span className="text-[11px] font-medium text-muted-foreground/60">
                          {skills.activeSkillId}.md
                        </span>
                        {skills.skillDirty && (
                          <span className="w-1.5 h-1.5 rounded-full bg-accent/60" />
                        )}
                      </div>
                      <button
                        type="button"
                        onClick={skills.handleSaveSkill}
                        disabled={!skills.skillDirty || skills.skillSaving}
                        className={cn(
                          "flex items-center gap-1 px-2 py-1 rounded text-[10px] font-medium transition-colors",
                          skills.skillDirty
                            ? "bg-accent text-accent-foreground hover:bg-accent/90"
                            : "text-muted-foreground/30 cursor-not-allowed"
                        )}
                      >
                        {skills.skillSaving ? (
                          <Loader2 className="w-2.5 h-2.5 animate-spin" />
                        ) : (
                          <Save className="w-2.5 h-2.5" />
                        )}
                        {t("common.save")}
                      </button>
                    </div>
                    <textarea
                      value={skills.skillContent}
                      onChange={(e) => skills.updateContent(e.target.value)}
                      spellCheck={false}
                      className="flex-1 w-full px-4 py-3 text-[12px] font-mono leading-[1.7] bg-transparent text-foreground outline-none resize-none"
                      style={{ tabSize: 2 }}
                    />
                  </>
                ) : (
                  <div className="flex-1 flex flex-col items-center justify-center gap-3">
                    <BookOpen className="w-8 h-8 text-muted-foreground/30" />
                    <p className="text-[12px] text-muted-foreground/50">
                      {t("toolManager.selectSkill", "Select a skill to edit or create a new one")}
                    </p>
                  </div>
                )}
              </div>
            </div>
          ) : editor.editorMode === "output" ? (
            <OutputParserEditor
              formData={editor.formData}
              onChange={(output) => {
                editor.handleFormChange("output", output);
              }}
            />
          ) : (
            <div className="flex gap-4 h-full">
              <div className="flex-1 min-w-0 space-y-4 overflow-y-auto">
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8">
                    <span className="text-[11px] font-medium text-muted-foreground/60">
                      {t("toolManager.basicInfo")}
                    </span>
                  </div>
                  <FieldRow
                    label={t("toolManager.name")}
                    field="name"
                    placeholder="dirsearch"
                    ctx={fieldCtx}
                  />
                  <FieldRow
                    label={t("toolManager.icon")}
                    field="icon"
                    placeholder="📂"
                    ctx={fieldCtx}
                  />
                  <div className="flex items-start gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
                    <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0 mt-1.5">
                      {t("toolManager.description")}
                    </span>
                    <textarea
                      value={(editor.formData.description as string) ?? ""}
                      onChange={(e) => editor.handleFormChange("description", e.target.value)}
                      placeholder={t("toolManager.descriptionPlaceholder")}
                      rows={2}
                      className="flex-1 px-2 py-1.5 text-[12px] rounded-md bg-transparent border border-transparent hover:border-border/20 focus:border-accent/40 text-foreground placeholder:text-muted-foreground/20 outline-none transition-colors resize-y"
                    />
                  </div>
                  <FieldRow
                    label={t("common.version")}
                    field="version"
                    placeholder="1.0.0"
                    ctx={fieldCtx}
                  />
                  <FieldRow label="ID" field="id" mono placeholder="hash" ctx={fieldCtx} />
                  <FieldRow
                    label={t("toolManager.executable")}
                    field="executable"
                    mono
                    placeholder="tool/main.py"
                    ctx={fieldCtx}
                  />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8">
                    <span className="text-[11px] font-medium text-muted-foreground/60">
                      {t("toolManager.runtime")}
                    </span>
                  </div>
                  <FieldRow
                    label={t("toolManager.runtimeLabel")}
                    field="runtime"
                    type="select"
                    options={[
                      { value: "native", label: "Native" },
                      { value: "python", label: "Python" },
                      { value: "java", label: "Java" },
                      { value: "node", label: "Node.js" },
                      { value: "ruby", label: "Ruby" },
                    ]}
                    ctx={fieldCtx}
                  />
                  {editor.formData.runtime !== "native" &&
                    (() => {
                      const rt = (editor.formData.runtime as string) || "";
                      const versionOptions = RUNTIME_VERSION_MAP[rt];
                      if (versionOptions) {
                        return (
                          <FieldRow
                            label={t("toolManager.runtimeVersion")}
                            field="runtimeVersion"
                            type="select"
                            options={versionOptions}
                            ctx={fieldCtx}
                          />
                        );
                      }
                      return (
                        <FieldRow
                          label={t("toolManager.runtimeVersion")}
                          field="runtimeVersion"
                          placeholder="version"
                          ctx={fieldCtx}
                        />
                      );
                    })()}
                  <FieldRow
                    label={t("toolManager.launchModeLabel")}
                    field="launchMode"
                    type="select"
                    options={[
                      { value: "cli", label: "CLI" },
                      { value: "gui", label: "GUI" },
                      { value: "web", label: "Web" },
                    ]}
                    ctx={fieldCtx}
                  />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8">
                    <span className="text-[11px] font-medium text-muted-foreground/60">
                      {t("toolManager.installMethod")}
                    </span>
                  </div>
                  <InstallFieldRow
                    label={t("toolManager.installMethodLabel")}
                    subField="method"
                    type="select"
                    options={[
                      { value: "", label: t("common.none") },
                      { value: "github", label: "GitHub" },
                      { value: "homebrew", label: "Homebrew" },
                      { value: "homebrew-cask", label: "Homebrew Cask" },
                      { value: "pip", label: "pip" },
                      { value: "gem", label: "RubyGem" },
                      { value: "system", label: t("toolManager.system", "System") },
                      { value: "manual", label: t("toolManager.manual") },
                    ]}
                    ctx={fieldCtx}
                  />
                  <InstallFieldRow
                    label={t("toolManager.source")}
                    subField="source"
                    placeholder={
                      (editor.formData.install as Record<string, string>)?.method === "github"
                        ? "owner/repo"
                        : (editor.formData.install as Record<string, string>)?.method === "homebrew"
                          ? "formula-name"
                          : (editor.formData.install as Record<string, string>)?.method === "gem"
                            ? "gem-name"
                            : t("toolManager.source")
                    }
                    mono
                    ctx={fieldCtx}
                  />
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8">
                    <span className="text-[11px] font-medium text-muted-foreground/60">
                      {t("toolManager.paramConfig")}
                    </span>
                  </div>
                  <div className="py-2">
                    <ParamsEditor ctx={fieldCtx} />
                  </div>
                </div>
                <div className="rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden">
                  <div className="px-3 py-2 border-b border-border/8">
                    <span className="text-[11px] font-medium text-muted-foreground/60">
                      {t("toolManager.category")}
                    </span>
                  </div>
                  <FieldRow
                    label={t("toolManager.category")}
                    field="category"
                    type="select"
                    options={
                      (data.categories ?? []).length > 0
                        ? (data.categories ?? []).map((c) => ({ value: c.id, label: c.name }))
                        : [{ value: "misc", label: "misc" }]
                    }
                    ctx={fieldCtx}
                  />
                  <FieldRow
                    label={t("toolManager.subcategory")}
                    field="subcategory"
                    type="select"
                    options={(() => {
                      const cat = (data.categories ?? []).find(
                        (c) => c.id === (editor.formData.category as string)
                      );
                      if (cat && cat.items.length > 0)
                        return cat.items.map((s) => ({ value: s.id, label: s.name }));
                      return [{ value: "other", label: "other" }];
                    })()}
                    ctx={fieldCtx}
                  />
                </div>
              </div>
              <div className="w-[380px] flex-shrink-0 rounded-xl bg-[var(--bg-hover)]/20 overflow-hidden flex flex-col">
                <div className="px-3 py-2 border-b border-border/8 flex items-center gap-2">
                  <Code2 className="w-3 h-3 text-muted-foreground/30" />
                  <span className="text-[11px] font-medium text-muted-foreground/60">
                    {t("toolManager.jsonPreview")}
                  </span>
                </div>
                <pre className="flex-1 overflow-auto px-4 py-3 text-[10px] font-mono leading-[1.6] text-muted-foreground/60 select-all whitespace-pre">
                  {JSON.stringify({ tool: editor.formData }, null, 2)}
                </pre>
              </div>
            </div>
          )}
        </div>
      ) : (
        <>
          {/* Search + filters toolbar */}
          <div className="px-6 py-3 flex items-center gap-3 border-b border-border/10 flex-shrink-0">
            <div className="relative flex-1 max-w-sm">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
              <input
                value={data.search}
                onChange={(e) => data.setSearch(e.target.value)}
                placeholder={t("toolManager.searchPlaceholder")}
                className="w-full h-8 pl-8 pr-3 text-[12px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
              />
            </div>

            <Select
              value={data.selectedCategory ?? "_all"}
              onValueChange={(v) => data.setSelectedCategory(v === "_all" ? null : v)}
            >
              <SelectTrigger
                size="sm"
                className="h-7 w-auto min-w-[120px] border-border/15 bg-[var(--bg-hover)]/30 text-[11px] shadow-none px-2.5 gap-1.5"
              >
                <FolderOpen className="w-3 h-3 text-muted-foreground/40" />
                <SelectValue placeholder={t("common.all")} />
              </SelectTrigger>
              <SelectContent position="popper" className="min-w-[140px]">
                <SelectItem value="_all" className="text-[12px]">
                  {t("common.all")}
                </SelectItem>
                {data.allCategories.map((catId) => (
                  <SelectItem key={catId} value={catId} className="text-[12px]">
                    {data.categoryDisplayName(catId)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Select
              value={data.selectedTier ?? "_all"}
              onValueChange={(v) => data.setSelectedTier(v === "_all" ? null : v)}
            >
              <SelectTrigger
                size="sm"
                className="h-7 w-auto min-w-[110px] border-border/15 bg-[var(--bg-hover)]/30 text-[11px] shadow-none px-2.5 gap-1.5"
              >
                <SelectValue placeholder={t("common.all")} />
              </SelectTrigger>
              <SelectContent position="popper" className="min-w-[130px]">
                <SelectItem value="_all" className="text-[12px]">
                  {t("common.all")}
                </SelectItem>
                <SelectItem value="essential" className="text-[12px] text-red-400">
                  {t("toolManager.tierEssential")}
                </SelectItem>
                <SelectItem value="recommended" className="text-[12px] text-amber-400">
                  {t("toolManager.tierRecommended")}
                </SelectItem>
                <SelectItem value="optional" className="text-[12px]">
                  {t("toolManager.tierOptional")}
                </SelectItem>
              </SelectContent>
            </Select>

            <div className="ml-auto flex items-center gap-1">
              <Select value={data.sortKey} onValueChange={(v) => data.setSortKey(v as any)}>
                <SelectTrigger
                  size="sm"
                  className="h-7 w-auto border-transparent bg-transparent hover:bg-[var(--bg-hover)] text-[11px] shadow-none px-2 gap-1 text-muted-foreground/50"
                >
                  <ArrowUpDown className="w-3 h-3" />
                  <SelectValue />
                </SelectTrigger>
                <SelectContent position="popper" className="min-w-[100px]">
                  <SelectItem value="name" className="text-[12px]">
                    {t("toolManager.sortByName")}
                  </SelectItem>
                  <SelectItem value="status" className="text-[12px]">
                    {t("toolManager.sortByStatus")}
                  </SelectItem>
                  <SelectItem value="category" className="text-[12px]">
                    {t("toolManager.sortByCategory")}
                  </SelectItem>
                  <SelectItem value="runtime" className="text-[12px]">
                    {t("toolManager.sortByRuntime")}
                  </SelectItem>
                </SelectContent>
              </Select>

              <div className="flex items-center rounded-md border border-border/10 overflow-hidden">
                <button
                  type="button"
                  onClick={() => setViewMode("grid")}
                  title={t("toolManager.gridView")}
                  className={cn(
                    "p-1.5 transition-colors",
                    viewMode === "grid"
                      ? "bg-accent/15 text-accent"
                      : "text-muted-foreground/50 hover:text-foreground"
                  )}
                >
                  <Grid3X3 className="w-3.5 h-3.5" />
                </button>
                <button
                  type="button"
                  onClick={() => setViewMode("list")}
                  title={t("toolManager.listView")}
                  className={cn(
                    "p-1.5 transition-colors",
                    viewMode === "list"
                      ? "bg-accent/15 text-accent"
                      : "text-muted-foreground/50 hover:text-foreground"
                  )}
                >
                  <List className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
          </div>

          {/* Tool grid/list */}
          <div className="flex-1 overflow-y-auto px-6 py-4">
            {data.loading ? (
              <div key="tm-loading" className="flex items-center justify-center h-32">
                <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/50" />
              </div>
            ) : data.filteredTools.length === 0 ? (
              <div
                key="tm-empty"
                className="flex flex-col items-center justify-center h-32 gap-2 overflow-hidden"
              >
                <span className="text-[12px] text-muted-foreground/60">
                  {data.search.trim() ? t("toolManager.noMatch") : t("toolManager.noTools")}
                </span>
                {!data.search.trim() && (
                  <button
                    type="button"
                    onClick={editor.handleAddTool}
                    className="text-[11px] text-accent/60 hover:text-accent transition-colors flex items-center gap-1"
                  >
                    <Plus className="w-3 h-3" /> {t("toolManager.addFirstTool")}
                  </button>
                )}
              </div>
            ) : (
              (() => {
                const shouldGroup = !data.selectedTier && !data.search.trim();
                const requiredTools = data.filteredTools.filter(
                  (t) => t.tier === "essential" || t.tier === "recommended"
                );
                const optionalTools = data.filteredTools.filter(
                  (t) => !t.tier || t.tier === "optional"
                );

                if (!shouldGroup) return renderToolList(data.filteredTools);

                return (
                  <div className="space-y-6">
                    {requiredTools.length > 0 &&
                      (() => {
                        const uninstalledRequired = requiredTools.filter(
                          (t) => !t.installed && isAutoInstallMethod(t.install?.method)
                        );
                        return (
                          <div>
                            <div className="flex items-center gap-2 mb-3">
                              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-red-500/10 border border-red-500/20">
                                <div className="w-1.5 h-1.5 rounded-full bg-red-500" />
                                <span className="text-[11px] font-medium text-red-400">
                                  {t("toolManager.requiredSection", "Required")}
                                </span>
                                <span className="text-[10px] text-red-400/50">
                                  {requiredTools.length}
                                </span>
                              </div>
                              <span className="text-[10px] text-muted-foreground/60">
                                {t(
                                  "toolManager.requiredHint",
                                  "Core tools needed for full functionality"
                                )}
                              </span>
                              {uninstalledRequired.length > 0 && (
                                <div className="ml-auto flex items-center gap-1.5">
                                  {install.batchInstalling ? (
                                    <button
                                      type="button"
                                      onClick={install.cancelBatchInstall}
                                      className="flex items-center gap-1.5 px-3 py-1 rounded-md text-[10px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors"
                                    >
                                      <X className="w-3 h-3" />
                                      {t("common.cancel")}
                                    </button>
                                  ) : (
                                    <button
                                      type="button"
                                      onClick={() =>
                                        install.handleInstallAllRequired(data.filteredTools)
                                      }
                                      disabled={!!install.busy}
                                      className="flex items-center gap-1.5 px-3 py-1 rounded-md text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                                    >
                                      <Download className="w-3 h-3" />
                                      {t("toolManager.installAllRequired", "Install All")}
                                      <span className="text-[9px] px-1.5 py-px rounded-full bg-accent/15">
                                        {uninstalledRequired.length}
                                      </span>
                                    </button>
                                  )}
                                </div>
                              )}
                            </div>
                            {renderToolList(requiredTools)}
                          </div>
                        );
                      })()}
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
                              {t("toolManager.optionalSection", "Optional")}
                            </span>
                            <span className="text-[10px] text-muted-foreground/50">
                              {optionalTools.length}
                            </span>
                          </div>
                          <span className="text-[10px] text-muted-foreground/60">
                            {t("toolManager.optionalHint", "Install as needed for specific tasks")}
                          </span>
                        </button>
                        {!optionalCollapsed && renderToolList(optionalTools)}
                      </div>
                    )}
                  </div>
                );
              })()
            )}
          </div>
        </>
      )}

      {/* Dialogs */}
      {ctxMenu && <ContextMenu ctx={ctxMenu} onAction={ctxAction} />}
      {install.uninstallTarget && (
        <UninstallConfirmDialog
          target={install.uninstallTarget}
          onCancel={() => install.setUninstallTarget(null)}
          onConfirm={install.confirmUninstall}
        />
      )}
      {install.depPicker && (
        <DepPickerDialog
          tool={install.depPicker.tool}
          files={install.depPicker.files}
          onPick={install.doInstallDepFile}
          onCancel={() => install.setDepPicker(null)}
        />
      )}
      {install.execPicker && (
        <ExecPickerDialog
          state={install.execPicker}
          onDismiss={() => install.setExecPicker(null)}
        />
      )}
      {install.deleteTarget && (
        <DeleteConfirmDialog
          target={install.deleteTarget}
          onCancel={() => install.setDeleteTarget(null)}
          onConfirm={install.handleDeleteTool}
        />
      )}
      {editor.showCloseConfirm && (
        <CloseConfirmDialog
          onCancel={() => editor.setShowCloseConfirm(false)}
          onDiscard={editor.forceCloseEditor}
        />
      )}
      {install.showUpdates && (
        <UpdatesDialog
          updates={install.toolUpdates}
          onClose={() => install.setShowUpdates(false)}
        />
      )}
      {github.showGithubImport && (
        <GitHubImportDialog
          url={github.githubUrl}
          onUrlChange={github.setGithubUrl}
          analyzing={github.githubAnalyzing}
          onImport={github.handleGithubImport}
          onCancel={github.closeImportDialog}
        />
      )}
    </div>
  );
}
