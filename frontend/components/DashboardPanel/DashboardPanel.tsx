import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Bug,
  CheckCircle2,
  Circle,
  Crosshair,
  KeyRound,
  Layers,
  Network,
  Shield,
  Target,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import { getProjectPath } from "@/lib/projects";

interface TargetStore {
  targets: { id: string; scope: string; group: string }[];
  groups: string[];
}

interface MethodPhase {
  id: string;
  name: string;
  items: { id: string; checked: boolean }[];
}

interface ProjectMethodology {
  id: string;
  project_name: string;
  template_id: string;
  phases: MethodPhase[];
  created_at: string;
  updated_at: string;
}

interface VaultEntry {
  id: string;
  name: string;
  entry_type: string;
}

interface FindingsStore {
  findings: { id: string; severity: string; status: string }[];
}

interface DashboardStats {
  targetsTotal: number;
  targetsInScope: number;
  targetsOutScope: number;
  targetGroups: number;
  methodProjects: ProjectMethodology[];
  topoMaps: string[];
  vaultEntries: number;
  vaultByType: Record<string, number>;
  findingsTotal: number;
  findingsBySeverity: Record<string, number>;
  findingsOpen: number;
}

function StatCard({
  icon: Icon,
  label,
  value,
  sub,
  color,
}: {
  icon: typeof Activity;
  label: string;
  value: number | string;
  sub?: string;
  color: string;
}) {
  return (
    <div className="flex items-center gap-3 px-4 py-3 rounded-lg bg-muted/20 border border-border/10">
      <div className={cn("p-2 rounded-md", color)}>
        <Icon className="w-4 h-4" />
      </div>
      <div className="min-w-0">
        <div className="text-lg font-semibold leading-tight">{value}</div>
        <div className="text-[11px] text-muted-foreground/60 truncate">{label}</div>
        {sub && <div className="text-[10px] text-muted-foreground/40 truncate">{sub}</div>}
      </div>
    </div>
  );
}

function MethodologyProgress({ project }: { project: ProjectMethodology }) {
  const totalItems = project.phases.reduce((acc, p) => acc + p.items.length, 0);
  const checkedItems = project.phases.reduce(
    (acc, p) => acc + p.items.filter((i) => i.checked).length,
    0,
  );
  const pct = totalItems > 0 ? Math.round((checkedItems / totalItems) * 100) : 0;

  return (
    <div className="px-3 py-2.5 rounded-lg bg-muted/10 border border-border/10 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium truncate">{project.project_name}</span>
        <span className="text-[10px] text-muted-foreground/50">
          {checkedItems}/{totalItems}
        </span>
      </div>
      <div className="w-full h-1.5 rounded-full bg-muted/30 overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-500 ease-out",
            pct === 100 ? "bg-green-500/80" : pct > 50 ? "bg-accent/70" : "bg-amber-500/60",
          )}
          style={{ width: `${pct}%` }}
        />
      </div>
      <div className="flex gap-1 flex-wrap">
        {project.phases.map((phase) => {
          const done = phase.items.filter((i) => i.checked).length;
          const total = phase.items.length;
          const allDone = done === total && total > 0;
          return (
            <div
              key={phase.id}
              className={cn(
                "flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px]",
                allDone
                  ? "bg-green-500/10 text-green-400/80"
                  : "bg-muted/20 text-muted-foreground/50",
              )}
            >
              {allDone ? (
                <CheckCircle2 className="w-2.5 h-2.5" />
              ) : (
                <Circle className="w-2.5 h-2.5" />
              )}
              <span className="truncate max-w-[80px]">{phase.name}</span>
              <span>
                {done}/{total}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function DashboardPanel() {
  const { t } = useTranslation();
  const currentProjectName = useStore((s) => s.currentProjectName);
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [stats, setStats] = useState<DashboardStats>({
    targetsTotal: 0,
    targetsInScope: 0,
    targetsOutScope: 0,
    targetGroups: 0,
    methodProjects: [],
    topoMaps: [],
    vaultEntries: 0,
    vaultByType: {},
    findingsTotal: 0,
    findingsBySeverity: {},
    findingsOpen: 0,
  });
  const [loading, setLoading] = useState(true);

  const loadStats = useCallback(async () => {
    setLoading(true);
    const pp = getProjectPath();

    const results = await Promise.allSettled([
      invoke<TargetStore>("target_list", { projectPath: pp }),
      invoke<ProjectMethodology[]>("method_list_projects", { projectPath: pp }),
      invoke<string[]>("topo_list", { projectPath: pp }),
      invoke<VaultEntry[]>("vault_list", { projectPath: pp }),
      invoke<FindingsStore>("findings_list", { projectPath: pp }),
    ]);

    const targetData =
      results[0].status === "fulfilled" ? results[0].value : { targets: [], groups: [] };
    const methodData = results[1].status === "fulfilled" ? results[1].value : [];
    const topoData = results[2].status === "fulfilled" ? results[2].value : [];
    const vaultData = results[3].status === "fulfilled" ? results[3].value : [];
    const findingsData =
      results[4].status === "fulfilled" ? results[4].value : { findings: [] };

    const vaultByType: Record<string, number> = {};
    for (const e of vaultData) {
      vaultByType[e.entry_type] = (vaultByType[e.entry_type] || 0) + 1;
    }

    const findingsBySeverity: Record<string, number> = {};
    let findingsOpen = 0;
    for (const f of findingsData.findings) {
      findingsBySeverity[f.severity] = (findingsBySeverity[f.severity] || 0) + 1;
      if (f.status === "open" || f.status === "confirmed") findingsOpen++;
    }

    setStats({
      targetsTotal: targetData.targets.length,
      targetsInScope: targetData.targets.filter((t) => t.scope === "in").length,
      targetsOutScope: targetData.targets.filter((t) => t.scope === "out").length,
      targetGroups: targetData.groups.length,
      methodProjects: methodData,
      topoMaps: topoData,
      vaultEntries: vaultData.length,
      vaultByType,
      findingsTotal: findingsData.findings.length,
      findingsBySeverity,
      findingsOpen,
    });
    setLoading(false);
  }, []);

  useEffect(() => {
    loadStats();
  }, [loadStats, currentProjectPath]);

  const overallProgress = useMemo(() => {
    if (stats.methodProjects.length === 0) return null;
    let total = 0;
    let checked = 0;
    for (const p of stats.methodProjects) {
      for (const phase of p.phases) {
        total += phase.items.length;
        checked += phase.items.filter((i) => i.checked).length;
      }
    }
    return total > 0 ? Math.round((checked / total) * 100) : 0;
  }, [stats.methodProjects]);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="animate-pulse text-muted-foreground/40 text-sm">Loading...</div>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-card rounded-xl">
      <div className="flex items-center justify-between px-4 py-3 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-2">
          <Layers className="w-4 h-4 text-accent/70" />
          <span className="text-sm font-medium">
            {t("dashboard.title", "Project Dashboard")}
          </span>
        </div>
        {currentProjectName && (
          <span className="text-[11px] text-muted-foreground/50 truncate max-w-[200px]">
            {currentProjectName}
          </span>
        )}
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-6">
        {/* Stats grid */}
        <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
          <StatCard
            icon={Target}
            label={t("dashboard.targets", "Targets")}
            value={stats.targetsTotal}
            sub={`${stats.targetsInScope} in / ${stats.targetsOutScope} out`}
            color="bg-blue-500/10 text-blue-400/80"
          />
          <StatCard
            icon={Crosshair}
            label={t("dashboard.groups", "Groups")}
            value={stats.targetGroups}
            color="bg-purple-500/10 text-purple-400/80"
          />
          <StatCard
            icon={Network}
            label={t("dashboard.topoMaps", "Topology Maps")}
            value={stats.topoMaps.length}
            color="bg-emerald-500/10 text-emerald-400/80"
          />
          <StatCard
            icon={Bug}
            label={t("dashboard.findings", "Findings")}
            value={stats.findingsTotal}
            sub={stats.findingsOpen > 0 ? `${stats.findingsOpen} open` : undefined}
            color="bg-red-500/10 text-red-400/80"
          />
          <StatCard
            icon={KeyRound}
            label={t("dashboard.credentials", "Credentials")}
            value={stats.vaultEntries}
            sub={Object.entries(stats.vaultByType)
              .map(([k, v]) => `${v} ${k}`)
              .join(", ")}
            color="bg-amber-500/10 text-amber-400/80"
          />
        </div>

        {/* Findings severity breakdown */}
        {stats.findingsTotal > 0 && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Bug className="w-3.5 h-3.5 text-muted-foreground/50" />
              <span className="text-xs font-medium text-muted-foreground/70">
                {t("dashboard.findingsSeverity", "Findings by Severity")}
              </span>
            </div>
            <div className="flex gap-2">
              {(["critical", "high", "medium", "low", "info"] as const).map((sev) => {
                const count = stats.findingsBySeverity[sev] || 0;
                if (count === 0) return null;
                const colors: Record<string, string> = {
                  critical: "bg-red-500/15 text-red-400 border-red-500/20",
                  high: "bg-orange-500/15 text-orange-400 border-orange-500/20",
                  medium: "bg-yellow-500/15 text-yellow-400 border-yellow-500/20",
                  low: "bg-blue-500/15 text-blue-400 border-blue-500/20",
                  info: "bg-slate-500/15 text-slate-400 border-slate-500/20",
                };
                return (
                  <div key={sev} className={cn("px-2.5 py-1 rounded-md border text-[10px] font-medium", colors[sev])}>
                    {count} {sev}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Overall progress */}
        {overallProgress !== null && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Shield className="w-3.5 h-3.5 text-accent/60" />
              <span className="text-xs font-medium text-muted-foreground/70">
                {t("dashboard.overallProgress", "Overall Methodology Progress")}
              </span>
              <span className="text-xs font-semibold text-accent/80 ml-auto">{overallProgress}%</span>
            </div>
            <div className="w-full h-2 rounded-full bg-muted/20 overflow-hidden">
              <div
                className={cn(
                  "h-full rounded-full transition-all duration-700 ease-out",
                  overallProgress === 100
                    ? "bg-green-500/80"
                    : overallProgress > 50
                      ? "bg-accent/70"
                      : "bg-amber-500/60",
                )}
                style={{ width: `${overallProgress}%` }}
              />
            </div>
          </div>
        )}

        {/* Methodology details */}
        {stats.methodProjects.length > 0 && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Activity className="w-3.5 h-3.5 text-muted-foreground/50" />
              <span className="text-xs font-medium text-muted-foreground/70">
                {t("dashboard.methodologies", "Methodology Projects")}
              </span>
            </div>
            <div className="space-y-2">
              {stats.methodProjects.map((mp) => (
                <MethodologyProgress key={mp.id} project={mp} />
              ))}
            </div>
          </div>
        )}

        {/* Topology maps list */}
        {stats.topoMaps.length > 0 && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Network className="w-3.5 h-3.5 text-muted-foreground/50" />
              <span className="text-xs font-medium text-muted-foreground/70">
                {t("dashboard.savedTopologies", "Saved Topologies")}
              </span>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {stats.topoMaps.map((name) => (
                <span
                  key={name}
                  className="px-2 py-1 rounded-md bg-muted/15 text-[10px] text-muted-foreground/60 border border-border/10"
                >
                  {name}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Empty state */}
        {stats.targetsTotal === 0 &&
          stats.methodProjects.length === 0 &&
          stats.topoMaps.length === 0 &&
          stats.vaultEntries === 0 &&
          stats.findingsTotal === 0 && (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <Layers className="w-10 h-10 text-muted-foreground/20 mb-3" />
              <p className="text-sm text-muted-foreground/40">
                {t("dashboard.empty", "No project data yet")}
              </p>
              <p className="text-[11px] text-muted-foreground/30 mt-1">
                {t(
                  "dashboard.emptyHint",
                  "Add targets, run scans, or start a methodology to see stats here",
                )}
              </p>
            </div>
          )}
      </div>
    </div>
  );
}
