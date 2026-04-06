import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";
import {
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  ClipboardList,
  MessageSquare,
  Plus,
  Trash2,
  Wrench,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";

interface CheckItem {
  id: string;
  title: string;
  description: string;
  checked: boolean;
  notes: string;
  tools: string[];
}

interface Phase {
  id: string;
  name: string;
  description: string;
  items: CheckItem[];
}

interface MethodologyTemplate {
  id: string;
  name: string;
  description: string;
  phases: Phase[];
}

interface ProjectMethodology {
  id: string;
  template_id: string;
  template_name: string;
  project_name: string;
  phases: Phase[];
  created_at: string;
  updated_at: string;
}

export function MethodologyPanel() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [view, setView] = useState<"list" | "project">("list");
  const [templates, setTemplates] = useState<MethodologyTemplate[]>([]);
  const [projects, setProjects] = useState<ProjectMethodology[]>([]);
  const [activeProject, setActiveProject] = useState<ProjectMethodology | null>(null);
  const [expandedPhases, setExpandedPhases] = useState<Set<string>>(new Set());
  const [showNewProject, setShowNewProject] = useState(false);
  const [newProjectName, setNewProjectName] = useState("");
  const [newTemplateId, setNewTemplateId] = useState("");
  const [editingNotes, setEditingNotes] = useState<string | null>(null);

  const loadData = useCallback(async () => {
    try {
      const [t, p] = await Promise.all([
        invoke<MethodologyTemplate[]>("method_list_templates"),
        invoke<ProjectMethodology[]>("method_list_projects", { projectPath: getProjectPath() }),
      ]);
      setTemplates(t);
      setProjects(p);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData, currentProjectPath]);

  const handleCreateProject = useCallback(async () => {
    if (!newProjectName.trim() || !newTemplateId) return;
    try {
      const project = await invoke<ProjectMethodology>("method_start_project", {
        templateId: newTemplateId,
        projectName: newProjectName,
        projectPath: getProjectPath(),
      });
      setActiveProject(project);
      setView("project");
      setShowNewProject(false);
      setNewProjectName("");
      setNewTemplateId("");
      setExpandedPhases(new Set(project.phases.map((p) => p.id)));
      await loadData();
    } catch (e) {
      console.error("Create failed:", e);
    }
  }, [newProjectName, newTemplateId, loadData]);

  const handleOpenProject = useCallback(async (id: string) => {
    try {
      const project = await invoke<ProjectMethodology>("method_load_project", { id, projectPath: getProjectPath() });
      setActiveProject(project);
      setView("project");
      setExpandedPhases(new Set(project.phases.map((p) => p.id)));
    } catch (e) {
      console.error("Load failed:", e);
    }
  }, []);

  const handleDeleteProject = useCallback(
    async (id: string) => {
      if (!confirm(t("methodology.deleteConfirm", "Delete this project?"))) return;
      try {
        await invoke("method_delete_project", { id, projectPath: getProjectPath() });
        await loadData();
      } catch (e) {
        console.error("Delete failed:", e);
      }
    },
    [t, loadData]
  );

  const handleToggleItem = useCallback(
    async (phaseId: string, itemId: string, checked: boolean) => {
      if (!activeProject) return;
      try {
        await invoke("method_update_item", {
          projectId: activeProject.id,
          phaseId,
          itemId,
          checked,
          notes: null as string | null,
          projectPath: getProjectPath(),
        });
        setActiveProject((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            phases: prev.phases.map((p) =>
              p.id === phaseId
                ? {
                    ...p,
                    items: p.items.map((i) =>
                      i.id === itemId ? { ...i, checked } : i
                    ),
                  }
                : p
            ),
          };
        });
      } catch (e) {
        console.error("Update failed:", e);
      }
    },
    [activeProject]
  );

  const handleSaveNotes = useCallback(
    async (phaseId: string, itemId: string, notes: string) => {
      if (!activeProject) return;
      try {
        await invoke("method_update_item", {
          projectId: activeProject.id,
          phaseId,
          itemId,
          checked: null as boolean | null,
          notes,
          projectPath: getProjectPath(),
        });
        setActiveProject((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            phases: prev.phases.map((p) =>
              p.id === phaseId
                ? {
                    ...p,
                    items: p.items.map((i) =>
                      i.id === itemId ? { ...i, notes } : i
                    ),
                  }
                : p
            ),
          };
        });
        setEditingNotes(null);
      } catch (e) {
        console.error("Update notes failed:", e);
      }
    },
    [activeProject]
  );

  const togglePhase = useCallback((phaseId: string) => {
    setExpandedPhases((prev) => {
      const next = new Set(prev);
      if (next.has(phaseId)) next.delete(phaseId);
      else next.add(phaseId);
      return next;
    });
  }, []);

  const progress = useMemo(() => {
    if (!activeProject) return { total: 0, checked: 0, percent: 0 };
    let total = 0;
    let checked = 0;
    for (const phase of activeProject.phases) {
      for (const item of phase.items) {
        total++;
        if (item.checked) checked++;
      }
    }
    return { total, checked, percent: total > 0 ? Math.round((checked / total) * 100) : 0 };
  }, [activeProject]);

  // Project detail view
  if (view === "project" && activeProject) {
    return (
      <div className="flex flex-col h-full">
        {/* Header */}
        <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/30 flex-shrink-0">
          <button
            className="p-1 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => {
              setView("list");
              setActiveProject(null);
            }}
          >
            <ArrowLeft className="w-4 h-4" />
          </button>
          <ClipboardList className="w-4 h-4 text-accent" />
          <span className="text-sm font-medium truncate flex-1">
            {activeProject.project_name}
          </span>
          <span className="text-[10px] text-muted-foreground px-2 py-0.5 rounded bg-muted/50">
            {activeProject.template_name}
          </span>
        </div>

        {/* Progress bar */}
        <div className="px-4 py-2 border-b border-border/20 flex items-center gap-3 flex-shrink-0">
          <div className="flex-1 h-1.5 rounded-full bg-muted/50 overflow-hidden">
            <div
              className="h-full rounded-full bg-accent transition-all duration-500 ease-out"
              style={{ width: `${progress.percent}%` }}
            />
          </div>
          <span className="text-[10px] text-muted-foreground whitespace-nowrap">
            {progress.checked}/{progress.total} ({progress.percent}%)
          </span>
        </div>

        {/* Phases */}
        <div className="flex-1 overflow-y-auto px-2 py-2 space-y-1">
          {activeProject.phases.map((phase) => {
            const isExpanded = expandedPhases.has(phase.id);
            const phaseChecked = phase.items.filter((i) => i.checked).length;
            const phaseTotal = phase.items.length;
            return (
              <div key={phase.id} className="rounded-lg border border-border/20 overflow-hidden">
                <button
                  className="w-full flex items-center gap-2 px-3 py-2 hover:bg-muted/30 transition-colors text-left"
                  onClick={() => togglePhase(phase.id)}
                >
                  {isExpanded ? (
                    <ChevronDown className="w-3.5 h-3.5 text-muted-foreground flex-shrink-0" />
                  ) : (
                    <ChevronRight className="w-3.5 h-3.5 text-muted-foreground flex-shrink-0" />
                  )}
                  <span className="text-xs font-medium flex-1 truncate">{phase.name}</span>
                  <span className="text-[10px] text-muted-foreground">
                    {phaseChecked}/{phaseTotal}
                  </span>
                  {phaseChecked === phaseTotal && phaseTotal > 0 && (
                    <CheckCircle2 className="w-3.5 h-3.5 text-green-400 flex-shrink-0" />
                  )}
                </button>

                {isExpanded && (
                  <div className="border-t border-border/20 px-2 py-1">
                    {phase.items.map((item) => (
                      <div key={item.id} className="py-1.5 px-1">
                        <div className="flex items-start gap-2">
                          <button
                            className="mt-0.5 flex-shrink-0"
                            onClick={() =>
                              handleToggleItem(phase.id, item.id, !item.checked)
                            }
                          >
                            {item.checked ? (
                              <Check className="w-3.5 h-3.5 text-green-400" />
                            ) : (
                              <Circle className="w-3.5 h-3.5 text-muted-foreground/50" />
                            )}
                          </button>
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-1.5">
                              <span
                                className={`text-xs ${
                                  item.checked
                                    ? "line-through text-muted-foreground/60"
                                    : ""
                                }`}
                              >
                                {item.title}
                              </span>
                            </div>
                            <p className="text-[10px] text-muted-foreground/70 mt-0.5">
                              {item.description}
                            </p>
                            {item.tools.length > 0 && (
                              <div className="flex items-center gap-1 mt-1 flex-wrap">
                                <Wrench className="w-2.5 h-2.5 text-muted-foreground/50" />
                                {item.tools.map((tool) => (
                                  <span
                                    key={tool}
                                    className="text-[9px] px-1.5 py-0.5 rounded bg-accent/10 text-accent/70"
                                  >
                                    {tool}
                                  </span>
                                ))}
                              </div>
                            )}
                            {/* Notes */}
                            {editingNotes === item.id ? (
                              <div className="mt-1.5">
                                <textarea
                                  className="w-full text-[10px] p-1.5 rounded bg-background border border-border/50 focus:border-accent outline-none resize-none"
                                  rows={2}
                                  defaultValue={item.notes}
                                  autoFocus
                                  onBlur={(e) =>
                                    handleSaveNotes(phase.id, item.id, e.target.value)
                                  }
                                  onKeyDown={(e) => {
                                    if (e.key === "Escape") setEditingNotes(null);
                                  }}
                                />
                              </div>
                            ) : item.notes ? (
                              <div
                                className="mt-1 text-[10px] text-muted-foreground/80 bg-muted/30 rounded px-2 py-1 cursor-pointer hover:bg-muted/50 transition-colors"
                                onClick={() => setEditingNotes(item.id)}
                              >
                                {item.notes}
                              </div>
                            ) : (
                              <button
                                className="mt-1 flex items-center gap-1 text-[10px] text-muted-foreground/40 hover:text-muted-foreground/70 transition-colors"
                                onClick={() => setEditingNotes(item.id)}
                              >
                                <MessageSquare className="w-2.5 h-2.5" />
                                Add notes
                              </button>
                            )}
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>
    );
  }

  // List view (templates + projects)
  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/30 flex-shrink-0">
        <ClipboardList className="w-4 h-4 text-accent" />
        <span className="text-sm font-medium">
          {t("methodology.title", "Methodology")}
        </span>
        <div className="flex-1" />
        <button
          className="flex items-center gap-1 px-2 py-1 text-xs rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => setShowNewProject(true)}
        >
          <Plus className="w-3.5 h-3.5" />
          {t("methodology.newProject", "New")}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto px-3 py-3 space-y-4">
        {/* Existing projects */}
        {projects.length > 0 && (
          <div>
            <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-medium mb-2">
              {t("methodology.projects", "Projects")}
            </div>
            <div className="space-y-1.5">
              {projects.map((p) => {
                const total = p.phases.reduce((a, ph) => a + ph.items.length, 0);
                const checked = p.phases.reduce(
                  (a, ph) => a + ph.items.filter((i) => i.checked).length,
                  0
                );
                const pct = total > 0 ? Math.round((checked / total) * 100) : 0;
                return (
                  <div
                    key={p.id}
                    className="group flex items-center gap-2 px-3 py-2 rounded-lg border border-border/20 hover:border-border/40 cursor-pointer transition-colors"
                    onClick={() => handleOpenProject(p.id)}
                  >
                    <ClipboardList className="w-4 h-4 text-accent/70 flex-shrink-0" />
                    <div className="flex-1 min-w-0">
                      <div className="text-xs font-medium truncate">{p.project_name}</div>
                      <div className="flex items-center gap-2 mt-0.5">
                        <span className="text-[10px] text-muted-foreground">
                          {p.template_name}
                        </span>
                        <div className="flex-1 h-1 rounded-full bg-muted/50 max-w-[80px]">
                          <div
                            className="h-full rounded-full bg-accent"
                            style={{ width: `${pct}%` }}
                          />
                        </div>
                        <span className="text-[10px] text-muted-foreground">
                          {pct}%
                        </span>
                      </div>
                    </div>
                    <button
                      className="opacity-0 group-hover:opacity-100 p-1 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-opacity"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDeleteProject(p.id);
                      }}
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Templates */}
        <div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-medium mb-2">
            {t("methodology.templates", "Templates")}
          </div>
          <div className="space-y-1.5">
            {templates.map((tmpl) => (
              <div
                key={tmpl.id}
                className="px-3 py-2 rounded-lg border border-border/20 hover:border-border/40 transition-colors"
              >
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium">{tmpl.name}</span>
                  <span className="text-[10px] text-muted-foreground">
                    ({tmpl.phases.length} phases)
                  </span>
                </div>
                <p className="text-[10px] text-muted-foreground/70 mt-0.5">
                  {tmpl.description}
                </p>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* New project dialog */}
      {showNewProject && (
        <div className="absolute inset-0 z-30 bg-background/80 backdrop-blur-sm flex items-center justify-center">
          <div className="w-80 bg-card rounded-xl border border-border/50 shadow-2xl">
            <div className="px-4 py-3 border-b border-border/30 text-sm font-medium">
              {t("methodology.createProject", "Create Methodology Project")}
            </div>
            <div className="p-4 space-y-3">
              <input
                className="w-full text-xs px-3 py-2 rounded-md bg-background border border-border/50 focus:border-accent outline-none"
                placeholder={t("methodology.projectName", "Project name...")}
                value={newProjectName}
                onChange={(e) => setNewProjectName(e.target.value)}
                autoFocus
              />
              <select
                className="w-full text-xs px-3 py-2 rounded-md bg-background border border-border/50 focus:border-accent outline-none"
                value={newTemplateId}
                onChange={(e) => setNewTemplateId(e.target.value)}
              >
                <option value="">
                  {t("methodology.selectTemplate", "Select template...")}
                </option>
                {templates.map((tmpl) => (
                  <option key={tmpl.id} value={tmpl.id}>
                    {tmpl.name} - {tmpl.description}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex justify-end gap-2 px-4 py-3 border-t border-border/30">
              <button
                className="px-3 py-1.5 text-xs rounded-md hover:bg-muted/50 transition-colors"
                onClick={() => {
                  setShowNewProject(false);
                  setNewProjectName("");
                  setNewTemplateId("");
                }}
              >
                {t("common.cancel", "Cancel")}
              </button>
              <button
                className="px-3 py-1.5 text-xs rounded-md bg-accent text-accent-foreground hover:bg-accent/90 transition-colors font-medium"
                onClick={handleCreateProject}
                disabled={!newProjectName.trim() || !newTemplateId}
              >
                {t("common.create", "Create")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
