import { homeDir } from "@tauri-apps/api/path";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import {
  ChevronDown,
  ChevronRight,
  File,
  FolderGit2,
  FolderOpen,
  GitBranch,
  Minus,
  Plus,
  Trash2,
  TreePine,
  X,
} from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import {
  listProjectsForHome,
  listRecentDirectories,
  type ProjectInfo,
  type RecentDirectory,
  removeRecentDirectory,
} from "@/lib/indexer";
import { logger } from "@/lib/logger";
import {
  deleteProject,
  listProjectConfigs,
  type ProjectData,
  type ProjectFormData,
  saveProject,
} from "@/lib/projects";
import { deleteWorktree } from "@/lib/tauri";
import {
  loadWorkspaceState,
  setLastProjectName,
  toChatConversation,
} from "@/lib/workspace-storage";
import { useStore } from "@/store";
import { createNewConversation } from "@/store/slices/conversation";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { SetupProjectModal } from "./SetupProjectModal";

/**
 * Debounce delay for window focus refresh (milliseconds).
 * Small delay to batch rapid focus events.
 */
export const HOME_VIEW_FOCUS_DEBOUNCE_MS = 100;

/**
 * Minimum interval between focus-triggered fetches (milliseconds).
 * Prevents excessive fetching when user rapidly switches windows.
 */
export const HOME_VIEW_FOCUS_MIN_INTERVAL_MS = 2000;

/** Context menu state */
interface ContextMenuState {
  x: number;
  y: number;
  projectPath: string;
  projectName: string;
}

/** Worktree context menu state */
interface WorktreeContextMenuState {
  x: number;
  y: number;
  projectPath: string;
  worktreePath: string;
  branchName: string;
}

/** Stats badge showing file count, insertions, and deletions */
const StatsBadge = memo(function StatsBadge({
  fileCount,
  insertions,
  deletions,
}: {
  fileCount: number;
  insertions: number;
  deletions: number;
}) {
  if (fileCount === 0 && insertions === 0 && deletions === 0) {
    return null;
  }

  return (
    <div className="flex items-center bg-[#0d1117] px-2 py-1 rounded-full border border-[#30363d] space-x-2 text-xs text-gray-500">
      {fileCount > 0 && (
        <div className="flex items-center">
          <File size={12} className="mr-0.5 text-gray-500" />
          <span>{fileCount}</span>
        </div>
      )}
      {insertions > 0 && (
        <div className="flex items-center">
          <Plus size={12} className="mr-0.5 text-[#3fb950]" />
          <span>{insertions}</span>
        </div>
      )}
      {deletions > 0 && (
        <div className="flex items-center">
          <Minus size={12} className="mr-0.5 text-[#f85149]" />
          <span>{deletions}</span>
        </div>
      )}
    </div>
  );
});

/** Worktree count badge */
const WorktreeBadge = memo(function WorktreeBadge({ count }: { count: number }) {
  return (
    <div className="flex items-center bg-[#0d1117] px-2 py-1 rounded-full border border-[#30363d] text-xs text-gray-500">
      <TreePine size={14} className="mr-1 text-[#238636]" />
      {count}
    </div>
  );
});

/** Worktree context menu component */
function WorktreeContextMenu({
  x,
  y,
  onDelete,
  onClose,
}: {
  x: number;
  y: number;
  onDelete: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on click outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleEscape);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleEscape);
    };
  }, [onClose]);

  const handleDeleteClick = useCallback(() => {
    onDelete();
    onClose();
  }, [onDelete, onClose]);

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-[#1c2128] border border-[#30363d] rounded-md shadow-xl py-1 min-w-[160px]"
      style={{ left: x, top: y }}
    >
      <button
        type="button"
        onClick={handleDeleteClick}
        className="w-full flex items-center px-3 py-2 text-sm text-red-400 hover:bg-[#30363d] hover:text-red-300 transition-colors text-left"
      >
        <Trash2 size={14} className="mr-2" />
        Delete Worktree
      </button>
    </div>
  );
}

/** Single project row (expandable) - memoized to prevent re-renders when parent state changes */
export const ProjectRow = memo(function ProjectRow({
  project,
  isExpanded,
  onToggle,
  onOpenDirectory,
  onContextMenu,
  onWorktreeContextMenu,
  onDelete,
}: {
  project: ProjectInfo;
  isExpanded: boolean;
  onToggle: () => void;
  onOpenDirectory: (path: string) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onWorktreeContextMenu: (e: React.MouseEvent, worktreePath: string, branchName: string) => void;
  onDelete: () => void;
}) {
  return (
    <div className="border-b border-[#30363d]/50 last:border-0">
      {/* Project header */}
      <button
        type="button"
        onClick={onToggle}
        onContextMenu={onContextMenu}
        className="w-full flex items-center justify-between p-3 hover:bg-[#1c2128] transition-colors group text-left"
      >
        <div className="flex items-center min-w-0 mr-4">
          <div className="mr-2 flex-shrink-0 hover:bg-[#30363d] rounded p-0.5 transition-colors">
            {isExpanded ? (
              <ChevronDown size={14} className="text-gray-500" />
            ) : (
              <ChevronRight size={14} className="text-gray-500" />
            )}
          </div>
          <FolderGit2
            size={16}
            className="text-gray-500 mr-3 flex-shrink-0 group-hover:text-[#58a6ff] transition-colors"
          />
          <div className="min-w-0">
            <div className="text-sm font-medium text-gray-300 truncate group-hover:text-white transition-colors">
              {project.name}
            </div>
          </div>
        </div>

        <div className="flex items-center text-xs text-gray-500 flex-shrink-0 space-x-3">
          <WorktreeBadge count={project.branches.length} />
          <span>{project.last_activity}</span>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
            className="opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded hover:bg-red-900/30 text-gray-500 hover:text-red-400 flex-shrink-0"
            title="Delete project"
          >
            <X size={14} />
          </button>
        </div>
      </button>

      {/* Expanded branches */}
      {isExpanded && project.branches.length > 0 && (
        <div className="bg-[#0d1117] border-t border-[#30363d]/50 max-h-[420px] overflow-y-auto custom-scrollbar">
          {project.branches.map((branch) => (
            <button
              type="button"
              key={branch.name}
              onClick={() => onOpenDirectory(branch.path)}
              onContextMenu={(e) => onWorktreeContextMenu(e, branch.path, branch.name)}
              className="w-full flex items-center p-3 pl-12 hover:bg-[#161b22] transition-colors text-left border-b border-[#30363d]/30 last:border-0 group"
            >
              <div className="flex items-center min-w-0 w-[450px] mr-4">
                <div className="min-w-0">
                  <div className="flex items-center text-xs text-gray-500">
                    <GitBranch size={12} className="mr-1 text-[#58a6ff] flex-shrink-0" />
                    <span className="text-gray-300 truncate">{branch.name}</span>
                  </div>
                  <div className="text-xs text-gray-600 truncate font-mono mt-0.5">
                    {branch.path}
                  </div>
                </div>
              </div>

              <StatsBadge
                fileCount={branch.file_count}
                insertions={branch.insertions}
                deletions={branch.deletions}
              />

              <div className="flex items-center text-xs text-gray-500 flex-shrink-0 ml-auto space-x-2">
                <span>{branch.last_activity}</span>
                <ChevronRight
                  size={14}
                  className="opacity-0 group-hover:opacity-100 transition-opacity text-[#58a6ff]"
                />
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
});

/** Single recent directory row - memoized to prevent re-renders when parent state changes */
export const RecentDirectoryRow = memo(function RecentDirectoryRow({
  directory,
  onOpen,
  onRemove,
}: {
  directory: RecentDirectory;
  onOpen: () => void;
  onRemove: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="w-full flex items-center p-3 hover:bg-[#1c2128] transition-colors group text-left border-b border-[#30363d]/50 last:border-0"
    >
      <div className="flex items-center min-w-0 w-[500px] mr-4">
        <FolderOpen
          size={16}
          className="text-gray-500 mr-3 flex-shrink-0 group-hover:text-[#58a6ff] transition-colors"
        />
        <div className="min-w-0">
          <div className="text-sm font-medium text-gray-300 truncate group-hover:text-white transition-colors">
            {directory.name}
          </div>
          {directory.branch && (
            <div className="flex items-center text-xs text-gray-500 opacity-60">
              <GitBranch size={12} className="mr-1 text-[#58a6ff]" />
              <span className="text-gray-300">{directory.branch}</span>
            </div>
          )}
        </div>
      </div>

      <StatsBadge
        fileCount={directory.file_count}
        insertions={directory.insertions}
        deletions={directory.deletions}
      />

      <div className="flex items-center text-xs text-gray-500 flex-shrink-0 ml-auto space-x-2">
        <span>{directory.last_accessed}</span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          className="opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded hover:bg-red-900/30 text-gray-500 hover:text-red-400 flex-shrink-0"
          title="Remove from recent"
        >
          <X size={14} />
        </button>
      </div>
    </button>
  );
});

/** Context menu component */
function ProjectContextMenu({
  x,
  y,
  onNewWorktree,
  onClose,
}: {
  x: number;
  y: number;
  onNewWorktree: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on click outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleEscape);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleEscape);
    };
  }, [onClose]);

  const handleNewWorktreeClick = useCallback(() => {
    onNewWorktree();
    onClose();
  }, [onNewWorktree, onClose]);

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-[#1c2128] border border-[#30363d] rounded-md shadow-xl py-1 min-w-[160px]"
      style={{ left: x, top: y }}
    >
      <button
        type="button"
        onClick={handleNewWorktreeClick}
        className="w-full flex items-center px-3 py-2 text-sm text-gray-300 hover:bg-[#30363d] hover:text-white transition-colors text-left"
      >
        <TreePine size={14} className="mr-2 text-[#238636]" />
        New Worktree
      </button>
    </div>
  );
}

export const HomeView = memo(function HomeView() {
  const { createTerminalTab } = useCreateTerminalTab();
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [savedProjects, setSavedProjects] = useState<ProjectData[]>([]);
  const [recentDirectories, setRecentDirectories] = useState<RecentDirectory[]>([]);
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(new Set());
  const [isLoading, setIsLoading] = useState(true);
  const [isSetupModalOpen, setIsSetupModalOpen] = useState(false);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [worktreeContextMenu, setWorktreeContextMenu] = useState<WorktreeContextMenuState | null>(
    null
  );
  const [worktreeModal, setWorktreeModal] = useState<{
    projectPath: string;
    projectName: string;
  } | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<{ name: string; path: string } | null>(null);
  const currentProjectName = useStore((s) => s.currentProjectName);

  // Fetch data — saved projects are critical, indexer data is nice-to-have
  const fetchData = useCallback(async (showLoadingState = true) => {
    if (showLoadingState) setIsLoading(true);
    try {
      // Saved projects are lightweight — fetch first so the UI is usable quickly
      try {
        const savedProjectsData = await listProjectConfigs();
        setSavedProjects(savedProjectsData);
      } catch (e) {
        logger.warn("Failed to load saved projects:", e);
      }

      // Indexer data can be slow — don't block the UI on it
      setIsLoading(false);

      try {
        const [projectsData, directoriesData] = await Promise.all([
          listProjectsForHome(),
          listRecentDirectories(10),
        ]);
        setProjects(projectsData);
        setRecentDirectories(directoriesData);
      } catch (e) {
        logger.warn("Failed to load indexer data:", e);
      }
    } finally {
      setIsLoading(false);
    }
  }, []);

  // Fetch data on mount
  useEffect(() => {
    fetchData();
  }, [fetchData]);

  // Refresh on window focus with debounce to avoid excessive fetches
  // when user rapidly switches windows
  // Track last fetch time at component level so it persists across effect re-runs
  // Start with 0 to allow the first focus fetch (after initial mount fetch)
  const lastFocusFetchTimeRef = useRef(0);

  useEffect(() => {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;

    const handleFocus = () => {
      const now = Date.now();
      // Skip if we fetched recently (minimum interval between fetches)
      if (now - lastFocusFetchTimeRef.current < HOME_VIEW_FOCUS_MIN_INTERVAL_MS) {
        return;
      }
      // Clear any pending debounced fetch
      if (timeoutId) {
        clearTimeout(timeoutId);
      }
      // Debounce the fetch
      timeoutId = setTimeout(() => {
        lastFocusFetchTimeRef.current = Date.now();
        fetchData(false);
        timeoutId = null;
      }, HOME_VIEW_FOCUS_DEBOUNCE_MS);
    };

    window.addEventListener("focus", handleFocus);
    return () => {
      window.removeEventListener("focus", handleFocus);
      if (timeoutId) {
        clearTimeout(timeoutId);
      }
    };
  }, [fetchData]);

  const toggleProject = useCallback((path: string) => {
    setExpandedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  const handleOpenDirectory = useCallback(
    (path: string) => {
      createTerminalTab(path);
    },
    [createTerminalTab]
  );

  /** Open a project: load its workspace state, set as current, create terminal */
  const handleOpenProject = useCallback(
    async (projectName: string, rootPath: string) => {
      try {
        // Set as current project
        useStore.getState().setCurrentProject(projectName);
        setLastProjectName(projectName);

        // Try to load saved workspace state for this project
        const saved = await loadWorkspaceState(projectName);
        if (saved && saved.conversations.length > 0) {
          const restoredConvs = saved.conversations.map(toChatConversation);
          useStore.getState().restoreConversations(
            restoredConvs,
            saved.conversationOrder,
            saved.activeConversationId,
          );
        } else {
          // No saved state for this project — create a fresh conversation
          const conv = createNewConversation();
          useStore.getState().restoreConversations([conv], [conv.id], conv.id);
        }

        // Create a terminal in the project root
        createTerminalTab(rootPath);
      } catch (error) {
        logger.error("Failed to open project:", error);
      }
    },
    [createTerminalTab],
  );

  const handleSetupNewProject = useCallback(() => {
    setIsSetupModalOpen(true);
  }, []);

  const handleOpenExistingProject = useCallback(async () => {
    let defaultPath: string | undefined;
    try { defaultPath = (await homeDir()) + "golish-platform"; } catch { /* ignore */ }
    const selected = await openFolderDialog({
      directory: true,
      multiple: false,
      title: "Open project folder",
      defaultPath,
    });
    if (!selected) return;
    const folderName = selected.split("/").pop() || selected.split("\\").pop() || "untitled";
    try {
      await saveProject({ name: folderName, rootPath: selected });
      fetchData(false);
      handleOpenProject(folderName, selected);
    } catch (error) {
      logger.error("Failed to open project:", error);
    }
  }, [fetchData, handleOpenProject]);

  const handleProjectContextMenu = useCallback((e: React.MouseEvent, project: ProjectInfo) => {
    e.preventDefault();
    setContextMenu({
      x: e.clientX,
      y: e.clientY,
      projectPath: project.path,
      projectName: project.name,
    });
  }, []);

  const handleWorktreeContextMenu = useCallback(
    (e: React.MouseEvent, projectPath: string, worktreePath: string, branchName: string) => {
      e.preventDefault();
      e.stopPropagation(); // Prevent project context menu
      setWorktreeContextMenu({
        x: e.clientX,
        y: e.clientY,
        projectPath,
        worktreePath,
        branchName,
      });
    },
    []
  );

  const handleNewWorktree = useCallback(() => {
    if (contextMenu) {
      setWorktreeModal({
        projectPath: contextMenu.projectPath,
        projectName: contextMenu.projectName,
      });
    }
  }, [contextMenu]);

  const handleDeleteWorktree = useCallback(async () => {
    if (worktreeContextMenu) {
      if (
        confirm(`Are you sure you want to delete worktree "${worktreeContextMenu.branchName}"?`)
      ) {
        try {
          await deleteWorktree(
            worktreeContextMenu.projectPath,
            worktreeContextMenu.worktreePath,
            true
          );
          fetchData(false);
        } catch (error) {
          logger.error("Failed to delete worktree:", error);
          alert(`Failed to delete worktree: ${error}`);
        }
      }
    }
  }, [worktreeContextMenu, fetchData]);

  const handleWorktreeCreated = useCallback(
    (worktreePath: string) => {
      // Refresh the project list to show the new worktree
      fetchData(false);
      // Optionally open the new worktree in a tab
      createTerminalTab(worktreePath);
    },
    [fetchData, createTerminalTab]
  );

  const handleProjectSubmit = useCallback(
    async (data: ProjectFormData) => {
      try {
        await saveProject(data);
        setIsSetupModalOpen(false);
        fetchData(false);
        // Open the newly created project
        handleOpenProject(data.name, data.rootPath);
      } catch (error) {
        logger.error("Failed to save project:", error);
      }
    },
    [fetchData, handleOpenProject]
  );

  if (isLoading) {
    return <div className="h-full flex items-center justify-center text-gray-500">Loading...</div>;
  }

  return (
    <>
      <SetupProjectModal
        isOpen={isSetupModalOpen}
        onClose={() => setIsSetupModalOpen(false)}
        onSubmit={handleProjectSubmit}
      />

      {/* Delete Project Confirmation Dialog */}
      <Dialog open={!!deleteConfirm} onOpenChange={() => setDeleteConfirm(null)}>
        <DialogContent className="bg-[#1c2128] border-[#30363d] text-gray-300 max-w-sm">
          <DialogHeader>
            <DialogTitle>Delete Project</DialogTitle>
            <DialogDescription>
              Remove <span className="text-white font-medium">{deleteConfirm?.name}</span> from
              Golish? This deletes the project configuration but won't delete any files.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button variant="outline" onClick={() => setDeleteConfirm(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={async () => {
                if (deleteConfirm) {
                  const wasCurrent = useStore.getState().currentProjectName === deleteConfirm.name;
                  await deleteProject(deleteConfirm.name);
                  if (wasCurrent) {
                    useStore.getState().setCurrentProject(null);
                    localStorage.removeItem("golish-pentest-conversations");
                    localStorage.removeItem("golish-pentest-conv-terminals");
                  }
                }
                setDeleteConfirm(null);
                fetchData(false);
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* New Worktree Modal */}
      {worktreeModal && (
        <NewWorktreeModal
          isOpen={true}
          projectPath={worktreeModal.projectPath}
          projectName={worktreeModal.projectName}
          onClose={() => setWorktreeModal(null)}
          onSuccess={handleWorktreeCreated}
        />
      )}

      {/* Context Menu */}
      {contextMenu &&
        createPortal(
          <ProjectContextMenu
            x={contextMenu.x}
            y={contextMenu.y}
            onNewWorktree={handleNewWorktree}
            onClose={() => setContextMenu(null)}
          />,
          document.body
        )}

      {/* Worktree Context Menu */}
      {worktreeContextMenu &&
        createPortal(
          <WorktreeContextMenu
            x={worktreeContextMenu.x}
            y={worktreeContextMenu.y}
            onDelete={handleDeleteWorktree}
            onClose={() => setWorktreeContextMenu(null)}
          />,
          document.body
        )}

      <div className="h-full overflow-auto bg-[#0d1117]">
        <div className="flex flex-col items-center justify-center min-h-full py-16 px-8">
          {/* Logo / Title */}
          <div className="text-center mb-10">
            <h1 className="text-3xl font-bold text-white tracking-tight mb-1">Golish</h1>
            <p className="text-sm text-gray-500">Penetration Testing Platform</p>
          </div>

          {/* Action Buttons */}
          <div className="flex items-center gap-3 mb-12">
            <button
              type="button"
              onClick={handleOpenExistingProject}
              className="flex items-center gap-2 px-5 py-2.5 bg-[#161b22] border border-[#30363d] rounded-lg hover:bg-[#1c2128] hover:border-[#484f58] transition-colors text-sm text-gray-200"
            >
              <FolderOpen size={16} className="text-gray-400" />
              Open Project
            </button>
            <button
              type="button"
              onClick={handleSetupNewProject}
              className="flex items-center gap-2 px-5 py-2.5 bg-[#161b22] border border-[#30363d] rounded-lg hover:bg-[#1c2128] hover:border-[#484f58] transition-colors text-sm text-gray-200"
            >
              <Plus size={16} className="text-gray-400" />
              New Project
            </button>
          </div>

          {/* Projects List */}
          <div className="w-full max-w-lg">
            {savedProjects.length > 0 && (
              <div className="mb-8">
                <div className="flex items-center justify-between mb-3">
                  <h2 className="text-xs font-medium text-gray-500 uppercase tracking-wider">
                    Recent projects
                  </h2>
                </div>
                <div className="space-y-0.5">
                  {savedProjects.map((proj) => (
                    <button
                      key={proj.name}
                      type="button"
                      onClick={() => handleOpenProject(proj.name, proj.rootPath)}
                      className={`w-full flex items-center justify-between px-3 py-2.5 rounded-md hover:bg-[#161b22] transition-colors text-left group ${
                        proj.name === currentProjectName ? "bg-[#161b22]" : ""
                      }`}
                    >
                      <div className="flex items-center gap-2.5 min-w-0">
                        <span className="text-sm text-gray-200">{proj.name}</span>
                        {proj.name === currentProjectName && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-[#238636]/20 text-[#238636]">
                            Active
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-2">
                        <span className="text-xs text-gray-600 font-mono truncate max-w-[200px]">
                          {proj.rootPath.replace(/^\/Users\/[^/]+/, "~")}
                        </span>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            setDeleteConfirm({ name: proj.name, path: proj.rootPath });
                          }}
                          className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-[#30363d] transition-all"
                          title="Delete project"
                        >
                          <X size={12} className="text-gray-500" />
                        </button>
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            {savedProjects.length === 0 && (
              <div className="text-center text-gray-500 text-sm">
                No projects yet. Create one to get started.
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
});
