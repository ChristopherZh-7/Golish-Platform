import { createPortal } from "react-dom";
import {
  ArrowUpCircle, Check, Copy, Download, ExternalLink,
  FolderOpen, FileText, Github, Loader2, Trash2, X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { ToolWithMeta } from "./OutputParserEditor";

/* ── Context Menu (portal) ── */

export interface CtxMenuState { tool: ToolWithMeta; x: number; y: number }

export function ContextMenu({ ctx, onAction }: {
  ctx: CtxMenuState;
  onAction: (action: string) => void;
}) {
  const { t } = useTranslation();
  return createPortal(
    <div className="fixed z-50 rounded-lg border border-border/20 bg-popover shadow-xl py-1 min-w-[140px]"
      style={{ left: ctx.x, top: ctx.y }}
      onClick={(e) => e.stopPropagation()}>
      <button type="button" onClick={() => onAction("edit")}
        className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
        <FileText className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.edit")}
      </button>
      {ctx.tool.installed ? (
        <button type="button" onClick={() => onAction("uninstall")}
          className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
          <Trash2 className="w-3 h-3 text-muted-foreground/50" /> {t("common.uninstall")}
        </button>
      ) : (
        <button type="button" onClick={() => onAction("install")}
          className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
          <Download className="w-3 h-3 text-muted-foreground/50" /> {t("common.install")}
        </button>
      )}
      <div className="my-1 border-t border-border/10" />
      <button type="button" onClick={() => onAction("copy-id")}
        className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
        <Copy className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.copyId")}
      </button>
      {ctx.tool.installed && (
        <button type="button" onClick={() => onAction("open-dir")}
          className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
          <FolderOpen className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.openDir")}
        </button>
      )}
      {ctx.tool.installed && ctx.tool.runtime === "python" && (
        <button type="button" onClick={() => onAction("install-deps")}
          className="w-full text-left px-3 py-1.5 text-[12px] text-foreground hover:bg-accent/10 transition-colors flex items-center gap-2">
          <Download className="w-3 h-3 text-muted-foreground/50" /> {t("toolManager.installDeps")}
        </button>
      )}
      <div className="my-1 border-t border-border/10" />
      <button type="button" onClick={() => onAction("delete")}
        className="w-full text-left px-3 py-1.5 text-[12px] text-red-400 hover:bg-red-500/10 transition-colors flex items-center gap-2">
        <Trash2 className="w-3 h-3" /> {t("toolManager.deleteConfig")}
      </button>
    </div>,
    document.body
  );
}

/* ── Uninstall Confirm ── */

export function UninstallConfirmDialog({ target, onCancel, onConfirm }: {
  target: ToolWithMeta; onCancel: () => void; onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onCancel}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
        <p className="text-[13px] text-foreground mb-1">{t("toolManager.uninstallConfirm", { name: target.name })}</p>
        <p className="text-[11px] text-muted-foreground/50 mb-4">{t("toolManager.uninstallKeepConfig")}</p>
        <div className="flex justify-end gap-2">
          <button type="button" onClick={onCancel}
            className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.cancel")}</button>
          <button type="button" onClick={onConfirm}
            className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">{t("toolManager.confirmUninstall")}</button>
        </div>
      </div>
    </div>
  );
}

/* ── Dep File Picker ── */

export function DepPickerDialog({ tool, files, onPick, onCancel }: {
  tool: ToolWithMeta; files: string[];
  onPick: (tool: ToolWithMeta, file: string) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onCancel}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-sm w-full" onClick={(e) => e.stopPropagation()}>
        <p className="text-[13px] text-foreground mb-1">{t("toolManager.selectDepFile")}</p>
        <p className="text-[11px] text-muted-foreground/50 mb-3">{t("toolManager.depFileHint", { name: tool.name })}</p>
        <div className="space-y-1 max-h-48 overflow-y-auto mb-4">
          {files.map((f) => (
            <button key={f} type="button" onClick={() => onPick(tool, f)}
              className="w-full text-left px-3 py-2 rounded-lg text-[12px] font-mono text-foreground hover:bg-accent/10 transition-colors">
              {f}
            </button>
          ))}
        </div>
        <div className="flex justify-end">
          <button type="button" onClick={onCancel}
            className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.cancel")}</button>
        </div>
      </div>
    </div>
  );
}

/* ── Exec Picker ── */

export interface ExecPickerState {
  tool: ToolWithMeta;
  dirName: string;
  candidates: string[];
  resolve: (v: string | null) => void;
}

export function ExecPickerDialog({ state, onDismiss }: {
  state: ExecPickerState;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={() => { state.resolve(null); onDismiss(); }}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-sm w-full" onClick={(e) => e.stopPropagation()}>
        <p className="text-[13px] text-foreground mb-1">{t("toolManager.selectExecutable")}</p>
        <p className="text-[11px] text-muted-foreground/50 mb-3">
          {t("toolManager.multipleExecsDetected", { name: state.tool.name })}
        </p>
        <div className="space-y-1 max-h-48 overflow-y-auto mb-4">
          {state.candidates.map((f, i) => (
            <button key={f} type="button"
              onClick={() => { state.resolve(f); onDismiss(); }}
              className={cn(
                "w-full text-left px-3 py-2 rounded-lg text-[12px] font-mono transition-colors",
                i === 0
                  ? "bg-accent/15 text-accent hover:bg-accent/20 font-semibold"
                  : "text-foreground hover:bg-accent/10",
              )}>
              {f}{i === 0 && <span className="ml-2 text-[10px] text-accent/70 font-normal">{t("common.recommended")}</span>}
            </button>
          ))}
        </div>
        <div className="flex justify-end">
          <button type="button" onClick={() => { state.resolve(null); onDismiss(); }}
            className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.skip")}</button>
        </div>
      </div>
    </div>
  );
}

/* ── Delete Config Confirm ── */

export function DeleteConfirmDialog({ target, onCancel, onConfirm }: {
  target: ToolWithMeta; onCancel: () => void; onConfirm: (tool: ToolWithMeta) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onCancel}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
        <p className="text-[13px] text-foreground mb-1">{t("toolManager.deleteConfirmTitle", { name: target.name })}</p>
        <p className="text-[11px] text-muted-foreground/50 mb-4">
          {t("toolManager.deleteConfirmMsg")}
          {target.installed && t("toolManager.deleteKeepFiles")}
        </p>
        <div className="flex justify-end gap-2">
          <button type="button" onClick={onCancel}
            className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("common.cancel")}</button>
          <button type="button" onClick={() => onConfirm(target)}
            className="text-[12px] px-3 py-1.5 rounded-lg bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors">{t("toolManager.confirmDelete")}</button>
        </div>
      </div>
    </div>
  );
}

/* ── Close Confirm ── */

export function CloseConfirmDialog({ onCancel, onDiscard }: {
  onCancel: () => void; onDiscard: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onCancel}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
        <p className="text-[13px] text-foreground mb-1">{t("toolManager.unsavedChanges")}</p>
        <p className="text-[11px] text-muted-foreground/50 mb-4">{t("toolManager.unsavedChangesMsg")}</p>
        <div className="flex justify-end gap-2">
          <button type="button" onClick={onCancel}
            className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">{t("toolManager.continueEditing")}</button>
          <button type="button" onClick={onDiscard}
            className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">{t("toolManager.discardChanges")}</button>
        </div>
      </div>
    </div>
  );
}

/* ── Tool Updates ── */

export interface ToolUpdateInfo {
  tool_id: string; tool_name: string;
  current_version: string; latest_version: string;
  has_update: boolean; release_url: string;
}

export function UpdatesDialog({ updates, onClose }: {
  updates: ToolUpdateInfo[]; onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onClose}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-md w-full" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 mb-3">
          <ArrowUpCircle className="w-4 h-4 text-accent" />
          <h2 className="text-[14px] font-semibold flex-1">Tool Updates</h2>
          <button onClick={onClose} className="p-0.5 rounded hover:bg-muted/50">
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
        {updates.length === 0 ? (
          <p className="text-[11px] text-muted-foreground/50 py-4 text-center">No tools with GitHub sources found.</p>
        ) : (
          <div className="space-y-1.5 max-h-[400px] overflow-y-auto">
            {updates.map((u) => (
              <div key={u.tool_id} className={cn(
                "flex items-center gap-2 px-3 py-2 rounded-lg text-[11px]",
                u.has_update ? "bg-amber-500/5 border border-amber-500/20" : "bg-muted/10 border border-border/10",
              )}>
                <span className="flex-1 font-medium truncate">{u.tool_name}</span>
                <span className="text-muted-foreground/50 font-mono">{u.current_version || "?"}</span>
                {u.has_update && (
                  <>
                    <span className="text-muted-foreground/50">→</span>
                    <span className="text-amber-400 font-mono font-medium">{u.latest_version}</span>
                    {u.release_url && (
                      <a href={u.release_url} target="_blank" rel="noopener noreferrer"
                        className="p-0.5 text-accent/50 hover:text-accent transition-colors">
                        <ExternalLink className="w-3 h-3" />
                      </a>
                    )}
                  </>
                )}
                {!u.has_update && (
                  <span className="text-green-400/60 flex items-center gap-1">
                    <Check className="w-3 h-3" /> latest
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/* ── GitHub Import ── */

export function GitHubImportDialog({ url, onUrlChange, analyzing, onImport, onCancel }: {
  url: string; onUrlChange: (url: string) => void;
  analyzing: boolean; onImport: () => void; onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onCancel}>
      <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-6 shadow-xl max-w-md w-full" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 mb-4">
          <Github className="w-5 h-5 text-accent" />
          <h2 className="text-[15px] font-semibold text-foreground">{t("toolManager.importGithub")}</h2>
        </div>
        <p className="text-[11px] text-muted-foreground/50 mb-3">{t("toolManager.importGithubHint")}</p>
        <input
          value={url}
          onChange={(e) => onUrlChange(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") onImport(); }}
          placeholder="owner/repo or https://github.com/owner/repo"
          autoFocus
          className="w-full h-9 px-3 text-[12px] font-mono bg-background rounded-lg border border-border/20 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
        />
        <div className="flex justify-end gap-2 mt-4">
          <button type="button" onClick={onCancel}
            className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">
            {t("common.cancel")}
          </button>
          <button type="button" onClick={onImport} disabled={analyzing || !url.trim()}
            className={cn("flex items-center gap-1.5 text-[12px] px-4 py-1.5 rounded-lg font-medium transition-colors",
              url.trim()
                ? "bg-accent text-accent-foreground hover:bg-accent/90"
                : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
            {analyzing && <Loader2 className="w-3 h-3 animate-spin" />}
            {t("toolManager.analyzeImport")}
          </button>
        </div>
      </div>
    </div>
  );
}
