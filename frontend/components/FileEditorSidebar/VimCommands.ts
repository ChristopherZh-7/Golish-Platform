import { Vim } from "@replit/codemirror-vim";
import { useFileEditorSidebarStore } from "@/store/file-editor-sidebar";

let vimSaveCallback: (() => void) | null = null;
let vimCloseCallback: (() => void) | null = null;
let vimForceCloseCallback: (() => void) | null = null;
let vimCloseAllCallback: (() => void) | null = null;
let vimReloadCallback: (() => void) | null = null;
let vimNextTabCallback: (() => void) | null = null;
let vimPrevTabCallback: (() => void) | null = null;

export function setVimCallbacks(callbacks: {
  save: (() => void) | null;
  close: (() => void) | null;
  forceClose: (() => void) | null;
  closeAll: (() => void) | null;
  reload: (() => void) | null;
  nextTab: (() => void) | null;
  prevTab: (() => void) | null;
}) {
  vimSaveCallback = callbacks.save;
  vimCloseCallback = callbacks.close;
  vimForceCloseCallback = callbacks.forceClose;
  vimCloseAllCallback = callbacks.closeAll;
  vimReloadCallback = callbacks.reload;
  vimNextTabCallback = callbacks.nextTab;
  vimPrevTabCallback = callbacks.prevTab;
}

let vimCommandsRegistered = false;
export function registerVimCommands() {
  if (vimCommandsRegistered) return;
  vimCommandsRegistered = true;

  // biome-ignore lint/suspicious/noExplicitAny: Vim.defineEx not fully typed
  const defineEx = (Vim as any).defineEx;
  if (!defineEx) return;

  defineEx("set", "", (_cm: unknown, params: { args?: string[] }) => {
    const args = params.args || [];
    const arg = args[0]?.toLowerCase();

    const state = useFileEditorSidebarStore.getState();

    switch (arg) {
      case "wrap":
        state.setWrap(true);
        break;
      case "nowrap":
        state.setWrap(false);
        break;
      case "number":
      case "nu":
        state.setLineNumbers(true);
        break;
      case "nonumber":
      case "nonu":
        state.setLineNumbers(false);
        break;
      case "relativenumber":
      case "rnu":
        state.setRelativeLineNumbers(true);
        break;
      case "norelativenumber":
      case "nornu":
        state.setRelativeLineNumbers(false);
        break;
    }
  });

  defineEx("write", "w", () => {
    vimSaveCallback?.();
  });

  defineEx("quit", "q", () => {
    vimCloseCallback?.();
  });

  defineEx("q!", "q!", () => {
    vimForceCloseCallback?.();
  });

  defineEx("qall", "qa", () => {
    vimCloseAllCallback?.();
  });

  defineEx("wq", "wq", () => {
    vimSaveCallback?.();
    setTimeout(() => vimCloseCallback?.(), 100);
  });

  defineEx("e!", "e!", () => {
    vimReloadCallback?.();
  });

  defineEx("bnext", "bn", () => {
    vimNextTabCallback?.();
  });

  defineEx("bprev", "bp", () => {
    vimPrevTabCallback?.();
  });

  defineEx("tabnext", "tabn", () => {
    vimNextTabCallback?.();
  });

  defineEx("tabprev", "tabp", () => {
    vimPrevTabCallback?.();
  });
}
