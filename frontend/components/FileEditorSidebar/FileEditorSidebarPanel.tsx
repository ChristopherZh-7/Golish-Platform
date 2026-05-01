import type { Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { vim } from "@replit/codemirror-vim";
import { basicSetup as uiwBasicSetup } from "@uiw/codemirror-extensions-basic-setup";
import { lineNumbersRelative } from "@uiw/codemirror-extensions-line-numbers-relative";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import { Eye, FileText, FolderOpen, Save, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useFileEditorSidebar } from "@/hooks/useFileEditorSidebar";
import { useFileWatcher } from "@/hooks/useFileWatcher";
import { useThrottledResize } from "@/hooks/useThrottledResize";
import { getLanguageExtension } from "@/lib/codemirror-languages";
import { golishTheme } from "@/lib/codemirror-theme";
import { cn } from "@/lib/utils";
import { useFocusedSessionId, useStore } from "@/store";
import { useFileEditorSidebarStore } from "@/store/file-editor-sidebar";
import { EditablePathBar, FileOpenPrompt, MarkdownPreview } from "./EditorPanels";
import { FileBrowser } from "./FileBrowser";
import { FileConflictBanner } from "./FileConflictBanner";
import { TabBar } from "./TabBar";
import { registerVimCommands, setVimCallbacks } from "./VimCommands";

interface FileEditorSidebarPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const MIN_WIDTH = 320;
const MAX_WIDTH = 1200;
const DEFAULT_WIDTH = 420;

export function FileEditorSidebarPanel({ open, onOpenChange }: FileEditorSidebarPanelProps) {
  const activeSessionId = useStore((state) => state.activeSessionId);
  const focusedSessionId = useFocusedSessionId(activeSessionId);
  const workingDirectory = useStore((state) =>
    focusedSessionId ? state.sessions[focusedSessionId]?.workingDirectory : undefined
  );
  const {
    activeTabId,
    activeTab,
    activeFile,
    tabs,
    vimMode,
    vimModeState,
    wrap,
    lineNumbers,
    relativeLineNumbers,
    showHiddenFiles,
    recentFiles,
    width,
    openFile,
    openBrowser,
    saveFile,
    reloadFile,
    setActiveTab,
    closeTab,
    closeAllTabs,
    closeOtherTabs,
    setOpen,
    setWidth,
    updateFileContent,
    setBrowserPath,
    setVimMode,
    setVimModeState,
    setShowHiddenFiles,
    toggleMarkdownPreview,
    reorderTabs,
  } = useFileEditorSidebar(workingDirectory || undefined);

  useFileWatcher();

  const [containerWidth, setContainerWidth] = useState(DEFAULT_WIDTH);
  const [languageExtension, setLanguageExtension] = useState<Extension | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<ReactCodeMirrorRef>(null);

  const handleWidthChange = useCallback(
    (newWidth: number) => {
      setContainerWidth(newWidth);
      setWidth(newWidth);
    },
    [setWidth]
  );

  const { startResizing: onStartResize } = useThrottledResize({
    minWidth: MIN_WIDTH,
    maxWidth: MAX_WIDTH,
    onWidthChange: handleWidthChange,
    calculateWidth: (e) => window.innerWidth - e.clientX,
  });

  const goToNextTab = useCallback(() => {
    if (tabs.length <= 1) return;
    const tabOrder = tabs.map((tab) => tab.id);
    const currentIndex = activeTabId ? tabOrder.indexOf(activeTabId) : -1;
    const nextIndex = (currentIndex + 1) % tabs.length;
    const nextId = tabOrder[nextIndex];
    if (nextId) setActiveTab(nextId);
  }, [activeTabId, tabs, setActiveTab]);

  const goToPrevTab = useCallback(() => {
    if (tabs.length <= 1) return;
    const tabOrder = tabs.map((tab) => tab.id);
    const currentIndex = activeTabId ? tabOrder.indexOf(activeTabId) : -1;
    const prevIndex = (currentIndex - 1 + tabs.length) % tabs.length;
    const prevId = tabOrder[prevIndex];
    if (prevId) setActiveTab(prevId);
  }, [activeTabId, tabs, setActiveTab]);

  useEffect(() => {
    if (width) {
      setContainerWidth(width);
    }
  }, [width]);

  useEffect(() => {
    setOpen(open);
  }, [open, setOpen]);

  useEffect(() => {
    let cancelled = false;

    async function loadLanguage() {
      const lang = activeFile?.language;
      if (!lang) {
        setLanguageExtension(null);
        return;
      }

      const ext = await getLanguageExtension(lang);
      if (!cancelled) {
        setLanguageExtension(ext);
      }
    }

    void loadLanguage();

    return () => {
      cancelled = true;
    };
  }, [activeFile?.language]);

  useEffect(() => {
    if (vimMode) {
      registerVimCommands();
      setVimCallbacks({
        save: () => void saveFile(),
        close: () => {
          closeTab();
          const state = useFileEditorSidebarStore.getState();
          if (state.tabOrder.length === 0) {
            onOpenChange(false);
          }
        },
        forceClose: () => {
          closeTab();
          const state = useFileEditorSidebarStore.getState();
          if (state.tabOrder.length === 0) {
            onOpenChange(false);
          }
        },
        closeAll: () => {
          closeAllTabs();
          onOpenChange(false);
        },
        reload: () => void reloadFile(),
        nextTab: goToNextTab,
        prevTab: goToPrevTab,
      });
    } else {
      setVimCallbacks({
        save: null,
        close: null,
        forceClose: null,
        closeAll: null,
        reload: null,
        nextTab: null,
        prevTab: null,
      });
    }
    return () => {
      setVimCallbacks({
        save: null,
        close: null,
        forceClose: null,
        closeAll: null,
        reload: null,
        nextTab: null,
        prevTab: null,
      });
    };
  }, [
    vimMode,
    saveFile,
    reloadFile,
    closeTab,
    closeAllTabs,
    onOpenChange,
    goToNextTab,
    goToPrevTab,
  ]);

  useEffect(() => {
    if (!vimMode || !editorRef.current?.view) return;

    // biome-ignore lint/suspicious/noExplicitAny: CodeMirror vim internals not fully typed
    const cm = (editorRef.current.view as any).cm;
    if (!cm) return;

    const handler = (event: { mode: string }) => {
      const mode = event.mode.toLowerCase();
      if (mode === "normal" || mode === "insert" || mode === "visual") {
        setVimModeState(mode);
      }
    };

    cm.on("vim-mode-change", handler);
    return () => {
      cm.off("vim-mode-change", handler);
    };
  }, [vimMode, setVimModeState]);

  const extensions = useMemo(() => {
    const ext: Extension[] = [];

    ext.push(
      uiwBasicSetup({
        lineNumbers: (lineNumbers ?? true) && !relativeLineNumbers,
        foldGutter: true,
        highlightActiveLine: true,
      })
    );

    if (relativeLineNumbers) {
      ext.push(lineNumbersRelative);
    }

    if (languageExtension) {
      ext.push(languageExtension);
    }

    if (wrap) {
      ext.push(EditorView.lineWrapping);
    }
    if (vimMode) {
      ext.push(vim());
    }
    ext.push(
      keymap.of([
        {
          key: "Mod-s",
          preventDefault: true,
          run: () => {
            void saveFile();
            return true;
          },
        },
      ])
    );

    return ext;
  }, [saveFile, languageExtension, vimMode, wrap, lineNumbers, relativeLineNumbers]);

  if (!open) return null;

  const hasTabs = tabs.length > 0;

  const renderTabContent = () => {
    if (!activeTab) {
      return (
        <FileOpenPrompt
          workingDirectory={workingDirectory ?? undefined}
          onOpen={(path) => openFile(path)}
          onOpenBrowser={() => openBrowser()}
          recentFiles={recentFiles}
        />
      );
    }

    if (activeTab.type === "browser") {
      return (
        <FileBrowser
          currentPath={activeTab.browser.currentPath}
          workingDirectory={workingDirectory ?? undefined}
          onNavigate={(path) => {
            if (activeTab) {
              setBrowserPath(activeTab.id, path);
            }
          }}
          onOpenFile={(path) => openFile(path)}
          showHiddenFiles={showHiddenFiles}
          onToggleHiddenFiles={() => setShowHiddenFiles(!showHiddenFiles)}
        />
      );
    }

    if (activeTab.type === "file") {
      if (activeTab.file.language === "markdown" && activeTab.file.markdownPreview) {
        return (
          <div className="flex-1 min-h-0 overflow-auto p-4">
            <MarkdownPreview content={activeTab.file.content} />
          </div>
        );
      }

      return (
        <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
          {activeTab.file.externallyModified && (
            <FileConflictBanner tabId={activeTab.id} filePath={activeTab.file.path} />
          )}
          <CodeMirror
            ref={editorRef}
            value={activeTab.file.content}
            height="100%"
            theme={golishTheme}
            extensions={extensions}
            basicSetup={false}
            onChange={(value) => updateFileContent(activeTab.id, value)}
            className="h-full [&_.cm-editor]:h-full [&_.cm-scroller]:overflow-auto"
          />
        </div>
      );
    }

    return null;
  };

  return (
    <div
      ref={panelRef}
      className="bg-card border-l border-border flex flex-col relative"
      style={{
        width: `${containerWidth}px`,
        minWidth: `${MIN_WIDTH}px`,
        maxWidth: `${MAX_WIDTH}px`,
      }}
    >
      {/* biome-ignore lint/a11y/noStaticElementInteractions: resize handle is mouse-only */}
      <div
        className="absolute top-0 left-0 w-1 h-full cursor-col-resize hover:bg-primary/50 transition-colors z-10 group"
        onMouseDown={onStartResize}
      />

      <div className="flex items-center justify-between px-2 py-1.5 bg-muted border-b border-border">
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-1.5 gap-1 text-[11px]"
            onClick={() => openBrowser()}
            title="Open file browser"
          >
            <FolderOpen className="w-3 h-3" />
            <span>Browse</span>
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-1.5 gap-1 text-[11px]"
            onClick={() => void saveFile()}
            disabled={!activeFile}
            title="Save file (Ctrl+S)"
          >
            <Save className="w-3 h-3" />
            <span>Save</span>
          </Button>
          <div className="w-px h-3 bg-border mx-1" />
          {activeTab?.type === "file" && activeTab.file.language === "markdown" && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5 gap-1 text-[11px]"
              onClick={() => toggleMarkdownPreview(activeTab.id)}
              title={activeTab.file.markdownPreview ? "Switch to edit" : "Switch to preview"}
            >
              {activeTab.file.markdownPreview ? (
                <>
                  <FileText className="w-3 h-3" />
                  <span>Edit</span>
                </>
              ) : (
                <>
                  <Eye className="w-3 h-3" />
                  <span>Preview</span>
                </>
              )}
            </Button>
          )}
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-5 w-5"
          onClick={() => {
            closeAllTabs();
            onOpenChange(false);
          }}
          title="Close file editor"
        >
          <X className="w-3 h-3" />
        </Button>
      </div>

      {hasTabs && (
        <TabBar
          tabs={tabs}
          activeTabId={activeTabId}
          onSelectTab={setActiveTab}
          onCloseTab={(tabId) => {
            closeTab(tabId);
            const state = useFileEditorSidebarStore.getState();
            if (state.tabOrder.length === 0) {
              onOpenChange(false);
            }
          }}
          onCloseOtherTabs={closeOtherTabs}
          onReorderTabs={reorderTabs}
          onCloseAllTabs={() => {
            closeAllTabs();
            onOpenChange(false);
          }}
        />
      )}

      <div className="flex-1 min-h-0 flex flex-col">{renderTabContent()}</div>

      <div className="px-3 py-2 border-t border-border text-xs text-muted-foreground flex items-center justify-between">
        <div className="flex-1 flex items-center gap-2 min-w-0">
          {vimMode && activeTab?.type === "file" && (
            <Badge variant="outline" className="text-[11px] font-mono uppercase">
              {vimModeState ?? "normal"}
            </Badge>
          )}
          {activeTab?.type === "browser" && (
            <EditablePathBar
              value={activeTab.browser.currentPath || workingDirectory || ""}
              onNavigate={(path) => setBrowserPath(activeTab.id, path)}
            />
          )}
        </div>
        <div className="flex items-center gap-2 shrink-0">
          {activeTab?.type === "file" && (
            <button
              type="button"
              onClick={() => setVimMode(!vimMode)}
              className={cn(
                "text-[11px] px-1.5 py-0.5 rounded transition-colors",
                vimMode
                  ? "bg-primary/20 text-primary hover:bg-primary/30"
                  : "text-muted-foreground hover:text-foreground hover:bg-muted"
              )}
              title={vimMode ? "Disable Vim mode" : "Enable Vim mode"}
            >
              Vim
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
