import { useTranslation } from "react-i18next";
import { Switch } from "@/components/ui/switch";
import { useFileEditorSidebarStore } from "@/store/file-editor-sidebar";

export function EditorSettings() {
  const { t } = useTranslation();
  const {
    vimMode,
    setVimMode,
    wrap,
    setWrap,
    lineNumbers,
    setLineNumbers,
    relativeLineNumbers,
    setRelativeLineNumbers,
  } = useFileEditorSidebarStore();

  return (
    <div className="space-y-6">
      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">{t("editorSettings.general")}</h3>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <label
              htmlFor="editor-wrap"
              className="text-sm font-medium text-foreground cursor-pointer"
            >
              {t("editorSettings.wordWrap")}
            </label>
            <p className="text-xs text-muted-foreground">{t("editorSettings.wordWrapDesc")}</p>
          </div>
          <Switch id="editor-wrap" checked={wrap} onCheckedChange={setWrap} />
        </div>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <label
              htmlFor="editor-line-numbers"
              className="text-sm font-medium text-foreground cursor-pointer"
            >
              {t("editorSettings.lineNumbers")}
            </label>
            <p className="text-xs text-muted-foreground">{t("editorSettings.lineNumbersDesc")}</p>
          </div>
          <Switch id="editor-line-numbers" checked={lineNumbers} onCheckedChange={setLineNumbers} />
        </div>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <label
              htmlFor="editor-relative-line-numbers"
              className={`text-sm font-medium cursor-pointer ${!lineNumbers ? "text-muted-foreground" : "text-foreground"}`}
            >
              {t("editorSettings.relativeLineNumbers")}
            </label>
            <p className="text-xs text-muted-foreground">
              {t("editorSettings.relativeLineNumbersDesc")}
            </p>
          </div>
          <Switch
            id="editor-relative-line-numbers"
            checked={relativeLineNumbers}
            onCheckedChange={setRelativeLineNumbers}
            disabled={!lineNumbers}
          />
        </div>
      </div>

      <div className="border-t border-[var(--border-subtle)]" />

      <div className="space-y-4">
        <h3 className="text-sm font-medium text-foreground">{t("editorSettings.vimMode")}</h3>

        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <label
              htmlFor="editor-vim-mode"
              className="text-sm font-medium text-foreground cursor-pointer"
            >
              {t("editorSettings.enableVimMode")}
            </label>
            <p className="text-xs text-muted-foreground">{t("editorSettings.enableVimModeDesc")}</p>
          </div>
          <Switch id="editor-vim-mode" checked={vimMode} onCheckedChange={setVimMode} />
        </div>
      </div>
    </div>
  );
}
