import { ChevronDown, ChevronRight, Database, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { getStageAssetCoverage, type StageAssetCoverageSnapshot } from "@/lib/api/stage-coverage";
import { cn } from "@/lib/utils";

type TechniqueState =
  | "found"
  | "checked_empty"
  | "error"
  | "blocked"
  | "not_applicable"
  | "pending";

const TECH_META: Record<TechniqueState, { className: string; label: string; mark: string }> = {
  found: {
    className: "bg-emerald-500/15 text-emerald-300 border-emerald-500/30",
    label: "命中",
    mark: "✓",
  },
  checked_empty: {
    className: "bg-slate-500/15 text-slate-300 border-slate-500/30",
    label: "查空",
    mark: "∅",
  },
  error: {
    className: "bg-red-500/15 text-red-300 border-red-500/35",
    label: "错误",
    mark: "×",
  },
  blocked: {
    className: "bg-amber-500/15 text-amber-300 border-amber-500/30",
    label: "阻塞",
    mark: "!",
  },
  not_applicable: {
    className: "bg-muted/30 text-muted-foreground/50 border-border/30",
    label: "不适用",
    mark: "-",
  },
  pending: {
    className: "bg-transparent text-muted-foreground/40 border-border/40",
    label: "未查",
    mark: "·",
  },
};

const STATUS_LEGEND: TechniqueState[] = [
  "found",
  "checked_empty",
  "error",
  "blocked",
  "pending",
  "not_applicable",
];

type CoverageCell = StageAssetCoverageSnapshot["assets"][number]["coverage"][number];
type CoverageRow = StageAssetCoverageSnapshot["assets"][number];
type CoverageSummary = {
  blocked_assets: number;
  done_assets: number;
  new_assets: number;
  pending_assets: number;
  total_assets: number;
};

function normalizeTechniqueState(state: string): TechniqueState {
  return state in TECH_META ? (state as TechniqueState) : "pending";
}

function coverageCellTitle(cell: CoverageCell, state: TechniqueState) {
  const meta = TECH_META[state];
  const parts = [`${cell.label}: ${meta.label}`];
  if (cell.source) parts.push(`source: ${cell.source}`);
  if (cell.evidence_refs.length > 0) parts.push(`evidence: ${cell.evidence_refs.join(", ")}`);
  if (cell.note) parts.push(cell.note);
  if (cell.suggested_tools.length > 0) {
    parts.push(`suggested: ${cell.suggested_tools.join(", ")}`);
  }
  return parts.join(" · ");
}

function isOrganizationCoverageRow(row: CoverageRow) {
  return row.target_type === "organization" && row.source === "engagement_org";
}

function coverageRowsSummary(rows: CoverageRow[]): CoverageSummary {
  return rows.reduce<CoverageSummary>(
    (summary, row) => {
      const hasBlocked = row.coverage.some((cell) =>
        ["blocked", "error"].includes(normalizeTechniqueState(cell.state))
      );
      const hasPending = row.coverage.some(
        (cell) => normalizeTechniqueState(cell.state) === "pending"
      );
      summary.total_assets += 1;
      if (row.discovered_phase === "new_in_stage") summary.new_assets += 1;
      if (hasBlocked) {
        summary.blocked_assets += 1;
      } else if (hasPending) {
        summary.pending_assets += 1;
      } else {
        summary.done_assets += 1;
      }
      return summary;
    },
    {
      blocked_assets: 0,
      done_assets: 0,
      new_assets: 0,
      pending_assets: 0,
      total_assets: 0,
    }
  );
}

function coverageSummaryText(summary: CoverageSummary) {
  if (summary.total_assets === 0) return "0 assets";
  return `${summary.done_assets}/${summary.total_assets} done`;
}

function CoverageStatusCell({ cell, compact = false }: { cell: CoverageCell; compact?: boolean }) {
  const state = normalizeTechniqueState(cell.state);
  const meta = TECH_META[state];
  return (
    <span
      className={cn(
        "flex items-center justify-center justify-self-center rounded-sm border font-medium",
        compact ? "h-4 w-4 text-[8px]" : "h-5 w-5 text-[10px]",
        meta.className
      )}
      title={coverageCellTitle(cell, state)}
    >
      {meta.mark}
    </span>
  );
}

function assetPhaseLabel(phase: string) {
  switch (phase) {
    case "new_in_stage":
      return "新增";
    case "seed":
      return "种子";
    default:
      return "历史";
  }
}

function techniqueShortLabel(label: string) {
  const normalized = label.toLowerCase();
  if (normalized.includes("subdomain")) return "SUB";
  if (normalized.includes("liveness")) return "LIVE";
  if (normalized.includes("service")) return "SVC";
  if (normalized.includes("directory")) return "DIR";
  if (normalized.includes("parameter")) return "PARAM";
  if (normalized.includes("dns")) return "DNS";
  if (normalized.includes("whois")) return "WHOIS";
  if (normalized.includes("asn")) return "ASN";
  if (normalized.includes("ct")) return "CT";
  if (normalized.includes("osint")) return "OSINT";
  return (
    label
      .replace(/[^a-z0-9]/gi, "")
      .slice(0, 5)
      .toUpperCase() || label.slice(0, 5)
  );
}

export function StageAssetCoveragePanel({
  snapshot,
  loading,
  error,
}: {
  snapshot: StageAssetCoverageSnapshot | null;
  loading: boolean;
  error: string | null;
}) {
  if (loading) {
    return (
      <div className="rounded-md border border-border/30 bg-background/40 px-3 py-2 text-[11px] text-muted-foreground">
        <span className="inline-flex items-center gap-2">
          <Loader2 className="h-3 w-3 animate-spin" />
          Loading assets
        </span>
      </div>
    );
  }
  if (error) {
    return (
      <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300">
        {error}
      </div>
    );
  }
  if (!snapshot) return null;
  const organizationRows = snapshot.assets.filter(isOrganizationCoverageRow);
  const assetRows = snapshot.assets.filter((asset) => !isOrganizationCoverageRow(asset));
  if (assetRows.length === 0 && organizationRows.length === 0) {
    return (
      <div className="rounded-md border border-border/30 bg-background/40 px-3 py-2 text-[11px] text-muted-foreground">
        No in-scope assets for this organization.
      </div>
    );
  }

  const summary = coverageRowsSummary(assetRows);
  const newAssets = assetRows.filter((asset) => asset.discovered_phase === "new_in_stage");
  const existingAssets = assetRows.filter((asset) => asset.discovered_phase !== "new_in_stage");
  const groups = [
    { key: "new", label: "新增资产", assets: newAssets },
    { key: "existing", label: "已有资产", assets: existingAssets },
  ].filter((group) => group.assets.length > 0);
  const techniques = assetRows[0]?.coverage ?? [];
  const techniqueColumnCount = Math.max(techniques.length, 1);
  const gridTemplateColumns = `minmax(0,1fr) repeat(${techniqueColumnCount}, minmax(24px,40px))`;

  return (
    <div className="rounded-md border border-border/30 bg-background/40">
      <div className="flex flex-wrap items-center gap-2 border-b border-border/20 px-3 py-2 text-[10px] text-muted-foreground">
        <span className="inline-flex items-center gap-1 font-medium text-foreground/80">
          <Database className="h-3 w-3" />
          {coverageSummaryText(summary)}
        </span>
        {summary.new_assets > 0 && (
          <span className="rounded bg-sky-500/15 px-1.5 py-0.5 text-sky-300">
            {summary.new_assets} 新增
          </span>
        )}
        {summary.pending_assets > 0 && (
          <span className="rounded bg-muted/40 px-1.5 py-0.5">
            {summary.pending_assets} pending
          </span>
        )}
        {summary.blocked_assets > 0 && (
          <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-300">
            {summary.blocked_assets} 需处理
          </span>
        )}
        <div className="flex flex-wrap items-center gap-1.5 text-[9px] text-muted-foreground/65">
          {STATUS_LEGEND.map((state) => {
            const meta = TECH_META[state];
            return (
              <span key={state} className="inline-flex items-center gap-1">
                <span
                  className={cn(
                    "flex h-3.5 w-3.5 items-center justify-center rounded-sm border text-[8px] font-medium",
                    meta.className
                  )}
                >
                  {meta.mark}
                </span>
                {meta.label}
              </span>
            );
          })}
        </div>
      </div>
      <div className="max-h-56 overflow-y-auto overflow-x-hidden">
        {organizationRows.length > 0 && (
          <div className="border-b border-border/15 px-3 py-2">
            {organizationRows.map((row) => (
              <div key={row.target_id} className="flex min-w-0 flex-wrap items-center gap-2">
                <span className="shrink-0 text-[10px] font-medium text-muted-foreground/70">
                  组织情报
                </span>
                <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/80">
                  {row.value}
                </span>
                <div className="flex shrink-0 flex-wrap items-center gap-1">
                  {row.coverage.map((cell) => (
                    <CoverageStatusCell key={cell.technique} cell={cell} compact />
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
        {assetRows.length === 0 && (
          <div className="px-3 py-3 text-[11px] text-muted-foreground/65">
            暂无已登记资产；组织级情报状态显示在上方。
          </div>
        )}
        {assetRows.length > 0 && (
          <div className="w-full">
            <div
              className="grid items-center gap-1.5 border-b border-border/15 px-3 py-1.5 text-[9px] font-medium uppercase text-muted-foreground/60"
              style={{ gridTemplateColumns }}
            >
              <span className="min-w-0">Asset</span>
              {techniques.map((technique) => (
                <span
                  key={technique.technique}
                  className="truncate text-center"
                  title={technique.label}
                >
                  {techniqueShortLabel(technique.label)}
                </span>
              ))}
            </div>
            {groups.map((group) => (
              <div key={group.key}>
                <div className="bg-muted/20 px-3 py-1 text-[10px] font-medium text-muted-foreground/70">
                  {group.label}
                </div>
                {group.assets.map((asset) => (
                  <div
                    key={asset.target_id}
                    className="grid items-center gap-1.5 border-t border-border/10 px-3 py-1.5 text-[11px]"
                    style={{ gridTemplateColumns }}
                  >
                    <div className="min-w-0">
                      <div className="truncate font-medium text-foreground/85" title={asset.value}>
                        {asset.value}
                      </div>
                      <div className="mt-0.5 truncate text-[9px] text-muted-foreground/50">
                        {assetPhaseLabel(asset.discovered_phase)} · {asset.target_type} ·{" "}
                        {asset.source || "-"}
                      </div>
                    </div>
                    {asset.coverage.map((cell) => {
                      return <CoverageStatusCell key={cell.technique} cell={cell} />;
                    })}
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export function StageAssetCoverageBlock({
  organizationId,
  stage,
  sessionId,
  title = "资产覆盖",
  subtitle,
  pollWhileActive = false,
  className,
}: {
  organizationId: string;
  stage: string;
  sessionId?: string | null;
  title?: string;
  subtitle?: string;
  pollWhileActive?: boolean;
  className?: string;
}) {
  const [snapshot, setSnapshot] = useState<StageAssetCoverageSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    setSnapshot(null);
    setError(null);
    setLoading(false);
    setExpanded(false);
  }, [organizationId, sessionId, stage]);

  useEffect(() => {
    if (!expanded) return;

    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    const load = async (showLoading: boolean) => {
      if (showLoading) setLoading(true);
      setError(null);
      try {
        const next = await getStageAssetCoverage({
          organizationId,
          stage,
          sessionId,
        });
        if (!cancelled) setSnapshot(next);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void load(true);
    if (pollWhileActive) {
      timer = setInterval(() => void load(false), 4000);
    }

    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [expanded, organizationId, pollWhileActive, sessionId, stage]);

  const summary = snapshot
    ? coverageRowsSummary(snapshot.assets.filter((asset) => !isOrganizationCoverageRow(asset)))
    : null;
  const summaryText = summary
    ? coverageSummaryText(summary)
    : error
      ? "加载失败"
      : expanded && loading
        ? "加载中"
        : "查看";

  return (
    <section className={cn("rounded-md border border-border/30 bg-background/25", className)}>
      <button
        type="button"
        className="flex min-h-10 w-full min-w-0 items-center justify-between gap-3 px-2.5 py-1.5 text-left hover:bg-muted/15"
        onClick={() => {
          if (expanded) setLoading(false);
          setExpanded(!expanded);
        }}
        aria-expanded={expanded}
      >
        <div className="flex min-w-0 items-center gap-2">
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
          )}
          <Database className="h-3.5 w-3.5 shrink-0 text-muted-foreground/80" />
          <div className="min-w-0">
            <div className="truncate text-[11px] font-semibold text-foreground/85">{title}</div>
            {subtitle && (
              <div className="mt-0.5 truncate text-[10px] text-muted-foreground/60">{subtitle}</div>
            )}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5 text-[10px] text-muted-foreground/70">
          {expanded && loading && (
            <Loader2 className="h-3 w-3 animate-spin text-[var(--ansi-blue)]/80" />
          )}
          <span className="rounded bg-muted/35 px-1.5 py-0.5 font-medium text-foreground/70">
            {summaryText}
          </span>
          {summary && summary.new_assets > 0 && (
            <span className="rounded bg-sky-500/15 px-1.5 py-0.5 text-sky-300">
              +{summary.new_assets}
            </span>
          )}
          {summary && summary.blocked_assets > 0 && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-300">
              {summary.blocked_assets} 需处理
            </span>
          )}
          {pollWhileActive && expanded && !loading && (
            <span className="rounded bg-[var(--ansi-blue)]/10 px-1.5 py-0.5 text-[var(--ansi-blue)]">
              Live
            </span>
          )}
        </div>
      </button>
      {expanded && (
        <div className="border-t border-border/20 p-2">
          <StageAssetCoveragePanel snapshot={snapshot} loading={loading} error={error} />
        </div>
      )}
    </section>
  );
}
