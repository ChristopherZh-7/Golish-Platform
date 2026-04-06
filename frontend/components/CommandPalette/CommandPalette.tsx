import {
  ArrowLeftRight,
  BookOpen,
  Clock,
  Columns,
  Database,
  Download,
  FilePenLine,
  FileSearch,
  FileText,
  Globe,
  Keyboard,
  MessageSquare,
  Monitor,
  Palette,
  Plus,
  RefreshCw,
  Rows,
  Search,
  Settings,
  Shield,
  Terminal,
  Trash2,
  Upload,
  Wrench,
  X,
} from "lucide-react";
import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save, open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import { indexDirectory, isIndexerInitialized, searchCode, searchFiles } from "@/lib/indexer";
import { notify } from "@/lib/notify";
import { getProjectPath } from "@/lib/projects";

export type PageRoute = "main" | "testbed";

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentPage: PageRoute;
  onNavigate: (page: PageRoute) => void;
  activeSessionId: string | null;
  onNewTab: () => void;
  onToggleMode: () => void;
  onClearConversation: () => void;
  onToggleFullTerminal?: () => void;
  workingDirectory?: string;
  onShowSearchResults?: (results: SearchResult[]) => void;
  onOpenSessionBrowser?: () => void;
  onToggleFileEditorPanel?: () => void;
  onOpenContextPanel?: () => void;
  onOpenSettings?: () => void;
  onOpenQuickOpen?: () => void;
  // Pane management
  onSplitPaneRight?: () => void;
  onSplitPaneDown?: () => void;
  onClosePane?: () => void;
  // Panel switching
  onOpenBrowser?: () => void;
  onOpenSecurity?: () => void;
  onToggleToolManager?: () => void;
  onToggleWiki?: () => void;
  onToggleBottomTerminal?: () => void;
  onFocusAiChat?: () => void;
  onOpenShortcutsHelp?: () => void;
  onOpenRecordings?: () => void;
}

// Types for search results
export interface SearchResult {
  file_path: string;
  line_number: number;
  line_content: string;
  matches: string[];
}

export function CommandPalette({
  open,
  onOpenChange,
  currentPage,
  onNavigate,
  activeSessionId,
  onNewTab,
  onToggleMode,
  onClearConversation,
  onToggleFullTerminal,
  workingDirectory,
  onShowSearchResults,
  onOpenSessionBrowser,
  onToggleFileEditorPanel,
  onOpenContextPanel,
  onOpenSettings,
  onOpenQuickOpen,
  onSplitPaneRight,
  onSplitPaneDown,
  onClosePane,
  onOpenBrowser,
  onOpenSecurity,
  onToggleToolManager,
  onToggleWiki,
  onToggleBottomTerminal,
  onFocusAiChat,
  onOpenShortcutsHelp,
  onOpenRecordings,
}: CommandPaletteProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);

  // Handle command selection
  const runCommand = useCallback(
    (command: () => void) => {
      onOpenChange(false);
      command();
    },
    [onOpenChange]
  );

  // Re-index workspace
  const handleReindex = useCallback(async () => {
    if (!workingDirectory) {
      notify.error("No workspace directory available");
      return;
    }
    try {
      const initialized = await isIndexerInitialized();
      if (!initialized) {
        notify.error("Indexer not initialized");
        return;
      }
      notify.info("Re-indexing workspace...");
      await indexDirectory(workingDirectory);
      notify.success("Workspace re-indexed successfully");
    } catch (error) {
      notify.error(`Failed to re-index: ${error}`);
    }
  }, [workingDirectory]);

  // Search code in workspace
  const handleSearchCode = useCallback(async () => {
    if (!searchQuery.trim()) {
      notify.error("Enter a search query first");
      return;
    }
    try {
      setIsSearching(true);
      const results = await searchCode(searchQuery);
      if (results.length === 0) {
        notify.info("No matches found");
      } else {
        notify.success(`Found ${results.length} matches`);
        onShowSearchResults?.(results);
      }
    } catch (error) {
      notify.error(`Search failed: ${error}`);
    } finally {
      setIsSearching(false);
    }
  }, [searchQuery, onShowSearchResults]);

  // Search files by name
  const handleSearchFiles = useCallback(async () => {
    if (!searchQuery.trim()) {
      notify.error("Enter a file name pattern first");
      return;
    }
    try {
      setIsSearching(true);
      const files = await searchFiles(searchQuery);
      if (files.length === 0) {
        notify.info("No files found");
      } else {
        notify.success(`Found ${files.length} files`);
        // Convert to search results format for display
        const results: SearchResult[] = files.map((f) => ({
          file_path: f,
          line_number: 0,
          line_content: "",
          matches: [],
        }));
        onShowSearchResults?.(results);
      }
    } catch (error) {
      notify.error(`File search failed: ${error}`);
    } finally {
      setIsSearching(false);
    }
  }, [searchQuery, onShowSearchResults]);

  const handleExportProject = useCallback(async () => {
    try {
      const path = await save({
        defaultPath: `golish-project-${new Date().toISOString().slice(0, 10)}.zip`,
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (!path) return;
      const result = await invoke<{ path: string; files_count: number; size_bytes: number }>("project_export", { outputPath: path, projectPath: getProjectPath() });
      const sizeMb = (result.size_bytes / 1024 / 1024).toFixed(1);
      notify.success(`Exported ${result.files_count} files (${sizeMb} MB)`);
    } catch (e) {
      notify.error(`Export failed: ${e}`);
    }
  }, []);

  const handleImportProject = useCallback(async () => {
    try {
      const path = await openFileDialog({
        filters: [{ name: "ZIP", extensions: ["zip"] }],
        multiple: false,
      });
      if (!path) return;
      const result = await invoke<{ files_count: number }>("project_import", { zipPath: path, overwrite: false, projectPath: getProjectPath() });
      notify.success(`Imported ${result.files_count} files`);
    } catch (e) {
      notify.error(`Import failed: ${e}`);
    }
  }, []);

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput
        placeholder="Type a command or search..."
        value={searchQuery}
        onValueChange={setSearchQuery}
      />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>

        {/* Navigation */}
        <CommandGroup heading="Navigation">
          <CommandItem
            onSelect={() => runCommand(() => onNavigate("main"))}
            disabled={currentPage === "main"}
          >
            <Terminal className="mr-2 size-icon-command-palette" />
            <span>Main App</span>
            {currentPage === "main" && (
              <span className="ml-auto text-xs text-muted-foreground">Current</span>
            )}
          </CommandItem>
          <CommandItem
            onSelect={() => runCommand(() => onNavigate("testbed"))}
            disabled={currentPage === "testbed"}
          >
            <Palette className="mr-2 size-icon-command-palette" />
            <span>Component Testbed</span>
            {currentPage === "testbed" && (
              <span className="ml-auto text-xs text-muted-foreground">Current</span>
            )}
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        {/* Session Actions */}
        <CommandGroup heading="Session">
          <CommandItem onSelect={() => runCommand(onNewTab)}>
            <Plus className="mr-2 size-icon-command-palette" />
            <span>New Tab</span>
            <CommandShortcut>⌘T</CommandShortcut>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(onToggleMode)}>
            <ArrowLeftRight className="mr-2 size-icon-command-palette" />
            <span>Toggle Mode</span>
            <CommandShortcut>⌘I</CommandShortcut>
          </CommandItem>
          {onToggleFullTerminal && activeSessionId && (
            <CommandItem onSelect={() => runCommand(onToggleFullTerminal)}>
              <Monitor className="mr-2 size-icon-command-palette" />
              <span>Toggle Full Terminal</span>
              <CommandShortcut>⌘⇧F</CommandShortcut>
            </CommandItem>
          )}
          {activeSessionId && (
            <CommandItem onSelect={() => runCommand(onClearConversation)}>
              <Trash2 className="mr-2 size-icon-command-palette" />
              <span>Clear Conversation</span>
              <CommandShortcut>⌘K</CommandShortcut>
            </CommandItem>
          )}
          {onOpenSessionBrowser && (
            <CommandItem onSelect={() => runCommand(onOpenSessionBrowser)}>
              <Clock className="mr-2 size-icon-command-palette" />
              <span>Browse Session History</span>
              <CommandShortcut>⌘H</CommandShortcut>
            </CommandItem>
          )}
          {onToggleFileEditorPanel && (
            <CommandItem onSelect={() => runCommand(onToggleFileEditorPanel)}>
              <FilePenLine className="mr-2 size-icon-command-palette" />
              <span>File Editor Panel</span>
              <CommandShortcut>⌘⇧E</CommandShortcut>
            </CommandItem>
          )}
          {onOpenContextPanel && (
            <CommandItem onSelect={() => runCommand(onOpenContextPanel)}>
              <Database className="mr-2 size-icon-command-palette" />
              <span>Context Capture</span>
              <CommandShortcut>⌘⇧C</CommandShortcut>
            </CommandItem>
          )}
        </CommandGroup>

        <CommandSeparator />

        {/* Pane Management */}
        {(onSplitPaneRight || onSplitPaneDown || onClosePane) && (
          <>
            <CommandGroup heading="Panes">
              {onSplitPaneRight && (
                <CommandItem onSelect={() => runCommand(onSplitPaneRight)}>
                  <Columns className="mr-2 size-icon-command-palette" />
                  <span>Split Pane Right</span>
                  <CommandShortcut>⌘D</CommandShortcut>
                </CommandItem>
              )}
              {onSplitPaneDown && (
                <CommandItem onSelect={() => runCommand(onSplitPaneDown)}>
                  <Rows className="mr-2 size-icon-command-palette" />
                  <span>Split Pane Down</span>
                  <CommandShortcut>⌘⇧D</CommandShortcut>
                </CommandItem>
              )}
              {onClosePane && (
                <CommandItem onSelect={() => runCommand(onClosePane)}>
                  <X className="mr-2 size-icon-command-palette" />
                  <span>Close Pane</span>
                  <CommandShortcut>⌘W</CommandShortcut>
                </CommandItem>
              )}
            </CommandGroup>
            <CommandSeparator />
          </>
        )}

        {/* Panels */}
        <CommandGroup heading="Panels">
          {onOpenBrowser && (
            <CommandItem onSelect={() => runCommand(onOpenBrowser)}>
              <Globe className="mr-2 size-icon-command-palette" />
              <span>Open Browser</span>
              <CommandShortcut>⌘B</CommandShortcut>
            </CommandItem>
          )}
          {onOpenSecurity && (
            <CommandItem onSelect={() => runCommand(onOpenSecurity)}>
              <Shield className="mr-2 size-icon-command-palette" />
              <span>Open Security</span>
              <CommandShortcut>⌘⇧S</CommandShortcut>
            </CommandItem>
          )}
          {onToggleToolManager && (
            <CommandItem onSelect={() => runCommand(onToggleToolManager)}>
              <Wrench className="mr-2 size-icon-command-palette" />
              <span>Tool Manager</span>
              <CommandShortcut>⌘⇧M</CommandShortcut>
            </CommandItem>
          )}
          {onToggleWiki && (
            <CommandItem onSelect={() => runCommand(onToggleWiki)}>
              <BookOpen className="mr-2 size-icon-command-palette" />
              <span>Wiki / Knowledge Base</span>
              <CommandShortcut>⌘⇧W</CommandShortcut>
            </CommandItem>
          )}
          {onToggleBottomTerminal && (
            <CommandItem onSelect={() => runCommand(onToggleBottomTerminal)}>
              <Terminal className="mr-2 size-icon-command-palette" />
              <span>Toggle Terminal</span>
              <CommandShortcut>⌘J</CommandShortcut>
            </CommandItem>
          )}
          {onFocusAiChat && (
            <CommandItem onSelect={() => runCommand(onFocusAiChat)}>
              <MessageSquare className="mr-2 size-icon-command-palette" />
              <span>Focus AI Chat</span>
              <CommandShortcut>⌘L</CommandShortcut>
            </CommandItem>
          )}
        </CommandGroup>

        <CommandSeparator />

        {/* Code Search & Analysis */}
        <CommandGroup heading="Code Search">
          {onOpenQuickOpen && (
            <CommandItem onSelect={() => runCommand(onOpenQuickOpen)}>
              <FileText className="mr-2 size-icon-command-palette" />
              <span>Open File</span>
              <CommandShortcut>⌘P</CommandShortcut>
            </CommandItem>
          )}
          <CommandItem onSelect={() => runCommand(handleSearchCode)} disabled={isSearching}>
            <Search className="mr-2 size-icon-command-palette" />
            <span>Search Code</span>
            <span className="ml-auto text-xs text-muted-foreground">regex</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(handleSearchFiles)} disabled={isSearching}>
            <FileSearch className="mr-2 size-icon-command-palette" />
            <span>Find Files</span>
            <span className="ml-auto text-xs text-muted-foreground">pattern</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(handleReindex)} disabled={!workingDirectory}>
            <RefreshCw className="mr-2 size-icon-command-palette" />
            <span>Re-index Workspace</span>
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        {/* Project */}
        <CommandGroup heading="Project">
          <CommandItem onSelect={() => runCommand(handleExportProject)}>
            <Download className="mr-2 size-icon-command-palette" />
            <span>Export Project Data</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(handleImportProject)}>
            <Upload className="mr-2 size-icon-command-palette" />
            <span>Import Project Data</span>
          </CommandItem>
          <CommandItem onSelect={() => runCommand(() => onOpenRecordings?.())}>
            <Terminal className="mr-2 size-icon-command-palette" />
            <span>Terminal Recordings</span>
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        {/* Help */}
        <CommandGroup heading="Help">
          <CommandItem onSelect={() => runCommand(() => onOpenShortcutsHelp?.())}>
            <Keyboard className="mr-2 size-icon-command-palette" />
            <span>Keyboard Shortcuts</span>
            <CommandShortcut>⌘/</CommandShortcut>
          </CommandItem>
          <CommandItem disabled>
            <FileText className="mr-2 size-icon-command-palette" />
            <span>Documentation</span>
          </CommandItem>
          {onOpenSettings && (
            <CommandItem onSelect={() => runCommand(onOpenSettings)}>
              <Settings className="mr-2 size-icon-command-palette" />
              <span>Settings</span>
              <CommandShortcut>⌘,</CommandShortcut>
            </CommandItem>
          )}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}

// Hook to manage command palette state
export function useCommandPalette() {
  return {
    // Can be extended with more functionality
  };
}
