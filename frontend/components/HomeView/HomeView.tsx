import { homeDir } from "@tauri-apps/api/path";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { FolderGit2, Loader2, Plus, X } from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Trans, useTranslation } from "react-i18next";
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
import { deleteWorktree } from "@/lib/api/git";
import { clearSaveFingerprints } from "@/lib/conversation-db-sync";
import {
  listProjectsForHome,
  listRecentDirectories,
  type ProjectInfo,
  type RecentDirectory,
} from "@/lib/indexer";
import { logger } from "@/lib/logger";
import {
  deleteProject,
  listProjectConfigs,
  type ProjectData,
  type ProjectFormData,
  saveProject,
} from "@/lib/projects";
import { disposeAllRuntimeTerminals } from "@/lib/terminal-restore";
import { openProject, useStore } from "@/store";
import {
  type ContextMenuState,
  ProjectContextMenu,
  WorktreeContextMenu,
  type WorktreeContextMenuState,
} from "./ContextMenus";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { SetupProjectModal } from "./SetupProjectModal";

export const HOME_VIEW_FOCUS_DEBOUNCE_MS = 100;
export const HOME_VIEW_FOCUS_MIN_INTERVAL_MS = 2000;

export const HomeView = memo(function HomeView() {
  const { t } = useTranslation();
  const { createTerminalTab } = useCreateTerminalTab();
  const [, setProjects] = useState<ProjectInfo[]>([]);
  const [savedProjects, setSavedProjects] = useState<ProjectData[]>([]);
  const [, setRecentDirectories] = useState<RecentDirectory[]>([]);
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
  const [openingProject, setOpeningProject] = useState<string | null>(null);
  const openingRef = useRef(false);
  const currentProjectName = useStore((s) => s.currentProjectName);

  const fetchData = useCallback(async (showLoadingState = true) => {
    if (showLoadingState) setIsLoading(true);
    try {
      try {
        const savedProjectsData = await listProjectConfigs();
        setSavedProjects(savedProjectsData);
      } catch (e) {
        logger.warn("Failed to load saved projects:", e);
      }

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

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const lastFocusFetchTimeRef = useRef(0);

  useEffect(() => {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;

    const handleFocus = () => {
      const now = Date.now();
      if (now - lastFocusFetchTimeRef.current < HOME_VIEW_FOCUS_MIN_INTERVAL_MS) {
        return;
      }
      if (timeoutId) {
        clearTimeout(timeoutId);
      }
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

  const handleOpenProject = useCallback(
    async (projectName: string, rootPath: string): Promise<string | null> => {
      if (openingRef.current) return null;
      openingRef.current = true;
      setOpeningProject(projectName);
      try {
        return await openProject(projectName, rootPath, { createTerminalTab });
      } finally {
        openingRef.current = false;
        setOpeningProject(null);
      }
    },
    [createTerminalTab]
  );

  const handleSetupNewProject = useCallback(() => {
    setIsSetupModalOpen(true);
  }, []);

  const handleOpenExistingProject = useCallback(async () => {
    let defaultPath: string | undefined;
    try {
      defaultPath = `${await homeDir()}golish-platform`;
    } catch {
      /* ignore */
    }
    const selected = await openFolderDialog({
      directory: true,
      multiple: false,
      title: t("home.openProjectFolderDialog"),
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
        confirm(t("home.deleteWorktreeConfirm", { branch: worktreeContextMenu.branchName }))
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
          alert(t("home.deleteWorktreeFailed", { error: String(error) }));
        }
      }
    }
  }, [worktreeContextMenu, fetchData, t]);

  const handleWorktreeCreated = useCallback(
    (worktreePath: string) => {
      fetchData(false);
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
        await handleOpenProject(data.name, data.rootPath);

        if (data.targets && data.targets.length > 0) {
          const { targets: targetsApi } = await import("@/lib/api");
          // NOTE: intentionally uses dynamic `import("@tauri-apps/api/event")` for
          // `emit` rather than `sendCustomEvent` from `@/lib/events` — static import
          // of `@/lib/events` here destabilises the `HomeView.test.tsx` module graph
          // (savedProjects becomes undefined on render, see vitest mock plan).
          // Investigate in a follow-up dedicated to `HomeView` mock setup.
          const { emit } = await import("@tauri-apps/api/event");
          try {
            await targetsApi.batchAddTargets({
              values: data.targets.join("\n"),
              group: "default",
              projectPath: data.rootPath,
            });
            await emit("targets-changed");
          } catch (e) {
            logger.error("Failed to batch-add targets:", e);
          }
        }
      } catch (error) {
        logger.error("Failed to save project:", error);
      }
    },
    [fetchData, handleOpenProject]
  );

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center text-muted-foreground">
        {t("common.loading")}
      </div>
    );
  }

  return (
    <>
      <SetupProjectModal
        isOpen={isSetupModalOpen}
        onClose={() => setIsSetupModalOpen(false)}
        onSubmit={handleProjectSubmit}
      />

      <Dialog open={!!deleteConfirm} onOpenChange={() => setDeleteConfirm(null)}>
        <DialogContent className="bg-card border-border text-foreground/80 max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("home.deleteProjectTitle")}</DialogTitle>
            <DialogDescription>
              <Trans
                i18nKey="home.deleteProjectDesc"
                values={{ name: deleteConfirm?.name ?? "" }}
                components={{ 0: <span className="text-foreground font-medium" /> }}
              />
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2">
            <Button variant="outline" onClick={() => setDeleteConfirm(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={async () => {
                if (deleteConfirm) {
                  const wasCurrent = useStore.getState().currentProjectName === deleteConfirm.name;
                  if (wasCurrent) {
                    await disposeAllRuntimeTerminals();
                    clearSaveFingerprints();
                  }
                  await deleteProject(deleteConfirm.name);
                  if (wasCurrent) {
                    useStore.getState().setCurrentProject(null);
                  }
                }
                setDeleteConfirm(null);
                fetchData(false);
              }}
            >
              {t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {worktreeModal && (
        <NewWorktreeModal
          isOpen={true}
          projectPath={worktreeModal.projectPath}
          projectName={worktreeModal.projectName}
          onClose={() => setWorktreeModal(null)}
          onSuccess={handleWorktreeCreated}
        />
      )}

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

      <div className="h-full overflow-auto bg-background">
        <div className="flex flex-col items-center justify-center min-h-full py-16 px-8">
          <div className="text-center mb-10">
            <h1 className="text-3xl font-bold text-foreground tracking-tight mb-1">Golish</h1>
            <p className="text-sm text-muted-foreground">{t("home.tagline")}</p>
          </div>

          <div className="flex items-center gap-3 mb-12">
            <button
              type="button"
              onClick={handleOpenExistingProject}
              className="flex items-center gap-2 px-5 py-2.5 bg-card border border-border rounded-lg hover:bg-muted hover:border-border/70 transition-colors text-sm text-foreground/90"
            >
              <FolderGit2 size={16} className="text-muted-foreground" />
              {t("home.openProject")}
            </button>
            <button
              type="button"
              onClick={handleSetupNewProject}
              className="flex items-center gap-2 px-5 py-2.5 bg-card border border-border rounded-lg hover:bg-muted hover:border-border/70 transition-colors text-sm text-foreground/90"
            >
              <Plus size={16} className="text-muted-foreground" />
              {t("home.newProject")}
            </button>
          </div>

          <div className="w-full max-w-lg">
            {savedProjects.length > 0 && (
              <div className="mb-8">
                <div className="flex items-center justify-between mb-3">
                  <h2 className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                    {t("home.recentProjects")}
                  </h2>
                </div>
                <div className="space-y-0.5">
                  {savedProjects.map((proj) => {
                    const isOpening = openingProject === proj.name;
                    return (
                      <div
                        key={proj.name}
                        role="button"
                        tabIndex={0}
                        onClick={() => {
                          if (!openingProject) handleOpenProject(proj.name, proj.rootPath);
                        }}
                        onKeyDown={(e) => {
                          if (!openingProject && (e.key === "Enter" || e.key === " ")) {
                            e.preventDefault();
                            handleOpenProject(proj.name, proj.rootPath);
                          }
                        }}
                        className={`w-full flex items-center justify-between px-3 py-2.5 rounded-md transition-colors text-left group ${
                          openingProject ? "cursor-wait" : "cursor-pointer hover:bg-card"
                        } ${proj.name === currentProjectName ? "bg-card" : ""} ${
                          isOpening ? "bg-card ring-1 ring-accent/30" : ""
                        }`}
                      >
                        <div className="flex items-center gap-2.5 min-w-0">
                          {isOpening ? (
                            <Loader2 size={14} className="text-accent animate-spin flex-shrink-0" />
                          ) : null}
                          <span className="text-sm text-foreground/90">{proj.name}</span>
                          {proj.name === currentProjectName && !isOpening && (
                            <span className="text-[10px] px-1.5 py-0.5 rounded bg-primary/20 text-primary">
                              {t("home.active")}
                            </span>
                          )}
                          {isOpening && (
                            <span className="text-[10px] px-1.5 py-0.5 rounded bg-accent/20 text-accent">
                              {t("common.loading")}
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-muted-foreground/60 font-mono truncate max-w-[200px]">
                            {proj.rootPath.replace(/^\/Users\/[^/]+/, "~")}
                          </span>
                          {!isOpening && (
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                setDeleteConfirm({ name: proj.name, path: proj.rootPath });
                              }}
                              className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-muted transition-all"
                              title={t("home.deleteProjectButton")}
                            >
                              <X size={12} className="text-muted-foreground" />
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {savedProjects.length === 0 && (
              <div className="text-center text-muted-foreground text-sm">
                {t("home.noProjects")}
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
});
