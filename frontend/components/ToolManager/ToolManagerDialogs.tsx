/* ── Dialog stack for ToolManager ─────────────────────────────────────
 *
 * Conditionally renders the modal/popover dialogs and the right-click
 * context menu. Pulled out of `ToolManager.tsx` so the main file can stay
 * a thin shell. All dialog components themselves still live in `Dialogs.tsx`.
 */

import {
  CloseConfirmDialog,
  ContextMenu,
  type CtxMenuState,
  DeleteConfirmDialog,
  DepPickerDialog,
  ExecPickerDialog,
  type ExecPickerState,
  GitHubImportDialog,
  type ToolUpdateInfo,
  UninstallConfirmDialog,
  UpdatesDialog,
} from "./Dialogs";
import type { ToolWithMeta } from "./OutputParserEditor";

export interface ToolManagerDialogsProps {
  ctxMenu: CtxMenuState | null;
  ctxAction: (action: string) => void;

  uninstallTarget: ToolWithMeta | null;
  setUninstallTarget: (v: ToolWithMeta | null) => void;
  confirmUninstall: () => void;

  depPicker: { tool: ToolWithMeta; files: string[] } | null;
  setDepPicker: (v: { tool: ToolWithMeta; files: string[] } | null) => void;
  doInstallDepFile: (tool: ToolWithMeta, fileName: string) => void;

  execPicker: ExecPickerState | null;
  setExecPicker: (v: ExecPickerState | null) => void;

  deleteTarget: ToolWithMeta | null;
  setDeleteTarget: (v: ToolWithMeta | null) => void;
  handleDeleteTool: (tool: ToolWithMeta) => void;

  showCloseConfirm: boolean;
  setShowCloseConfirm: (v: boolean) => void;
  forceCloseEditor: () => void;

  showUpdates: boolean;
  setShowUpdates: (v: boolean) => void;
  toolUpdates: ToolUpdateInfo[];

  showGithubImport: boolean;
  githubUrl: string;
  setGithubUrl: (v: string) => void;
  githubAnalyzing: boolean;
  handleGithubImport: () => void;
  closeImportDialog: () => void;
}

/** Renders all conditional ToolManager dialogs. The component itself owns
 *  no state — it's a pure conditional aggregator. */
export function ToolManagerDialogs(props: ToolManagerDialogsProps) {
  return (
    <>
      {props.ctxMenu && <ContextMenu ctx={props.ctxMenu} onAction={props.ctxAction} />}
      {props.uninstallTarget && (
        <UninstallConfirmDialog
          target={props.uninstallTarget}
          onCancel={() => props.setUninstallTarget(null)}
          onConfirm={props.confirmUninstall}
        />
      )}
      {props.depPicker && (
        <DepPickerDialog
          tool={props.depPicker.tool}
          files={props.depPicker.files}
          onPick={props.doInstallDepFile}
          onCancel={() => props.setDepPicker(null)}
        />
      )}
      {props.execPicker && (
        <ExecPickerDialog state={props.execPicker} onDismiss={() => props.setExecPicker(null)} />
      )}
      {props.deleteTarget && (
        <DeleteConfirmDialog
          target={props.deleteTarget}
          onCancel={() => props.setDeleteTarget(null)}
          onConfirm={props.handleDeleteTool}
        />
      )}
      {props.showCloseConfirm && (
        <CloseConfirmDialog
          onCancel={() => props.setShowCloseConfirm(false)}
          onDiscard={props.forceCloseEditor}
        />
      )}
      {props.showUpdates && (
        <UpdatesDialog updates={props.toolUpdates} onClose={() => props.setShowUpdates(false)} />
      )}
      {props.showGithubImport && (
        <GitHubImportDialog
          url={props.githubUrl}
          onUrlChange={props.setGithubUrl}
          analyzing={props.githubAnalyzing}
          onImport={props.handleGithubImport}
          onCancel={props.closeImportDialog}
        />
      )}
    </>
  );
}
