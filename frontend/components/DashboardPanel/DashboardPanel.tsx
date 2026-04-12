import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

interface Finding {
  id: string;
  severity: string;
  status: string;
  created_at: string;
  tool?: string;
}

interface FindingsStore {
  findings: Finding[];
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
  findingsByTool: Record<string, number>;
  findingsTimeline: { date: string; count: number }[];
}

const SEV_COLORS: Record<string, string> = {
  critical: "#ef4444",
  high: "#f97316",
  medium: "#eab308",
  low: "#3b82f6",
  info: "#64748b",
};

function DonutChart({ data, size = 120 }: { data: { label: string; value: number; color: string }[]; size?: number }) {
  const total = data.reduce((s, d) => s + d.value, 0);
  if (total === 0) return null;
  const cx = size / 2;
  const cy = size / 2;
  const r = (size - 16) / 2;
  const innerR = r * 0.6;
  let cumulative = 0;

  const arcs = data.filter((d) => d.value > 0).map((d) => {
    const startAngle = (cumulative / total) * 2 * Math.PI - Math.PI / 2;
    cumulative += d.value;
    const endAngle = (cumulative / total) * 2 * Math.PI - Math.PI / 2;
    const largeArc = d.value / total > 0.5 ? 1 : 0;
    const x1 = cx + r * Math.cos(startAngle);
    const y1 = cy + r * Math.sin(startAngle);
    const x2 = cx + r * Math.cos(endAngle);
    const y2 = cy + r * Math.sin(endAngle);
    const ix1 = cx + innerR * Math.cos(endAngle);
    const iy1 = cy + innerR * Math.sin(endAngle);
    const ix2 = cx + innerR * Math.cos(startAngle);
    const iy2 = cy + innerR * Math.sin(startAngle);
    const path = [
      `M ${x1} ${y1}`,
      `A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2}`,
      `L ${ix1} ${iy1}`,
      `A ${innerR} ${innerR} 0 ${largeArc} 0 ${ix2} ${iy2}`,
      "Z",
    ].join(" ");
    return { ...d, path };
  });

  return (
    <div className="flex items-center gap-3">
      <svg width={size} height={size}>
        {arcs.map((arc, i) => (
          <path key={i} d={arc.path} fill={arc.color} opacity={0.8}>
            <title>{arc.label}: {arc.value}</title>
          </path>
        ))}
        <text x={cx} y={cy - 4} textAnchor="middle" className="fill-foreground text-lg font-bold">{total}</text>
        <text x={cx} y={cy + 10} textAnchor="middle" className="fill-muted-foreground/50 text-[9px]">total</text>
      </svg>
      <div className="space-y-1">
        {data.filter((d) => d.value > 0).map((d) => (
          <div key={d.label} className="flex items-center gap-2 text-[10px]">
            <div className="w-2 h-2 rounded-full" style={{ backgroundColor: d.color }} />
            <span className="text-muted-foreground/60 capitalize">{d.label}</span>
            <span className="text-foreground/80 font-medium">{d.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function BarChart({ data, height = 80 }: { data: { label: string; value: number }[]; height?: number }) {
  const max = Math.max(...data.map((d) => d.value), 1);
  if (data.length === 0) return null;

  return (
    <div className="flex items-end gap-1" style={{ height }}>
      {data.map((d) => {
        const barH = Math.max((d.value / max) * (height - 16), 2);
        return (
          <div key={d.label} className="flex-1 flex flex-col items-center gap-0.5 min-w-0">
            <span className="text-[8px] text-muted-foreground/50">{d.value > 0 ? d.value : ""}</span>
            <div
              className="w-full rounded-t bg-accent/40 transition-all duration-500"
              style={{ height: barH }}
              title={`${d.label}: ${d.value}`}
            />
            <span className="text-[7px] text-muted-foreground/40 truncate w-full text-center">{d.label}</span>
          </div>
        );
      })}
    </div>
  );
}

function MiniTimeline({ data }: { data: { date: string; count: number }[] }) {
  if (data.length === 0) return null;
  const max = Math.max(...data.map((d) => d.count), 1);
  const w = 280;
  const h = 60;
  const padY = 4;
  const usableH = h - padY * 2;
  const stepX = data.length > 1 ? w / (data.length - 1) : w / 2;

  const points = data.map((d, i) => ({
    x: i * stepX,
    y: padY + usableH - (d.count / max) * usableH,
  }));

  const line = points.map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`).join(" ");
  const area = line + ` L ${points[points.length - 1].x} ${h} L 0 ${h} Z`;

  return (
    <svg width={w} height={h} className="overflow-visible">
      <defs>
        <linearGradient id="timeline-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.2" />
          <stop offset="100%" stopColor="var(--accent)" stopOpacity="0.02" />
        </linearGradient>
      </defs>
      <path d={area} fill="url(#timeline-fill)" />
      <path d={line} fill="none" stroke="var(--accent)" strokeWidth="1.5" opacity="0.6" />
      {points.map((p, i) => (
        <circle key={i} cx={p.x} cy={p.y} r={2} fill="var(--accent)" opacity="0.8">
          <title>{data[i].date}: {data[i].count}</title>
        </circle>
      ))}
    </svg>
  );
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
    findingsByTool: {},
    findingsTimeline: [],
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

    const targetRaw = results[0].status === "fulfilled" ? results[0].value : null;
    const targetData = targetRaw && targetRaw.targets ? targetRaw : { targets: [], groups: [] };
    const methodRaw = results[1].status === "fulfilled" ? results[1].value : [];
    const methodData = Array.isArray(methodRaw) ? methodRaw : [];
    const topoRaw = results[2].status === "fulfilled" ? results[2].value : [];
    const topoData = Array.isArray(topoRaw) ? topoRaw : [];
    const vaultRaw = results[3].status === "fulfilled" ? results[3].value : [];
    const vaultData = Array.isArray(vaultRaw) ? vaultRaw : [];
    const findingsRaw = results[4].status === "fulfilled" ? results[4].value : null;
    const findingsData = findingsRaw && findingsRaw.findings ? findingsRaw : { findings: [] };

    const vaultByType: Record<string, number> = {};
    for (const e of vaultData) {
      vaultByType[e.entry_type] = (vaultByType[e.entry_type] || 0) + 1;
    }

    const findingsBySeverity: Record<string, number> = {};
    const findingsByTool: Record<string, number> = {};
    const dateMap: Record<string, number> = {};
    let findingsOpen = 0;
    for (const f of findingsData.findings) {
      findingsBySeverity[f.severity] = (findingsBySeverity[f.severity] || 0) + 1;
      if (f.status === "open" || f.status === "confirmed") findingsOpen++;
      if (f.tool) findingsByTool[f.tool] = (findingsByTool[f.tool] || 0) + 1;
      if (f.created_at) {
        const day = f.created_at.slice(0, 10);
        dateMap[day] = (dateMap[day] || 0) + 1;
      }
    }
    const findingsTimeline = Object.entries(dateMap)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([date, count]) => ({ date, count }));

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
      findingsByTool,
      findingsTimeline,
    });
    setLoading(false);
  }, []);

  useEffect(() => {
    loadStats();
  }, [loadStats, currentProjectPath]);

  useEffect(() => {
    const REFRESH_TOOLS = new Set(["manage_targets", "record_finding", "credential_vault"]);
    const unlisten = listen<{ type: string; tool_name?: string }>("ai-event", (event) => {
      if (event.payload.type === "tool_result" && event.payload.tool_name && REFRESH_TOOLS.has(event.payload.tool_name)) {
        loadStats();
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [loadStats]);

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

        {/* Findings charts row */}
        {stats.findingsTotal > 0 && (
          <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
            {/* Severity donut */}
            <div className="rounded-lg bg-muted/10 border border-border/10 p-3 space-y-2">
              <div className="flex items-center gap-2">
                <Bug className="w-3.5 h-3.5 text-muted-foreground/50" />
                <span className="text-[11px] font-medium text-muted-foreground/70">Severity Distribution</span>
              </div>
              <DonutChart
                data={["critical", "high", "medium", "low", "info"].map((sev) => ({
                  label: sev,
                  value: stats.findingsBySeverity[sev] || 0,
                  color: SEV_COLORS[sev],
                }))}
              />
            </div>

            {/* Findings by tool */}
            {Object.keys(stats.findingsByTool).length > 0 && (
              <div className="rounded-lg bg-muted/10 border border-border/10 p-3 space-y-2">
                <div className="flex items-center gap-2">
                  <Crosshair className="w-3.5 h-3.5 text-muted-foreground/50" />
                  <span className="text-[11px] font-medium text-muted-foreground/70">Findings by Tool</span>
                </div>
                <BarChart
                  data={Object.entries(stats.findingsByTool)
                    .sort(([, a], [, b]) => b - a)
                    .slice(0, 8)
                    .map(([label, value]) => ({ label, value }))}
                />
              </div>
            )}

            {/* Findings timeline */}
            {stats.findingsTimeline.length > 1 && (
              <div className="rounded-lg bg-muted/10 border border-border/10 p-3 space-y-2">
                <div className="flex items-center gap-2">
                  <Activity className="w-3.5 h-3.5 text-muted-foreground/50" />
                  <span className="text-[11px] font-medium text-muted-foreground/70">Findings Timeline</span>
                </div>
                <MiniTimeline data={stats.findingsTimeline} />
                <div className="flex justify-between text-[8px] text-muted-foreground/30">
                  <span>{stats.findingsTimeline[0].date}</span>
                  <span>{stats.findingsTimeline[stats.findingsTimeline.length - 1].date}</span>
                </div>
              </div>
            )}
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
