/* ── ToolManager editor pane ──────────────────────────────────────────
 *
 * Hosts the four edit modes for a single tool:
 *   - "form"    — friendly field rows + JSON preview side-panel
 *   - "raw"     — raw JSON textarea
 *   - "skills"  — markdown editor for tool skills
 *   - "output"  — output-parser editor
 *
 * Pulled out of `ToolManager.tsx` to keep that file under the 800-line
 * file-size budget. Owns no state — the parent hook bundle is passed in
 * via props.
 */

import { BookOpen, Check, Code2, Loader2, Pencil, Plus, Save, Trash2, X } from "lucide-react";
import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import type { SkillFileInfo } from "@/lib/pentest/api";
import type { ToolCategory } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import {
  type EditorFieldsContext,
  FieldRow,
  InstallFieldRow,
  ParamsEditor,
  RUNTIME_VERSION_MAP,
} from "./EditorFields";
import { type OutputConfigData, OutputParserEditor } from "./OutputParserEditor";

export interface ToolManagerEditorProps {
  editorVisible: boolean;
  editorLoading: boolean;
  editorMode: "form" | "raw" | "skills" | "output";
  textareaRef: RefObject<HTMLTextAreaElement | null>;

  rawJson: string;
  onRawChange: (value: string) => void;

  formData: Record<string, unknown>;
  onFormChange: (field: string, value: unknown) => void;
  onOutputChange: (output: OutputConfigData) => void;

  categories: ToolCategory[];

  skills: {
    skillsList: SkillFileInfo[];
    activeSkillId: string | null;
    skillContent: string;
    skillDirty: boolean;
    skillSaving: boolean;
    showNewSkill: boolean;
    setShowNewSkill: (v: boolean) => void;
    newSkillName: string;
    setNewSkillName: (v: string) => void;
    handleCreateSkill: () => void;
    loadSkillContent: (skillId: string) => void;
    handleDeleteSkill: (skillId: string) => void;
    handleSaveSkill: () => void;
    updateContent: (val: string) => void;
  };
}

export function ToolManagerEditor(props: ToolManagerEditorProps) {
  const {
    editorVisible,
    editorLoading,
    editorMode,
    textareaRef,
    rawJson,
    onRawChange,
    formData,
    onFormChange,
    onOutputChange,
    categories,
    skills,
  } = props;

  const fieldCtx: EditorFieldsContext = { formData, handleFormChange: onFormChange };

  return (
    <div
      className={cn(
        "flex-1 overflow-y-auto px-6 py-4 transition-all duration-[180ms] ease-out",
        editorVisible ? "opacity-100 translate-x-0" : "opacity-0 translate-x-3"
      )}
    >
      {editorLoading ? (
        <div className="flex items-center justify-center h-32">
          <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
        </div>
      ) : editorMode === "raw" ? (
        <textarea
          ref={textareaRef}
          value={rawJson}
          onChange={(e) => onRawChange(e.target.value)}
          spellCheck={false}
          className="w-full h-full min-h-[400px] px-4 py-3 text-[11px] font-mono leading-[1.6] rounded-lg border border-border/10 bg-[var(--bg-hover)]/20 text-foreground outline-none focus:border-accent/30 transition-colors resize-none"
          style={{ tabSize: 2 }}
        />
      ) : editorMode === "skills" ? (
        <SkillsPane skills={skills} />
      ) : editorMode === "output" ? (
        <OutputParserEditor formData={formData} onChange={onOutputChange} />
      ) : (
        <FormPane fieldCtx={fieldCtx} formData={formData} categories={categories} />
      )}
    </div>
  );
}

/* ── Skills mode pane ─────────────────────────────────────────────── */

function SkillsPane({ skills }: { skills: ToolManagerEditorProps["skills"] }) {
  const { t } = useTranslation();
  return (
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
              placeholder={t("toolManager.newSkillName")}
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
                {t("toolManager.noSkills")}
              </span>
            </div>
          ) : (
            skills.skillsList.map((skill) => (
              <button
                type="button"
                key={skill.id}
                className={cn(
                  "group flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors w-full text-left",
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
              </button>
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
                {skills.skillDirty && <span className="w-1.5 h-1.5 rounded-full bg-accent/60" />}
              </div>
              <div className="flex items-center gap-1.5">
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
            </div>
            <div className="flex-1 min-h-0">
              <MarkdownEditor
                editorKey={skills.activeSkillId ?? ""}
                value={skills.skillContent}
                onChange={(val) => skills.updateContent(val)}
              />
            </div>
          </>
        ) : (
          <div className="flex-1 flex flex-col items-center justify-center gap-3">
            <BookOpen className="w-8 h-8 text-muted-foreground/30" />
            <p className="text-[12px] text-muted-foreground/50">{t("toolManager.selectSkill")}</p>
          </div>
        )}
      </div>
    </div>
  );
}

/* ── Form mode pane ───────────────────────────────────────────────── */

function FormPane({
  fieldCtx,
  formData,
  categories,
}: {
  fieldCtx: EditorFieldsContext;
  formData: Record<string, unknown>;
  categories: ToolCategory[];
}) {
  const { t } = useTranslation();
  const installMethod = (formData.install as Record<string, string>)?.method;
  const sourcePlaceholder =
    installMethod === "github"
      ? "owner/repo"
      : installMethod === "homebrew"
        ? "formula-name"
        : installMethod === "gem"
          ? "gem-name"
          : t("toolManager.source");

  const cat = categories.find((c) => c.id === (formData.category as string));
  const subcatOptions =
    cat && cat.items.length > 0
      ? cat.items.map((s) => ({ value: s.id, label: s.name }))
      : [{ value: "other", label: "other" }];

  const runtime = (formData.runtime as string) || "";
  const versionOptions = RUNTIME_VERSION_MAP[runtime];

  return (
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
          <FieldRow label={t("toolManager.icon")} field="icon" placeholder="📂" ctx={fieldCtx} />
          <div className="flex items-start gap-3 py-2 px-3 rounded-lg hover:bg-[var(--bg-hover)]/30 transition-colors">
            <span className="text-[12px] text-muted-foreground/60 w-24 flex-shrink-0 mt-1.5">
              {t("toolManager.description")}
            </span>
            <textarea
              value={(formData.description as string) ?? ""}
              onChange={(e) => fieldCtx.handleFormChange("description", e.target.value)}
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
          {runtime !== "native" &&
            (versionOptions ? (
              <FieldRow
                label={t("toolManager.runtimeVersion")}
                field="runtimeVersion"
                type="select"
                options={versionOptions}
                ctx={fieldCtx}
              />
            ) : (
              <FieldRow
                label={t("toolManager.runtimeVersion")}
                field="runtimeVersion"
                placeholder="version"
                ctx={fieldCtx}
              />
            ))}
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
              { value: "system", label: t("toolManager.system") },
              { value: "manual", label: t("toolManager.manual") },
            ]}
            ctx={fieldCtx}
          />
          <InstallFieldRow
            label={t("toolManager.source")}
            subField="source"
            placeholder={sourcePlaceholder}
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
              categories.length > 0
                ? categories.map((c) => ({ value: c.id, label: c.name }))
                : [{ value: "misc", label: "misc" }]
            }
            ctx={fieldCtx}
          />
          <FieldRow
            label={t("toolManager.subcategory")}
            field="subcategory"
            type="select"
            options={subcatOptions}
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
          {JSON.stringify({ tool: formData }, null, 2)}
        </pre>
      </div>
    </div>
  );
}
