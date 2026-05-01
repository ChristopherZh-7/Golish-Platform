import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  dashboardApi,
  type TokenUsageStats, type AgentUsage, type ToolCallStat,
  type ProjectMethodology, type AuditEntry,
} from "@/lib/dashboard";
import { findingsApi, type Finding } from "@/lib/findings";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import {
  Activity,
  AlertTriangle,
  ArrowRight,
  BarChart3,
  Bot,
  Brain,
  Bug,
  CheckCircle2,
  Circle,
  Clock,
  Cpu,
  DollarSign,
  Globe,
  KeyRound,
  Layers,
  Monitor,
  Radio,
  Shield,
  ShieldAlert,
  ShieldCheck,
  Target,
  Wifi,
  Wrench,
} from "lucide-react";
import { formatRelativeTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import { getProjectPath } from "@/lib/projects";
import { type Target as PentestTarget } from "@/lib/pentest/types";
import {
  fmtNum, SeverityBar, MiniTimeline, MethodologyRing, MetricCard, SevIcon,
} from "./StatCards";
import { eventLabel, ActivityDot, AgentUsageChart, ToolCallChart } from "./ActivityFeed";

interface AiStats {
  tokenUsage: TokenUsageStats | null;
  agentUsage: AgentUsage[];
  toolCallStats: ToolCallStat[];
  memoryCount: number;
}

interface DashboardStats {
  targets: PentestTarget[];
  methodProjects: ProjectMethodology[];
  vaultEntries: { entry_type: string }[];
  findings: Finding[];
  recentActivity: AuditEntry[];
}

const TYPE_ICON: Record<string, typeof Globe> = {
  domain: Globe,
  ip: Monitor,
  cidr: Wifi,
  url: ArrowRight,
  wildcard: Radio,
};

export function DashboardPanel() {
  const { t } = useTranslation();
  const currentProjectName = useStore((s) => s.currentProjectName);
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [stats, setStats] = useState<DashboardStats>({
    targets: [],
    methodProjects: [],
    vaultEntries: [],
    findings: [],
    recentActivity: [],
  });
  const [loading, setLoading] = useState(true);
  const [aiStats, setAiStats] = useState<AiStats>({
    tokenUsage: null,
    agentUsage: [],
    toolCallStats: [],
    memoryCount: 0,
  });

  const loadAiStats = useCallback(async () => {
    const aiResults = await Promise.allSettled([
      dashboardApi.getTokenUsageStats(),
      dashboardApi.getUsageByAgent(),
      dashboardApi.getToolCallStats(),
      dashboardApi.getMemoryCount(),
    ]);

    setAiStats({
      tokenUsage: aiResults[0].status === "fulfilled" ? aiResults[0].value : null,
      agentUsage: aiResults[1].status === "fulfilled" ? aiResults[1].value : [],
      toolCallStats: aiResults[2].status === "fulfilled" ? aiResults[2].value : [],
      memoryCount: aiResults[3].status === "fulfilled" ? aiResults[3].value : 0,
    });
  }, []);

  const loadStats = useCallback(async () => {
    setLoading(true);
    const pp = getProjectPath();
    if (!pp) { setLoading(false); return; }

    const results = await Promise.allSettled([
      dashboardApi.targetList(pp),
      dashboardApi.methodListProjects(pp),
      dashboardApi.vaultList(pp),
      findingsApi.list(pp),
      dashboardApi.oplogList(pp, 15),
    ]);

    const targetRaw = results[0].status === "fulfilled" ? results[0].value : null;
    const targets = targetRaw?.targets ?? [];
    const methodRaw = results[1].status === "fulfilled" ? results[1].value : [];
    const vaultRaw = results[2].status === "fulfilled" ? results[2].value : [];
    const findingsRaw = results[3].status === "fulfilled" ? results[3].value : null;
    const activityRaw = results[4].status === "fulfilled" ? results[4].value : [];

    setStats({
      targets: Array.isArray(targets) ? targets : [],
      methodProjects: Array.isArray(methodRaw) ? methodRaw : [],
      vaultEntries: Array.isArray(vaultRaw) ? vaultRaw : [],
      findings: findingsRaw?.findings ?? [],
      recentActivity: Array.isArray(activityRaw) ? activityRaw.slice(0, 10) : [],
    });
    setLoading(false);
  }, []);

  useEffect(() => { loadStats(); loadAiStats(); }, [loadStats, loadAiStats, currentProjectPath]);

  useEffect(() => {
    const REFRESH = new Set(["manage_targets", "record_finding", "credential_vault"]);
    const unlisten = listen<{ type: string; tool_name?: string }>("ai-event", (event) => {
      if (event.payload.type === "tool_result") {
        if (event.payload.tool_name && REFRESH.has(event.payload.tool_name)) {
          loadStats();
        }
        loadAiStats();
      }
    });
    return () => { runTauriUnlistenFromPromise(unlisten); };
  }, [loadStats]);

  const derived = useMemo(() => {
    const { targets, findings, vaultEntries } = stats;

    const inScope = targets.filter((t) => t.scope === "in").length;
    const outScope = targets.filter((t) => t.scope === "out").length;
    const byType: Record<string, number> = {};
    let totalPorts = 0;
    for (const t of targets) {
      byType[t.type] = (byType[t.type] || 0) + 1;
      if (t.ports) totalPorts += t.ports.length;
    }

    const findingsBySev: Record<string, number> = {};
    const findingsByTool: Record<string, number> = {};
    const dateMap: Record<string, number> = {};
    let openCount = 0;
    for (const f of findings) {
      findingsBySev[f.severity] = (findingsBySev[f.severity] || 0) + 1;
      if (f.status === "open" || f.status === "confirmed") openCount++;
      if (f.tool) findingsByTool[f.tool] = (findingsByTool[f.tool] || 0) + 1;
      if (f.created_at) {
        const day = new Date(f.created_at * 1000).toISOString().slice(0, 10);
        dateMap[day] = (dateMap[day] || 0) + 1;
      }
    }
    const timeline = Object.entries(dateMap)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([date, count]) => ({ date, count }));

    const vaultByType: Record<string, number> = {};
    for (const e of vaultEntries) {
      vaultByType[e.entry_type] = (vaultByType[e.entry_type] || 0) + 1;
    }

    return {
      inScope, outScope, byType, totalPorts,
      techCount: 0,
      findingsBySev, findingsByTool, timeline, openCount,
      vaultByType,
    };
  }, [stats]);

  const isEmpty =
    stats.targets.length === 0 &&
    stats.findings.length === 0 &&
    stats.vaultEntries.length === 0 &&
    stats.methodProjects.length === 0;

  const hasAiData =
    (aiStats.tokenUsage != null &&
      (aiStats.tokenUsage.total_tokens_in > 0 || aiStats.tokenUsage.total_tokens_out > 0)) ||
    aiStats.agentUsage.length > 0 ||
    aiStats.toolCallStats.length > 0;

  const totalTokens = aiStats.tokenUsage
    ? aiStats.tokenUsage.total_tokens_in + aiStats.tokenUsage.total_tokens_out
    : 0;
  const totalCost = aiStats.tokenUsage
    ? aiStats.tokenUsage.total_cost_in + aiStats.tokenUsage.total_cost_out
    : 0;
  const totalToolCalls = aiStats.toolCallStats.reduce((s, t) => s + t.total_count, 0);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="animate-pulse text-muted-foreground/30 text-sm">Loading...</div>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden bg-card rounded-xl">
      {/* Header */}
      <div className="flex items-center justify-between px-5 py-3.5 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-2.5">
          <div className="p-1.5 rounded-md bg-accent/10">
            <Layers className="w-4 h-4 text-accent/70" />
          </div>
          <div>
            <div className="text-sm font-semibold">{currentProjectName || t("dashboard.title", "Project Dashboard")}</div>
            {currentProjectName && (
              <div className="text-[10px] text-muted-foreground/40">{t("dashboard.title", "Project Dashboard")}</div>
            )}
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {isEmpty && !hasAiData ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-center px-8">
            <div className="p-4 rounded-2xl bg-muted/10 border border-border/10">
              <Layers className="w-8 h-8 text-muted-foreground/15" />
            </div>
            <p className="text-sm text-muted-foreground/40 font-medium">
              {t("dashboard.empty", "No project data yet")}
            </p>
            <p className="text-[11px] text-muted-foreground/25 max-w-[280px]">
              {t("dashboard.emptyHint", "Add targets, run scans, or start a methodology to see stats here")}
            </p>
          </div>
        ) : (
          <div className="p-5 space-y-5">
            {!isEmpty && (<>
            {/* Key Metrics Row */}
            <div className="grid grid-cols-4 gap-3">
              <MetricCard
                icon={Target}
                value={stats.targets.length}
                label={t("dashboard.targets", "Targets")}
                detail={`${derived.inScope} in · ${derived.outScope} out`}
                accent="blue"
              />
              <MetricCard
                icon={Bug}
                value={stats.findings.length}
                label={t("dashboard.findings", "Findings")}
                detail={derived.openCount > 0 ? `${derived.openCount} open` : undefined}
                accent="red"
              />
              <MetricCard
                icon={KeyRound}
                value={stats.vaultEntries.length}
                label={t("dashboard.credentials", "Credentials")}
                detail={Object.entries(derived.vaultByType).map(([k, v]) => `${v} ${k}`).join(", ") || undefined}
                accent="amber"
              />
              <MetricCard
                icon={Shield}
                value={derived.totalPorts}
                label="Open Ports"
                detail={derived.techCount > 0 ? `${derived.techCount} techs` : undefined}
                accent="green"
              />
            </div>

            {/* Target Breakdown + Findings */}
            <div className="grid grid-cols-2 gap-4">
              {/* Target Breakdown */}
              <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                <div className="flex items-center gap-2">
                  <Target className="w-3.5 h-3.5 text-blue-400/60" />
                  <span className="text-[11px] font-medium text-foreground/60">Target Breakdown</span>
                </div>
                <div className="grid grid-cols-2 gap-2">
                  {(["domain", "ip", "cidr", "url", "wildcard"] as const).map((type) => {
                    const count = derived.byType[type] || 0;
                    if (count === 0) return null;
                    const Icon = TYPE_ICON[type] || Globe;
                    return (
                      <div key={type} className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-muted/10">
                        <Icon className="w-3 h-3 text-muted-foreground/40" />
                        <span className="text-[10px] text-muted-foreground/50 capitalize flex-1">{type}</span>
                        <span className="text-[11px] font-semibold text-foreground/70 tabular-nums">{count}</span>
                      </div>
                    );
                  })}
                </div>
                {stats.targets.length > 0 && (
                  <div className="flex items-center gap-2 pt-1">
                    <div className="flex-1 h-1.5 rounded-full bg-muted/15 overflow-hidden flex">
                      <div
                        className="h-full bg-blue-500/50 transition-all"
                        style={{ width: `${(derived.inScope / stats.targets.length) * 100}%` }}
                        title={`In scope: ${derived.inScope}`}
                      />
                      <div
                        className="h-full bg-muted-foreground/15 transition-all"
                        style={{ width: `${(derived.outScope / stats.targets.length) * 100}%` }}
                        title={`Out of scope: ${derived.outScope}`}
                      />
                    </div>
                    <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap">
                      {derived.inScope}/{stats.targets.length} in scope
                    </span>
                  </div>
                )}
              </div>

              {/* Findings Overview */}
              <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                <div className="flex items-center gap-2">
                  <Bug className="w-3.5 h-3.5 text-red-400/60" />
                  <span className="text-[11px] font-medium text-foreground/60">Findings Overview</span>
                  {derived.openCount > 0 && (
                    <span className="ml-auto flex items-center gap-1 text-[9px] px-1.5 py-0.5 rounded-full bg-red-500/10 text-red-400">
                      <AlertTriangle className="w-2.5 h-2.5" /> {derived.openCount} open
                    </span>
                  )}
                </div>
                {stats.findings.length > 0 ? (
                  <>
                    <SeverityBar data={derived.findingsBySev} />
                    {Object.keys(derived.findingsByTool).length > 0 && (
                      <div className="flex flex-wrap gap-1.5 pt-1">
                        {Object.entries(derived.findingsByTool)
                          .sort(([, a], [, b]) => b - a)
                          .slice(0, 6)
                          .map(([tool, count]) => (
                            <span key={tool} className="text-[9px] px-2 py-0.5 rounded-full bg-muted/15 text-muted-foreground/50">
                              {tool} <span className="font-medium text-foreground/60">{count}</span>
                            </span>
                          ))}
                      </div>
                    )}
                  </>
                ) : (
                  <div className="flex items-center justify-center py-4 text-muted-foreground/20 text-[11px]">
                    <ShieldCheck className="w-4 h-4 mr-1.5" /> No findings yet
                  </div>
                )}
              </div>
            </div>

            {/* Methodology + Timeline + Activity */}
            <div className="grid grid-cols-3 gap-4">
              {/* Methodology */}
              <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                <div className="flex items-center gap-2">
                  <CheckCircle2 className="w-3.5 h-3.5 text-accent/60" />
                  <span className="text-[11px] font-medium text-foreground/60">Methodology</span>
                </div>
                {stats.methodProjects.length > 0 ? (
                  <MethodologyRing projects={stats.methodProjects} />
                ) : (
                  <div className="flex items-center justify-center py-4 text-muted-foreground/20 text-[11px]">
                    <Circle className="w-4 h-4 mr-1.5" /> No methodology started
                  </div>
                )}
              </div>

              {/* Findings Timeline */}
              <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                <div className="flex items-center gap-2">
                  <Activity className="w-3.5 h-3.5 text-accent/60" />
                  <span className="text-[11px] font-medium text-foreground/60">Findings Timeline</span>
                </div>
                {derived.timeline.length >= 2 ? (
                  <MiniTimeline data={derived.timeline} />
                ) : (
                  <div className="flex items-center justify-center py-4 text-muted-foreground/20 text-[11px]">
                    <Activity className="w-4 h-4 mr-1.5" /> Not enough data
                  </div>
                )}
              </div>

              {/* Recent Activity */}
              <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-2">
                <div className="flex items-center gap-2">
                  <Clock className="w-3.5 h-3.5 text-muted-foreground/40" />
                  <span className="text-[11px] font-medium text-foreground/60">Recent Activity</span>
                </div>
                {stats.recentActivity.length > 0 ? (
                  <div className="space-y-0.5 max-h-[140px] overflow-y-auto">
                    {stats.recentActivity.map((entry) => (
                      <div key={entry.id} className="flex items-start gap-2 py-1 text-[10px]">
                        <ActivityDot type={entry.action} />
                        <div className="min-w-0 flex-1">
                          <span className="text-foreground/60">{eventLabel(entry.action)}</span>
                          {entry.details && (
                            <span className="text-muted-foreground/35 ml-1 truncate">{entry.details.slice(0, 60)}</span>
                          )}
                        </div>
                        <span className="text-[9px] text-muted-foreground/25 flex-shrink-0 whitespace-nowrap">
                          {formatRelativeTime(entry.createdAt)}
                        </span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="flex items-center justify-center py-4 text-muted-foreground/20 text-[11px]">
                    <Clock className="w-4 h-4 mr-1.5" /> No recent activity
                  </div>
                )}
              </div>
            </div>

            {/* Top Findings */}
            {stats.findings.length > 0 && (
              <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                <div className="flex items-center gap-2">
                  <ShieldAlert className="w-3.5 h-3.5 text-orange-400/60" />
                  <span className="text-[11px] font-medium text-foreground/60">Latest Findings</span>
                  <span className="text-[9px] text-muted-foreground/30 ml-auto">{stats.findings.length} total</span>
                </div>
                <div className="space-y-1">
                  {stats.findings
                    .sort((a, b) => {
                      const sevIdx = (s: string) => ["critical", "high", "medium", "low", "info"].indexOf(s);
                      return sevIdx(a.severity) - sevIdx(b.severity);
                    })
                    .slice(0, 5)
                    .map((f) => (
                      <div key={f.id} className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg hover:bg-muted/10 transition-colors">
                        <SevIcon severity={f.severity} />
                        <span className="text-[10px] text-foreground/60 flex-1 truncate">
                          {f.title || `Finding ${f.id.slice(0, 8)}`}
                        </span>
                        {f.tool && (
                          <span className="text-[9px] text-muted-foreground/30 px-1.5 py-0.5 rounded bg-muted/10">{f.tool}</span>
                        )}
                        <span className={cn(
                          "text-[9px] px-1.5 py-0.5 rounded-full font-medium",
                          f.status === "open" || f.status === "confirmed"
                            ? "bg-red-500/10 text-red-400"
                            : "bg-green-500/10 text-green-400",
                        )}>
                          {f.status}
                        </span>
                      </div>
                    ))}
                </div>
              </div>
            )}
            </>)}

            {/* AI Usage Statistics */}
            {hasAiData && (
              <>
                <div className="flex items-center gap-2 pt-2">
                  <div className="h-px flex-1 bg-border/10" />
                  <div className="flex items-center gap-1.5 px-2">
                    <Brain className="w-3.5 h-3.5 text-purple-400/60" />
                    <span className="text-[11px] font-medium text-foreground/50">AI Usage Statistics</span>
                  </div>
                  <div className="h-px flex-1 bg-border/10" />
                </div>

                <div className="grid grid-cols-4 gap-3">
                  <MetricCard
                    icon={Cpu}
                    value={totalTokens}
                    label="Total Tokens"
                    detail={aiStats.tokenUsage ? `${fmtNum(aiStats.tokenUsage.total_tokens_in)} in · ${fmtNum(aiStats.tokenUsage.total_tokens_out)} out` : undefined}
                    accent="purple"
                  />
                  <MetricCard
                    icon={DollarSign}
                    value={totalCost}
                    displayValue={`$${totalCost.toFixed(4)}`}
                    label="Total Cost"
                    detail={aiStats.tokenUsage ? `$${aiStats.tokenUsage.total_cost_in.toFixed(4)} in · $${aiStats.tokenUsage.total_cost_out.toFixed(4)} out` : undefined}
                    accent="green"
                  />
                  <MetricCard
                    icon={Wrench}
                    value={totalToolCalls}
                    label="Tool Calls"
                    detail={aiStats.toolCallStats.length > 0 ? `${aiStats.toolCallStats.length} tools used` : undefined}
                    accent="blue"
                  />
                  <MetricCard
                    icon={Brain}
                    value={aiStats.memoryCount}
                    label="Memories"
                    detail={aiStats.agentUsage.length > 0 ? `${aiStats.agentUsage.length} agents active` : undefined}
                    accent="amber"
                  />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  {aiStats.agentUsage.length > 0 && (
                    <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                      <div className="flex items-center gap-2">
                        <Bot className="w-3.5 h-3.5 text-purple-400/60" />
                        <span className="text-[11px] font-medium text-foreground/60">Usage by Agent</span>
                        <span className="text-[9px] text-muted-foreground/30 ml-auto">
                          {aiStats.agentUsage.length} agents
                        </span>
                      </div>
                      <AgentUsageChart agents={aiStats.agentUsage} />
                    </div>
                  )}

                  {aiStats.toolCallStats.length > 0 && (
                    <div className="rounded-xl bg-muted/8 border border-border/10 p-4 space-y-3">
                      <div className="flex items-center gap-2">
                        <BarChart3 className="w-3.5 h-3.5 text-blue-400/60" />
                        <span className="text-[11px] font-medium text-foreground/60">Top Tools</span>
                        <span className="text-[9px] text-muted-foreground/30 ml-auto">
                          {totalToolCalls} total calls
                        </span>
                      </div>
                      <ToolCallChart tools={aiStats.toolCallStats.slice(0, 8)} maxCount={aiStats.toolCallStats[0]?.total_count ?? 1} />
                    </div>
                  )}
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
