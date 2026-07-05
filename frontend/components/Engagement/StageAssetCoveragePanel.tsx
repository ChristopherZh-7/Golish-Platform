import {
  ChevronDown,
  ChevronRight,
  Database,
  GripHorizontal,
  ListTree,
  Loader2,
} from "lucide-react";
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { getStageAssetCoverage, type StageAssetCoverageSnapshot } from "@/lib/api/stage-coverage";
import { cn } from "@/lib/utils";

export type StageAssetCoverageWorkStatus =
  | "running"
  | "backgrounded"
  | "completed"
  | "error"
  | "interrupted";

export interface StageAssetCoverageWorkItem {
  id: string;
  displayToolName: string;
  rawToolName: string;
  subject: string | null;
  subjects: string[];
  primary: string | null;
  techniques: string[];
  status: StageAssetCoverageWorkStatus;
  startedAt: string;
  completedAt?: string;
  outputPreview?: string;
}

type TechniqueState =
  | "found"
  | "checked_empty"
  | "error"
  | "blocked"
  | "not_applicable"
  | "next_wave_pending"
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
  next_wave_pending: {
    className: "bg-sky-500/10 text-sky-300 border-sky-500/25",
    label: "下批",
    mark: "↻",
  },
  pending: {
    className: "bg-transparent text-muted-foreground/55 border-border/45",
    label: "未查",
    mark: "?",
  },
};

const STATUS_LEGEND: TechniqueState[] = [
  "found",
  "checked_empty",
  "error",
  "blocked",
  "next_wave_pending",
  "pending",
  "not_applicable",
];

const ROW_STATUS_SUMMARY_ORDER: TechniqueState[] = [
  "error",
  "blocked",
  "pending",
  "next_wave_pending",
  "checked_empty",
  "not_applicable",
];

const ROW_STATUS_SUMMARY_LABEL: Partial<Record<TechniqueState, string>> = {
  blocked: "阻塞",
  checked_empty: "查空",
  error: "错误",
  next_wave_pending: "下批待查",
  not_applicable: "不适用",
  pending: "未查",
};

type CoverageCell = StageAssetCoverageSnapshot["assets"][number]["coverage"][number];
type CoverageCapabilitySuggestion = {
  id: string;
  label?: string;
  tools?: string[];
  risk?: string;
  batchable?: boolean;
  max_batch?: number;
  reason?: string;
};
type CoverageCellWithCapabilities = CoverageCell & {
  suggested_capabilities?: CoverageCapabilitySuggestion[];
};
type CoverageRow = StageAssetCoverageSnapshot["assets"][number];
type CoverageSummary = {
  blocked_assets: number;
  done_assets: number;
  new_assets: number;
  pending_assets: number;
  total_assets: number;
};
type CoverageViewMode = "active" | "all";

const ASSET_COVERAGE_BODY_DEFAULT_HEIGHT = 224;
const ASSET_COVERAGE_BODY_MIN_HEIGHT = 160;
const ASSET_COVERAGE_BODY_MAX_HEIGHT = 560;
const ASSET_COVERAGE_BODY_KEYBOARD_STEP = 24;
const ASSET_COVERAGE_GROUP_VIRTUALIZATION_THRESHOLD = 512;
const ASSET_COVERAGE_GROUP_OVERSCAN = 24;
export const LIVE_WORK_RETENTION_MS = 3500;
export const ASSET_COVERAGE_READING_FREEZE_MS = 8000;

const virtualItemBaseStyle = {
  left: 0,
  position: "absolute",
  top: 0,
  width: "100%",
} as const;

function coverageScrollRect(element: Element | null): { height: number; width: number } {
  if (!element) return { height: ASSET_COVERAGE_BODY_DEFAULT_HEIGHT, width: 0 };
  const rect = element.getBoundingClientRect();
  const styleHeight =
    element instanceof HTMLElement ? Number.parseFloat(element.style.height || "") : 0;
  const height =
    rect.height || element.clientHeight || styleHeight || ASSET_COVERAGE_BODY_DEFAULT_HEIGHT;
  return {
    height: Math.round(height),
    width: Math.round(rect.width || element.clientWidth || 0),
  };
}

function clampAssetCoverageBodyHeight(height: number): number {
  return Math.min(
    ASSET_COVERAGE_BODY_MAX_HEIGHT,
    Math.max(ASSET_COVERAGE_BODY_MIN_HEIGHT, Math.round(height))
  );
}

function normalizeTechniqueState(state: string): TechniqueState {
  return state in TECH_META ? (state as TechniqueState) : "pending";
}

function coverageCellTitle(cell: CoverageCell, state: TechniqueState) {
  const meta = TECH_META[state];
  const parts = [`${cell.label}: ${meta.label}`];
  if (cell.source) parts.push(`source: ${cell.source}`);
  if (cell.evidence_refs.length > 0) parts.push(`evidence: ${cell.evidence_refs.join(", ")}`);
  if (cell.note) parts.push(cell.note);
  const capabilities = (cell as CoverageCellWithCapabilities).suggested_capabilities ?? [];
  if (capabilities.length > 0) {
    parts.push(
      `capability: ${capabilities
        .map((capability) => capability.label || capability.id)
        .join(", ")}`
    );
  }
  if (cell.suggested_tools.length > 0) {
    parts.push(`suggested: ${cell.suggested_tools.join(", ")}`);
  }
  return parts.join(" · ");
}

function isOrganizationCoverageRow(row: CoverageRow) {
  return row.target_type === "organization" && row.source === "engagement_org";
}

function coverageSummaryText(summary: CoverageSummary) {
  if (summary.total_assets === 0) return "0 assets";
  return `${summary.done_assets}/${summary.total_assets} done`;
}

function isLiveWorkStatus(status: StageAssetCoverageWorkStatus): boolean {
  return status === "running" || status === "backgrounded";
}

function liveWorkItemsKey(items: StageAssetCoverageWorkItem[]): string {
  return items
    .map((item) =>
      [
        item.id,
        item.status,
        item.displayToolName,
        item.subject ?? "",
        item.subjects.join(","),
        item.techniques.join(","),
      ].join(":")
    )
    .join("|");
}

function workItemsRefreshKey(items: StageAssetCoverageWorkItem[]): string {
  return items
    .map((item) =>
      [
        item.id,
        item.status,
        item.subject ?? "",
        item.subjects.join(","),
        item.techniques.join(","),
      ].join(":")
    )
    .join("|");
}

function mergeDisplayLiveWorkItems(
  previous: StageAssetCoverageWorkItem[],
  incoming: StageAssetCoverageWorkItem[]
): StageAssetCoverageWorkItem[] {
  if (previous.length === 0) return incoming;
  const incomingById = new Map(incoming.map((item) => [item.id, item]));
  const previousIds = new Set(previous.map((item) => item.id));
  const retainedPrevious = previous.map((item) => incomingById.get(item.id) ?? item);
  const appendedIncoming = incoming.filter((item) => !previousIds.has(item.id));
  const next = [...retainedPrevious, ...appendedIncoming];
  return liveWorkItemsKey(next) === liveWorkItemsKey(previous) ? previous : next;
}

function normalizeAssetToken(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/\/$/, "");
}

function assetTokens(value: string): string[] {
  const normalized = normalizeAssetToken(value);
  const tokens = [normalized];
  try {
    const parsed = new URL(/^https?:\/\//i.test(value) ? value : `http://${value}`);
    tokens.push(parsed.hostname.toLowerCase());
    if (parsed.port) tokens.push(`${parsed.hostname.toLowerCase()}:${parsed.port}`);
  } catch {
    // Best-effort matching only; keep the normalized raw token above.
  }
  return Array.from(new Set(tokens.filter(Boolean)));
}

function workMatchesAsset(row: CoverageRow, item: StageAssetCoverageWorkItem): boolean {
  const rowTokens = assetTokens(row.value);
  return item.subjects.some((subject) => {
    const subjectTokens = assetTokens(subject);
    return subjectTokens.some((subjectToken) =>
      rowTokens.some((rowToken) => subjectToken === rowToken || subjectToken.includes(rowToken))
    );
  });
}

function techniqueKeyFromCell(cell: CoverageCell): string {
  const text = `${cell.technique} ${cell.label}`.toUpperCase();
  if (text.includes("LIVENESS") || text.includes("LIVE")) return "LIVENESS";
  if (text.includes("SERVICE") || text.includes("FINGERPRINT") || text.includes("SVC")) {
    return "SERVICE";
  }
  if (text.includes("PORT")) return "PORT";
  if (text.includes("DIR")) return "DIR";
  if (text.includes("PARAM")) return "PARAM";
  if (text.includes("JSAPI")) return "JSAPI";
  if (text.includes("JS")) return "JS";
  if (text.includes("DNS")) return "DNS";
  if (text.includes("WHOIS")) return "WHOIS";
  if (text.includes("ASN")) return "ASN";
  if (text.includes("CT")) return "CT";
  if (text.includes("SUBDOMAIN")) return "SUBDOMAIN";
  if (text.includes("OSINT")) return "OSINT";
  return text.trim();
}

function workMatchesTechnique(cell: CoverageCell, item: StageAssetCoverageWorkItem): boolean {
  if (item.techniques.length === 0) return true;
  const key = techniqueKeyFromCell(cell);
  return item.techniques.some((tech) => tech.toUpperCase() === key);
}

function workTechniqueLabel(item: StageAssetCoverageWorkItem): string {
  return item.techniques.length > 0 ? item.techniques.join("/") : "覆盖";
}

function workSubjectLabel(item: StageAssetCoverageWorkItem): string {
  if (item.subjects.length > 1) return `批量 ${item.subjects.length}`;
  return item.subject ?? item.primary ?? item.displayToolName;
}

function workFocusSubjectLabel(item: StageAssetCoverageWorkItem): string {
  const first = item.subject ?? item.subjects[0] ?? item.primary ?? item.displayToolName;
  if (item.subjects.length <= 1) return first;
  return `${first} +${item.subjects.length - 1}`;
}

function workMatchedRows(item: StageAssetCoverageWorkItem, rows: CoverageRow[]): CoverageRow[] {
  return rows.filter((row) => workMatchesAsset(row, item));
}

const IP_TARGET_TYPES = new Set(["ip", "ipv4", "ipv6", "ip_address", "cidr", "range", "netblock"]);

function isIpCoverageRow(row: CoverageRow): boolean {
  return IP_TARGET_TYPES.has(row.target_type.toLowerCase());
}

function rowResolvedIp(row: CoverageRow): string {
  return row.real_ip.trim();
}

interface AssetCoverageGroup {
  key: string;
  label: string;
  hostRow: CoverageRow | null;
  childRows: CoverageRow[];
  firstIndex: number;
  resolvedGroup: boolean;
}

function buildAssetCoverageGroups(rows: CoverageRow[]): AssetCoverageGroup[] {
  const groups = new Map<string, AssetCoverageGroup>();

  const getGroup = (key: string, label: string, firstIndex: number, resolvedGroup: boolean) => {
    const existing = groups.get(key);
    if (existing) {
      existing.firstIndex = Math.min(existing.firstIndex, firstIndex);
      existing.resolvedGroup = existing.resolvedGroup || resolvedGroup;
      return existing;
    }
    const next: AssetCoverageGroup = {
      key,
      label,
      hostRow: null,
      childRows: [],
      firstIndex,
      resolvedGroup,
    };
    groups.set(key, next);
    return next;
  };

  rows.forEach((row, index) => {
    if (isIpCoverageRow(row)) {
      const key = `ip:${normalizeAssetToken(row.value)}`;
      const group = getGroup(key, row.value, index, true);
      group.hostRow = row;
      group.label = row.value;
      return;
    }

    const realIp = rowResolvedIp(row);
    if (realIp) {
      const key = `ip:${normalizeAssetToken(realIp)}`;
      getGroup(key, realIp, index, true).childRows.push(row);
      return;
    }

    getGroup(`asset:${row.target_id}`, row.value, index, false).childRows.push(row);
  });

  return Array.from(groups.values()).sort(
    (a, b) => a.firstIndex - b.firstIndex || a.label.localeCompare(b.label)
  );
}

function groupRows(group: AssetCoverageGroup): CoverageRow[] {
  return group.hostRow ? [group.hostRow, ...group.childRows] : group.childRows;
}

function estimateCoverageGroupHeight(group?: AssetCoverageGroup): number {
  if (!group) return 72;
  const syntheticHostRows =
    !group.hostRow && group.resolvedGroup && group.childRows.length > 0 ? 1 : 0;
  const rowCount = groupRows(group).length + syntheticHostRows;
  return Math.max(52, rowCount * 54 + 2);
}

type RenderCoverageGroup = (group: AssetCoverageGroup, showActivityBadges: boolean) => ReactNode;

interface CoverageGroupMeasurement {
  end: number;
  group: AssetCoverageGroup;
  index: number;
  size: number;
  start: number;
}

function buildCoverageGroupMeasurements(groups: AssetCoverageGroup[]) {
  let offset = 0;
  const measurements = groups.map((group, index): CoverageGroupMeasurement => {
    const size = estimateCoverageGroupHeight(group);
    const start = offset;
    offset += size;
    return {
      end: offset,
      group,
      index,
      size,
      start,
    };
  });
  return { measurements, totalSize: offset };
}

function firstVisibleCoverageGroupIndex(
  measurements: CoverageGroupMeasurement[],
  scrollTop: number
): number {
  let low = 0;
  let high = measurements.length - 1;
  let result = 0;
  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    if ((measurements[mid]?.end ?? 0) >= scrollTop) {
      result = mid;
      high = mid - 1;
    } else {
      low = mid + 1;
    }
  }
  return result;
}

function CoverageGroupsList({
  groups,
  renderCoverageGroup,
  resetKey,
  scrollParent,
  showActivityBadges,
}: {
  groups: AssetCoverageGroup[];
  renderCoverageGroup: RenderCoverageGroup;
  resetKey: string;
  scrollParent: HTMLDivElement | null;
  showActivityBadges: boolean;
}) {
  const anchorRef = useRef<HTMLDivElement>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  const [scrollMetrics, setScrollMetrics] = useState({
    scrollTop: 0,
    viewportHeight: ASSET_COVERAGE_BODY_DEFAULT_HEIGHT,
  });
  const { measurements, totalSize } = useMemo(
    () => buildCoverageGroupMeasurements(groups),
    [groups]
  );

  useLayoutEffect(() => {
    const updateScrollMargin = () => {
      const anchor = anchorRef.current;
      const parent = scrollParent;
      if (!anchor || !parent) return;
      const nextMargin =
        anchor.getBoundingClientRect().top - parent.getBoundingClientRect().top + parent.scrollTop;
      setScrollMargin(Math.max(0, Math.round(nextMargin)));
    };

    updateScrollMargin();
    const frame = window.requestAnimationFrame(updateScrollMargin);
    let resizeObserver: ResizeObserver | undefined;
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(updateScrollMargin);
      if (scrollParent) resizeObserver.observe(scrollParent);
      if (anchorRef.current?.parentElement) resizeObserver.observe(anchorRef.current.parentElement);
    }

    return () => {
      window.cancelAnimationFrame(frame);
      resizeObserver?.disconnect();
    };
  }, [resetKey, scrollParent]);

  useLayoutEffect(() => {
    const scrollElement = scrollParent;
    if (!scrollElement) return;

    let frame: number | null = null;
    const readMetrics = () => {
      frame = null;
      const rect = coverageScrollRect(scrollElement);
      setScrollMetrics((previous) => {
        const next = {
          scrollTop: Math.max(0, scrollElement.scrollTop),
          viewportHeight: rect.height,
        };
        return previous.scrollTop === next.scrollTop &&
          previous.viewportHeight === next.viewportHeight
          ? previous
          : next;
      });
    };
    const scheduleRead = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(readMetrics);
    };
    const readScrollMetrics = () => {
      if (frame !== null) {
        window.cancelAnimationFrame(frame);
        frame = null;
      }
      readMetrics();
    };

    readMetrics();
    scrollElement.addEventListener("scroll", readScrollMetrics, { passive: true });
    let resizeObserver: ResizeObserver | undefined;
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(scheduleRead);
      resizeObserver.observe(scrollElement);
    }
    window.addEventListener("resize", scheduleRead, { passive: true });

    return () => {
      if (frame !== null) window.cancelAnimationFrame(frame);
      scrollElement.removeEventListener("scroll", readScrollMetrics);
      resizeObserver?.disconnect();
      window.removeEventListener("resize", scheduleRead);
    };
  }, [scrollParent]);

  useLayoutEffect(() => {
    const scrollElement = scrollParent;
    if (!scrollElement) return;
    const clampScrollTop = () => {
      const maxScrollTop = Math.max(0, scrollElement.scrollHeight - scrollElement.clientHeight);
      if (scrollElement.scrollTop > maxScrollTop) {
        scrollElement.scrollTop = maxScrollTop;
      }
      const rect = coverageScrollRect(scrollElement);
      setScrollMetrics((previous) => {
        const next = {
          scrollTop: Math.max(0, scrollElement.scrollTop),
          viewportHeight: rect.height,
        };
        return previous.scrollTop === next.scrollTop &&
          previous.viewportHeight === next.viewportHeight
          ? previous
          : next;
      });
    };
    clampScrollTop();
    const frame = window.requestAnimationFrame(clampScrollTop);
    return () => window.cancelAnimationFrame(frame);
  }, [groups.length, resetKey, scrollParent, totalSize]);

  if (groups.length < ASSET_COVERAGE_GROUP_VIRTUALIZATION_THRESHOLD) {
    return (
      <div ref={anchorRef} className="w-full" data-testid="stage-asset-coverage-groups">
        {groups.map((group) => renderCoverageGroup(group, showActivityBadges))}
      </div>
    );
  }

  const listScrollTop = Math.max(0, scrollMetrics.scrollTop - scrollMargin);
  const viewportHeight = Math.max(1, scrollMetrics.viewportHeight);
  const firstVisibleIndex = firstVisibleCoverageGroupIndex(measurements, listScrollTop);
  const startIndex = Math.max(0, firstVisibleIndex - ASSET_COVERAGE_GROUP_OVERSCAN);
  const viewportEnd = listScrollTop + viewportHeight;
  let endIndex = firstVisibleIndex;
  while (endIndex < measurements.length - 1 && (measurements[endIndex]?.start ?? 0) < viewportEnd) {
    endIndex += 1;
  }
  endIndex = Math.min(measurements.length - 1, endIndex + ASSET_COVERAGE_GROUP_OVERSCAN);
  const virtualItems = measurements.slice(startIndex, endIndex + 1);

  return (
    <div
      ref={anchorRef}
      className="w-full bg-background/40"
      data-testid="stage-asset-coverage-groups"
    >
      <div
        className="relative w-full bg-background/40"
        data-testid="stage-asset-coverage-virtual-groups"
        style={{ height: totalSize }}
      >
        {virtualItems.map((virtualRow) => {
          const { group } = virtualRow;
          return (
            <div
              key={group.key}
              data-index={virtualRow.index}
              style={{
                ...virtualItemBaseStyle,
                contain: "layout paint",
                transform: `translateY(${virtualRow.start}px)`,
              }}
            >
              {renderCoverageGroup(group, showActivityBadges)}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function groupMatchesLiveWork(
  group: AssetCoverageGroup,
  liveWorkItems: StageAssetCoverageWorkItem[]
): boolean {
  const rows = groupRows(group);
  return rows.some((row) => liveWorkItems.some((item) => workMatchesAsset(row, item)));
}

function groupRelatedWorkItems(
  group: AssetCoverageGroup,
  liveWorkItems: StageAssetCoverageWorkItem[]
): StageAssetCoverageWorkItem[] {
  if (group.childRows.length === 0) return [];
  const directHostIds = new Set(
    group.hostRow
      ? liveWorkItems
          .filter((item) => workMatchesAsset(group.hostRow as CoverageRow, item))
          .map((i) => i.id)
      : []
  );
  return liveWorkItems.filter(
    (item) =>
      !directHostIds.has(item.id) && group.childRows.some((row) => workMatchesAsset(row, item))
  );
}

function formatTechniqueList(labels: string[]): string {
  const uniqueLabels = Array.from(new Set(labels.filter(Boolean)));
  if (uniqueLabels.length <= 3) return uniqueLabels.join("/");
  return `${uniqueLabels.slice(0, 3).join("/")} +${uniqueLabels.length - 3}`;
}

function coverageRowStatusSummary(row: CoverageRow): string | null {
  const grouped = new Map<TechniqueState, string[]>();
  row.coverage.forEach((cell) => {
    const state = normalizeTechniqueState(cell.state);
    if (state === "found") return;
    grouped.set(state, [...(grouped.get(state) ?? []), techniqueShortLabel(cell.label)]);
  });

  const parts = ROW_STATUS_SUMMARY_ORDER.flatMap((state) => {
    const labels = grouped.get(state);
    const summaryLabel = ROW_STATUS_SUMMARY_LABEL[state];
    if (!labels || labels.length === 0 || !summaryLabel) return [];
    return `${summaryLabel} ${formatTechniqueList(labels)}`;
  });
  return parts.length > 0 ? parts.join(" · ") : null;
}

function rowMetaLabel(row: CoverageRow, child = false): string {
  const parts = [assetPhaseLabel(row.discovered_phase), row.target_type, row.source || "-"];
  if (child && rowResolvedIp(row)) parts.push(`解析到 ${rowResolvedIp(row)}`);
  const statusSummary = coverageRowStatusSummary(row);
  if (statusSummary) parts.push(statusSummary);
  return parts.join(" · ");
}

function WorkItemBadges({
  items,
  related = false,
}: {
  items: StageAssetCoverageWorkItem[];
  related?: boolean;
}) {
  if (items.length === 0) return null;
  return (
    <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1">
      {items.slice(0, 2).map((item) => (
        <span
          key={item.id}
          className={cn(
            "inline-flex max-w-full items-center gap-1 rounded px-1.5 py-0.5 text-[9px]",
            related
              ? "bg-emerald-500/10 text-emerald-300"
              : "bg-[var(--ansi-blue)]/10 text-[var(--ansi-blue)]"
          )}
          title={item.primary ?? item.outputPreview ?? item.displayToolName}
        >
          <Loader2
            className={cn(
              "h-2.5 w-2.5 shrink-0 animate-spin",
              related ? "text-emerald-300" : "text-[var(--ansi-blue)]"
            )}
          />
          <span className="truncate">
            {related ? `关联 ${workSubjectLabel(item)} · ` : "正在补 "}
            {workTechniqueLabel(item)} · {item.displayToolName}
          </span>
        </span>
      ))}
      {items.length > 2 && (
        <span className="rounded bg-muted/30 px-1 py-0.5 text-[9px] text-muted-foreground/70">
          +{items.length - 2}
        </span>
      )}
    </div>
  );
}

function LiveFocusBar({
  activeAssetCount,
  activeGroupCount,
  assetRows,
  coverageView,
  liveWorkItems,
}: {
  activeAssetCount: number;
  activeGroupCount: number;
  assetRows: CoverageRow[];
  coverageView: CoverageViewMode;
  liveWorkItems: StageAssetCoverageWorkItem[];
}) {
  if (liveWorkItems.length === 0 && coverageView !== "active") return null;
  if (liveWorkItems.length === 0) {
    return (
      <div className="flex h-10 min-w-0 shrink-0 items-center gap-2 overflow-hidden border-b border-border/20 bg-muted/10 px-3 text-[10px]">
        <span className="inline-flex h-6 min-w-16 shrink-0 items-center justify-center gap-1 rounded bg-muted/30 px-1.5 font-medium text-muted-foreground/80">
          运行中
        </span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground/70">
          当前没有运行中的资产任务
        </span>
      </div>
    );
  }
  const primary = liveWorkItems[0];
  const matches = workMatchedRows(primary, assetRows);
  const firstMatch = matches[0];
  const targetLabel = firstMatch
    ? `${firstMatch.value}${rowResolvedIp(firstMatch) ? ` -> ${rowResolvedIp(firstMatch)}` : ""}`
    : workFocusSubjectLabel(primary);
  return (
    <div className="flex h-10 min-w-0 shrink-0 items-center gap-2 overflow-hidden border-b border-border/20 bg-[var(--ansi-blue)]/[0.03] px-3 text-[10px]">
      <span className="inline-flex h-6 min-w-16 shrink-0 items-center justify-center gap-1 rounded bg-[var(--ansi-blue)]/10 px-1.5 font-medium text-[var(--ansi-blue)]">
        <Loader2 className="h-3 w-3 animate-spin" />
        运行中
      </span>
      <span
        className="min-w-0 flex-1 truncate font-medium text-foreground/85"
        title={primary.primary ?? primary.outputPreview ?? primary.displayToolName}
      >
        {primary.displayToolName} · {workTechniqueLabel(primary)} · {workSubjectLabel(primary)} ·{" "}
        {targetLabel}
      </span>
      {liveWorkItems.length > 1 && (
        <span className="inline-flex h-6 min-w-8 shrink-0 items-center justify-center rounded bg-background/35 px-1.5 tabular-nums text-muted-foreground/75">
          +{liveWorkItems.length - 1}
        </span>
      )}
      <span className="hidden h-6 min-w-24 shrink-0 items-center justify-center rounded bg-background/35 px-1.5 tabular-nums text-muted-foreground/75 sm:inline-flex">
        {activeGroupCount} 组 / {activeAssetCount} 资产
      </span>
    </div>
  );
}

function CoverageStatusCell({
  cell,
  compact = false,
  workItems = [],
}: {
  cell: CoverageCell;
  compact?: boolean;
  workItems?: StageAssetCoverageWorkItem[];
}) {
  const state = normalizeTechniqueState(cell.state);
  const meta = TECH_META[state];
  const liveWork = workItems.filter((item) => isLiveWorkStatus(item.status));
  const title = [
    coverageCellTitle(cell, state),
    ...liveWork.map(
      (item) => `正在补 ${workTechniqueLabel(item)}: ${item.displayToolName} ${item.primary ?? ""}`
    ),
  ].join(" · ");
  return (
    <span
      className={cn(
        "flex items-center justify-center justify-self-center rounded-sm border font-medium",
        compact ? "h-4 w-4 text-[8px]" : "h-5 w-5 text-[10px]",
        meta.className,
        liveWork.length > 0 && "border-[var(--ansi-blue)]/70 bg-[var(--ansi-blue)]/15"
      )}
      title={title}
    >
      {liveWork.length > 0 ? <Loader2 className="h-3 w-3 animate-spin" /> : meta.mark}
    </span>
  );
}

function organizationTechniqueLabel(cell: CoverageCell): string {
  const text = `${cell.technique} ${cell.label}`.toUpperCase();
  if (text.includes("WHOIS")) return "WHOIS";
  if (text.includes("ASN")) return "ASN";
  if (text.includes("SUBDOMAIN")) return "子域";
  if (text.includes("OSINT")) return "OSINT";
  if (text.includes("DNS")) return "DNS";
  if (text.includes("CT")) return "CT证书";
  return techniqueShortLabel(cell.label);
}

function OrganizationCoverageStatusCell({ cell }: { cell: CoverageCell }) {
  const state = normalizeTechniqueState(cell.state);
  const meta = TECH_META[state];
  return (
    <span
      className={cn(
        "inline-flex h-6 min-w-[3.75rem] items-center justify-between gap-1 rounded-sm border px-1.5 text-[9px] font-medium",
        meta.className
      )}
      title={coverageCellTitle(cell, state)}
    >
      <span className="truncate">{organizationTechniqueLabel(cell)}</span>
      <span className="shrink-0 text-[10px] tabular-nums">{meta.mark}</span>
    </span>
  );
}

function assetPhaseLabel(phase: string) {
  switch (phase) {
    case "new_in_stage":
      return "下批";
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
  if (normalized === "js") return "JS";
  if (normalized === "api" || normalized.includes("jsapi")) return "API";
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
  workItems = [],
  defaultBodyHeight = ASSET_COVERAGE_BODY_DEFAULT_HEIGHT,
  fillHeight = false,
  resizable = true,
}: {
  snapshot: StageAssetCoverageSnapshot | null;
  loading: boolean;
  error: string | null;
  workItems?: StageAssetCoverageWorkItem[];
  defaultBodyHeight?: number;
  fillHeight?: boolean;
  resizable?: boolean;
}) {
  const incomingLiveWorkItems = useMemo(
    () => workItems.filter((item) => isLiveWorkStatus(item.status)),
    [workItems]
  );
  const incomingLiveWorkKey = useMemo(
    () => liveWorkItemsKey(incomingLiveWorkItems),
    [incomingLiveWorkItems]
  );
  const [displayLiveWorkItems, setDisplayLiveWorkItems] =
    useState<StageAssetCoverageWorkItem[]>(incomingLiveWorkItems);
  const [coverageView, setCoverageView] = useState<CoverageViewMode>(
    incomingLiveWorkItems.length > 0 ? "active" : "all"
  );
  const [coverageViewTouched, setCoverageViewTouched] = useState(false);
  const [coverageBodyMaxHeight, setCoverageBodyMaxHeight] = useState(
    clampAssetCoverageBodyHeight(defaultBodyHeight)
  );
  const [scrollBodyElement, setScrollBodyElement] = useState<HTMLDivElement | null>(null);
  const setScrollBodyNode = useCallback((node: HTMLDivElement | null) => {
    setScrollBodyElement(node);
  }, []);
  const [readingFrozen, setReadingFrozen] = useState(false);
  const readingFrozenRef = useRef(false);
  const readingFreezeTimerRef = useRef<number | null>(null);
  const [displaySnapshot, setDisplaySnapshot] = useState<StageAssetCoverageSnapshot | null>(
    snapshot
  );
  const [matrixLiveWorkItems, setMatrixLiveWorkItems] =
    useState<StageAssetCoverageWorkItem[]>(incomingLiveWorkItems);
  const displayLiveWorkKey = useMemo(
    () => liveWorkItemsKey(displayLiveWorkItems),
    [displayLiveWorkItems]
  );
  const matrixLiveWorkKey = useMemo(
    () => liveWorkItemsKey(matrixLiveWorkItems),
    [matrixLiveWorkItems]
  );
  const markReadingInteraction = useCallback(() => {
    if (!readingFrozenRef.current) {
      readingFrozenRef.current = true;
      setReadingFrozen(true);
    }
    if (readingFreezeTimerRef.current !== null) {
      window.clearTimeout(readingFreezeTimerRef.current);
    }
    readingFreezeTimerRef.current = window.setTimeout(() => {
      readingFrozenRef.current = false;
      setReadingFrozen(false);
      readingFreezeTimerRef.current = null;
    }, ASSET_COVERAGE_READING_FREEZE_MS);
  }, []);

  useEffect(
    () => () => {
      if (readingFreezeTimerRef.current !== null) {
        window.clearTimeout(readingFreezeTimerRef.current);
      }
    },
    []
  );

  useEffect(() => {
    if (!snapshot) {
      setDisplaySnapshot(snapshot);
      return;
    }
    if (!readingFrozen || !displaySnapshot) {
      setDisplaySnapshot(snapshot);
    }
  }, [snapshot, readingFrozen, displaySnapshot]);

  useEffect(() => {
    if (incomingLiveWorkItems.length > 0) {
      setDisplayLiveWorkItems((previous) =>
        mergeDisplayLiveWorkItems(previous, incomingLiveWorkItems)
      );
      const incomingIds = new Set(incomingLiveWorkItems.map((item) => item.id));
      const timer = window.setTimeout(() => {
        setDisplayLiveWorkItems((previous) => previous.filter((item) => incomingIds.has(item.id)));
      }, LIVE_WORK_RETENTION_MS);
      return () => window.clearTimeout(timer);
    }

    if (displayLiveWorkItems.length === 0) return;
    const timer = window.setTimeout(() => {
      setDisplayLiveWorkItems([]);
    }, LIVE_WORK_RETENTION_MS);
    return () => window.clearTimeout(timer);
  }, [
    incomingLiveWorkItems,
    incomingLiveWorkItems.length,
    incomingLiveWorkKey,
    displayLiveWorkItems.length,
  ]);

  useEffect(() => {
    if (readingFrozen && matrixLiveWorkItems.length > 0) return;
    if (matrixLiveWorkKey !== displayLiveWorkKey) {
      setMatrixLiveWorkItems(displayLiveWorkItems);
    }
  }, [
    displayLiveWorkItems,
    displayLiveWorkKey,
    matrixLiveWorkItems.length,
    matrixLiveWorkKey,
    readingFrozen,
  ]);

  useEffect(() => {
    if (
      !coverageViewTouched &&
      (incomingLiveWorkItems.length > 0 || displayLiveWorkItems.length > 0)
    ) {
      setCoverageView("active");
    }
  }, [coverageViewTouched, incomingLiveWorkItems.length, displayLiveWorkItems.length]);

  if (loading && !displaySnapshot) {
    return (
      <div className="flex min-h-28 items-center rounded-md border border-border/30 bg-background/40 px-3 py-2 text-[11px] text-muted-foreground">
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
  if (!displaySnapshot) return null;
  const organizationRows = displaySnapshot.assets.filter(isOrganizationCoverageRow);
  const assetRows = displaySnapshot.assets.filter((asset) => !isOrganizationCoverageRow(asset));
  if (assetRows.length === 0 && organizationRows.length === 0) {
    return (
      <div className="rounded-md border border-border/30 bg-background/40 px-3 py-2 text-[11px] text-muted-foreground">
        No in-scope assets for this organization.
      </div>
    );
  }

  const summary = displaySnapshot.summary;
  const groups = buildAssetCoverageGroups(assetRows);
  const liveWorkItems = matrixLiveWorkItems;
  const activeGroups = groups.filter((group) => groupMatchesLiveWork(group, liveWorkItems));
  const activeAssetCount = assetRows.filter((row) =>
    liveWorkItems.some((item) => workMatchesAsset(row, item))
  ).length;
  const unmatchedLiveWorkItems = liveWorkItems.filter(
    (item) => !assetRows.some((asset) => workMatchesAsset(asset, item))
  );
  const techniques = assetRows[0]?.coverage ?? [];
  const techniqueColumnCount = Math.max(techniques.length, 1);
  const gridTemplateColumns = `minmax(0,1fr) repeat(${techniqueColumnCount}, minmax(24px,40px))`;
  const effectiveActiveView = coverageView === "active";
  const showCoverageViewToggle =
    assetRows.length > 0 || liveWorkItems.length > 0 || effectiveActiveView;
  const visibleGroups = effectiveActiveView ? activeGroups : groups;
  const visibleGroupsKey = visibleGroups.map((group) => group.key).join("|");
  const handleCoverageViewChange = (mode: CoverageViewMode) => {
    setCoverageViewTouched(true);
    setCoverageView(mode);
  };
  const resizeCoverageBody = (height: number) => {
    setCoverageBodyMaxHeight(clampAssetCoverageBodyHeight(height));
  };
  const handleCoverageResizePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startY = event.clientY;
    const startHeight = coverageBodyMaxHeight;
    const previousCursor = document.body.style.cursor;
    const previousUserSelect = document.body.style.userSelect;
    document.body.style.cursor = "ns-resize";
    document.body.style.userSelect = "none";

    const handlePointerMove = (moveEvent: PointerEvent) => {
      resizeCoverageBody(startHeight + moveEvent.clientY - startY);
    };
    const handlePointerUp = () => {
      window.removeEventListener("pointermove", handlePointerMove);
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousUserSelect;
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp, { once: true });
  };
  const handleCoverageResizeKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      resizeCoverageBody(coverageBodyMaxHeight + ASSET_COVERAGE_BODY_KEYBOARD_STEP);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      resizeCoverageBody(coverageBodyMaxHeight - ASSET_COVERAGE_BODY_KEYBOARD_STEP);
    } else if (event.key === "PageDown") {
      event.preventDefault();
      resizeCoverageBody(coverageBodyMaxHeight + ASSET_COVERAGE_BODY_KEYBOARD_STEP * 3);
    } else if (event.key === "PageUp") {
      event.preventDefault();
      resizeCoverageBody(coverageBodyMaxHeight - ASSET_COVERAGE_BODY_KEYBOARD_STEP * 3);
    } else if (event.key === "Home") {
      event.preventDefault();
      resizeCoverageBody(ASSET_COVERAGE_BODY_MIN_HEIGHT);
    } else if (event.key === "End") {
      event.preventDefault();
      resizeCoverageBody(ASSET_COVERAGE_BODY_MAX_HEIGHT);
    }
  };

  const renderAssetRow = (
    asset: CoverageRow,
    options: {
      child?: boolean;
      host?: boolean;
      relatedWorkItems?: StageAssetCoverageWorkItem[];
      showActivityBadges?: boolean;
    } = {}
  ) => {
    const rowWorkItems = liveWorkItems.filter((item) => workMatchesAsset(asset, item));
    const showActivityBadges = options.showActivityBadges ?? true;
    const metaLabel = rowMetaLabel(asset, options.child);
    return (
      <div
        key={asset.target_id}
        className={cn(
          "grid items-center gap-1.5 border-t border-border/10 px-3 py-1.5 text-[11px]",
          options.host && "bg-muted/15",
          options.child && "bg-background/20",
          rowWorkItems.length > 0 && "bg-[var(--ansi-blue)]/[0.045]"
        )}
        style={{ gridTemplateColumns }}
      >
        <div className={cn("min-w-0", options.child && "border-l border-border/25 pl-2")}>
          <div className="flex min-w-0 items-center gap-1.5">
            {options.host && (
              <span className="shrink-0 rounded bg-muted/40 px-1 py-0.5 text-[8px] font-medium text-muted-foreground/70">
                IP
              </span>
            )}
            {options.child && (
              <span className="shrink-0 rounded bg-emerald-500/10 px-1 py-0.5 text-[8px] font-medium text-emerald-300">
                域名
              </span>
            )}
            <div className="truncate font-medium text-foreground/85" title={asset.value}>
              {asset.value}
            </div>
          </div>
          <div className="mt-0.5 truncate text-[9px] text-muted-foreground/50" title={metaLabel}>
            {metaLabel}
          </div>
          {showActivityBadges && <WorkItemBadges items={rowWorkItems} />}
          {showActivityBadges && options.relatedWorkItems && (
            <WorkItemBadges items={options.relatedWorkItems} related />
          )}
        </div>
        {asset.coverage.map((cell) => {
          const cellWorkItems = rowWorkItems.filter((item) => workMatchesTechnique(cell, item));
          return <CoverageStatusCell key={cell.technique} cell={cell} workItems={cellWorkItems} />;
        })}
      </div>
    );
  };

  const renderSyntheticHostRow = (
    group: AssetCoverageGroup,
    relatedWorkItems: StageAssetCoverageWorkItem[],
    showActivityBadges = true
  ) => (
    <div
      key={`${group.key}:synthetic`}
      className={cn(
        "grid items-center gap-1.5 border-t border-border/10 bg-muted/15 px-3 py-1.5 text-[11px]",
        relatedWorkItems.length > 0 && "bg-emerald-500/[0.045]"
      )}
      style={{ gridTemplateColumns }}
    >
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="shrink-0 rounded bg-muted/40 px-1 py-0.5 text-[8px] font-medium text-muted-foreground/70">
            IP
          </span>
          <div className="truncate font-medium text-foreground/85" title={group.label}>
            {group.label}
          </div>
        </div>
        <div className="mt-0.5 truncate text-[9px] text-muted-foreground/50">
          解析聚合 · {group.childRows.length} 关联资产 · 仅分组，不计覆盖
        </div>
        {showActivityBadges && <WorkItemBadges items={relatedWorkItems} related />}
      </div>
      {Array.from({ length: techniqueColumnCount }).map((_, index) => (
        <span key={index} className="h-5 w-5 justify-self-center" />
      ))}
    </div>
  );

  const renderCoverageGroup = (group: AssetCoverageGroup, showActivityBadges: boolean) => {
    const relatedWorkItems = groupRelatedWorkItems(group, liveWorkItems);
    const rows = groupRows(group);
    const shouldRenderSyntheticHost =
      !group.hostRow && group.resolvedGroup && group.childRows.length > 0;
    return (
      <div
        key={group.key}
        className="border-b border-border/10 last:border-b-0"
        style={{
          containIntrinsicSize: `${estimateCoverageGroupHeight(group)}px`,
          contentVisibility: "auto",
        }}
      >
        {group.hostRow &&
          renderAssetRow(group.hostRow, {
            host: true,
            relatedWorkItems,
            showActivityBadges,
          })}
        {shouldRenderSyntheticHost &&
          renderSyntheticHostRow(group, relatedWorkItems, showActivityBadges)}
        {rows
          .filter((row) => row.target_id !== group.hostRow?.target_id)
          .map((row) =>
            renderAssetRow(row, {
              child: group.resolvedGroup,
              showActivityBadges,
            })
          )}
      </div>
    );
  };

  return (
    <div
      className={cn(
        "rounded-md border border-border/30 bg-background/40",
        fillHeight && "flex h-full min-h-0 flex-col"
      )}
    >
      <div className="flex min-h-12 flex-shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-border/20 px-3 py-2 text-[10px] text-muted-foreground">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="inline-flex h-6 min-w-24 items-center gap-1 rounded bg-background/25 px-1.5 font-medium tabular-nums text-foreground/80">
            <Database className="h-3 w-3 shrink-0" />
            {coverageSummaryText(summary)}
          </span>
          {summary.new_assets > 0 && (
            <span className="inline-flex h-6 min-w-16 items-center justify-center rounded bg-sky-500/15 px-1.5 tabular-nums text-sky-300">
              {summary.new_assets} 下批
            </span>
          )}
          {summary.pending_assets > 0 && (
            <span className="inline-flex h-6 min-w-24 items-center justify-center rounded bg-muted/40 px-1.5 tabular-nums">
              {summary.pending_assets} 未查
            </span>
          )}
          {summary.blocked_assets > 0 && (
            <span className="inline-flex h-6 min-w-20 items-center justify-center rounded bg-amber-500/15 px-1.5 tabular-nums text-amber-300">
              {summary.blocked_assets} 需处理
            </span>
          )}
          <span
            className={cn(
              "inline-flex h-6 min-w-20 items-center justify-center gap-1 rounded bg-[var(--ansi-blue)]/10 px-1.5 tabular-nums text-[var(--ansi-blue)]",
              liveWorkItems.length === 0 && "invisible"
            )}
          >
            <Loader2 className="h-3 w-3 animate-spin" />
            {liveWorkItems.length} 正在做
          </span>
          {showCoverageViewToggle && (
            <button
              type="button"
              className={cn(
                "h-6 rounded border border-border/35 px-1.5 font-medium transition-colors",
                effectiveActiveView
                  ? "bg-background/40 text-muted-foreground/85 hover:bg-muted/20"
                  : "bg-[var(--ansi-blue)]/10 text-[var(--ansi-blue)] hover:bg-[var(--ansi-blue)]/15"
              )}
              onClick={() => handleCoverageViewChange(effectiveActiveView ? "all" : "active")}
            >
              {effectiveActiveView ? "看全部" : "只看运行中"}
            </button>
          )}
        </div>
        <div className="ml-auto flex min-h-6 flex-wrap items-center justify-end gap-1.5 text-[9px] text-muted-foreground/65">
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
      <LiveFocusBar
        activeAssetCount={activeAssetCount}
        activeGroupCount={activeGroups.length}
        assetRows={assetRows}
        coverageView={coverageView}
        liveWorkItems={liveWorkItems}
      />
      <div
        ref={setScrollBodyNode}
        className={cn(
          "overflow-y-auto overflow-x-hidden bg-background/40",
          fillHeight && "min-h-0 flex-1"
        )}
        data-testid="stage-asset-coverage-scroll"
        onPointerDown={markReadingInteraction}
        onScroll={markReadingInteraction}
        onWheel={markReadingInteraction}
        style={
          fillHeight
            ? undefined
            : { height: coverageBodyMaxHeight, maxHeight: coverageBodyMaxHeight }
        }
      >
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
                    <OrganizationCoverageStatusCell key={cell.technique} cell={cell} />
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
            {effectiveActiveView && (
              <div className="bg-[var(--ansi-blue)]/[0.045] px-3 py-1 text-[10px] font-medium text-[var(--ansi-blue)]">
                正在做的资产
              </div>
            )}
            <CoverageGroupsList
              groups={visibleGroups}
              renderCoverageGroup={renderCoverageGroup}
              resetKey={visibleGroupsKey}
              scrollParent={scrollBodyElement}
              showActivityBadges={effectiveActiveView}
            />
            {effectiveActiveView &&
              activeGroups.length === 0 &&
              unmatchedLiveWorkItems.length === 0 && (
                <div className="px-3 py-3 text-[11px] text-muted-foreground/65">
                  暂无匹配到已登记资产的运行中任务。
                </div>
              )}
            {unmatchedLiveWorkItems.length > 0 && (
              <div className="border-t border-border/10 bg-[var(--ansi-blue)]/[0.035] px-3 py-2">
                <div className="mb-1 text-[10px] font-medium text-[var(--ansi-blue)]">
                  运行中但尚未匹配到资产行
                </div>
                <div className="flex flex-wrap gap-1">
                  {unmatchedLiveWorkItems.slice(0, 4).map((item) => (
                    <span
                      key={item.id}
                      className="inline-flex max-w-full items-center gap-1 rounded bg-background/35 px-1.5 py-0.5 text-[9px] text-muted-foreground/80"
                      title={item.primary ?? item.outputPreview ?? item.displayToolName}
                    >
                      <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin text-[var(--ansi-blue)]" />
                      <span className="truncate">
                        {workSubjectLabel(item)} · {workTechniqueLabel(item)} ·{" "}
                        {item.displayToolName}
                      </span>
                    </span>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
      {resizable && !fillHeight && (
        <div
          role="separator"
          aria-label="调整资产覆盖高度"
          aria-orientation="horizontal"
          aria-valuemax={ASSET_COVERAGE_BODY_MAX_HEIGHT}
          aria-valuemin={ASSET_COVERAGE_BODY_MIN_HEIGHT}
          aria-valuenow={coverageBodyMaxHeight}
          className="group flex h-3 cursor-ns-resize items-center justify-center border-t border-border/15 bg-background/35 text-muted-foreground/35 transition-colors hover:bg-muted/15 hover:text-muted-foreground/70 active:bg-muted/20"
          data-testid="stage-asset-coverage-resize-handle"
          onKeyDown={handleCoverageResizeKeyDown}
          onPointerDown={handleCoverageResizePointerDown}
          tabIndex={0}
          title="拖动调整资产覆盖高度"
        >
          <GripHorizontal className="h-3.5 w-3.5" />
        </div>
      )}
    </div>
  );
}

function useStageAssetCoverageSnapshot({
  enabled,
  organizationId,
  pollWhileActive,
  refreshKey,
  sessionId,
  stage,
  stageStartedAt,
}: {
  enabled: boolean;
  organizationId: string;
  pollWhileActive: boolean;
  refreshKey?: string;
  sessionId?: string | null;
  stage: string;
  stageStartedAt?: string | null;
}) {
  const [snapshot, setSnapshot] = useState<StageAssetCoverageSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setSnapshot(null);
    setError(null);
    setLoading(false);
  }, [organizationId, sessionId, stage, stageStartedAt]);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }

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
          stageStartedAt,
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
  }, [enabled, organizationId, pollWhileActive, refreshKey, sessionId, stage, stageStartedAt]);

  return { error, loading, snapshot };
}

function stageAssetCoverageSummaryText(
  snapshot: StageAssetCoverageSnapshot | null,
  loading: boolean,
  error: string | null
) {
  if (error) return "加载失败";
  if (loading && !snapshot) return "加载中";
  if (!snapshot) return "查看";
  return coverageSummaryText(snapshot.summary);
}

function StageAssetCoverageSummaryStrip({
  error,
  loading,
  onOpenCoverage,
  snapshot,
  subtitle,
  title,
  workItems,
}: {
  error: string | null;
  loading: boolean;
  onOpenCoverage?: () => void;
  snapshot: StageAssetCoverageSnapshot | null;
  subtitle?: string;
  title: string;
  workItems: StageAssetCoverageWorkItem[];
}) {
  const liveWorkItems = workItems.filter((item) => isLiveWorkStatus(item.status));
  const primary = liveWorkItems[0];
  const primaryLabel = primary
    ? `${primary.displayToolName} · ${workTechniqueLabel(primary)} · ${workSubjectLabel(primary)}`
    : null;

  return (
    <button
      type="button"
      className="flex min-h-10 w-full min-w-0 items-center gap-2 rounded-md border border-border/30 bg-background/30 px-2.5 py-1.5 text-left transition-colors hover:bg-muted/12"
      onClick={onOpenCoverage}
    >
      <Database className="h-3.5 w-3.5 shrink-0 text-muted-foreground/80" />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="truncate text-[11px] font-semibold text-foreground/85">{title}</span>
          <span className="shrink-0 rounded bg-muted/35 px-1.5 py-0.5 text-[10px] font-medium text-foreground/70">
            {stageAssetCoverageSummaryText(snapshot, loading, error)}
          </span>
          {loading && snapshot && (
            <Loader2 className="h-3 w-3 shrink-0 animate-spin text-[var(--ansi-blue)]/80" />
          )}
          {liveWorkItems.length > 0 && (
            <span className="inline-flex shrink-0 items-center gap-1 rounded bg-[var(--ansi-blue)]/10 px-1.5 py-0.5 text-[10px] text-[var(--ansi-blue)]">
              <Loader2 className="h-3 w-3 animate-spin" />
              {liveWorkItems.length}
            </span>
          )}
        </div>
        <div className="mt-0.5 truncate text-[10px] text-muted-foreground/60">
          {primaryLabel ?? subtitle ?? "资产覆盖"}
        </div>
      </div>
      <span className="shrink-0 rounded border border-border/35 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground/80">
        <ChevronRight className="h-3.5 w-3.5" />
      </span>
    </button>
  );
}

export function StageAssetCoverageBlock({
  onBackToRun,
  displayMode = "collapsible",
  onOpenCoverage,
  organizationId,
  stage,
  sessionId,
  stageStartedAt,
  title = "资产覆盖",
  subtitle,
  pollWhileActive = false,
  defaultExpanded = false,
  panelBodyHeight = 360,
  workItems = [],
  className,
}: {
  onBackToRun?: () => void;
  displayMode?: "collapsible" | "summary" | "panel";
  onOpenCoverage?: () => void;
  organizationId: string;
  stage: string;
  sessionId?: string | null;
  stageStartedAt?: string | null;
  title?: string;
  subtitle?: string;
  pollWhileActive?: boolean;
  defaultExpanded?: boolean;
  panelBodyHeight?: number;
  workItems?: StageAssetCoverageWorkItem[];
  className?: string;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const shouldLoadCoverage = displayMode === "summary" || displayMode === "panel" || expanded;
  const coverageRefreshKey = useMemo(() => workItemsRefreshKey(workItems), [workItems]);
  const { error, loading, snapshot } = useStageAssetCoverageSnapshot({
    enabled: shouldLoadCoverage,
    organizationId,
    pollWhileActive,
    refreshKey: coverageRefreshKey,
    sessionId,
    stage,
    stageStartedAt,
  });

  useEffect(() => {
    setExpanded(defaultExpanded);
  }, [defaultExpanded, organizationId, sessionId, stage]);

  useEffect(() => {
    if (defaultExpanded) setExpanded(true);
  }, [defaultExpanded]);

  const summary = snapshot ? snapshot.summary : null;
  const summaryText = summary
    ? coverageSummaryText(summary)
    : stageAssetCoverageSummaryText(snapshot, shouldLoadCoverage && loading, error);
  const liveWorkCount = workItems.filter((item) => isLiveWorkStatus(item.status)).length;

  if (displayMode === "summary") {
    return (
      <StageAssetCoverageSummaryStrip
        error={error}
        loading={loading}
        onOpenCoverage={onOpenCoverage}
        snapshot={snapshot}
        subtitle={subtitle}
        title={title}
        workItems={workItems}
      />
    );
  }

  if (displayMode === "panel") {
    return (
      <section
        className={cn(
          "flex h-full min-h-0 flex-col rounded-md border border-border/30 bg-background/25",
          className
        )}
      >
        <div className="flex min-h-10 w-full min-w-0 flex-shrink-0 items-center justify-between gap-3 border-b border-border/20 px-2.5 py-1.5 text-left">
          <div className="flex min-w-0 items-center gap-2">
            <Database className="h-3.5 w-3.5 shrink-0 text-muted-foreground/80" />
            <div className="min-w-0">
              <div className="truncate text-[11px] font-semibold text-foreground/85">{title}</div>
              {subtitle && (
                <div className="mt-0.5 truncate text-[10px] text-muted-foreground/60">
                  {subtitle}
                </div>
              )}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1.5 text-[10px] text-muted-foreground/70">
            {onBackToRun && (
              <button
                type="button"
                className="inline-flex h-6 items-center gap-1 rounded border border-border/35 px-1.5 font-medium text-muted-foreground/80 transition-colors hover:bg-muted/15 hover:text-foreground/85"
                onClick={onBackToRun}
              >
                <ListTree className="h-3 w-3" />
                运行流
              </button>
            )}
            {loading && <Loader2 className="h-3 w-3 animate-spin text-[var(--ansi-blue)]/80" />}
            <span className="inline-flex h-6 min-w-24 items-center justify-center rounded bg-muted/35 px-1.5 font-medium tabular-nums text-foreground/70">
              {summaryText}
            </span>
            <span
              className={cn(
                "inline-flex h-6 min-w-10 items-center justify-center gap-1 rounded bg-[var(--ansi-blue)]/10 px-1.5 tabular-nums text-[var(--ansi-blue)]",
                liveWorkCount === 0 && "invisible"
              )}
            >
              {liveWorkCount > 0 && (
                <>
                  <Loader2 className="h-3 w-3 animate-spin" />
                  {liveWorkCount}
                </>
              )}
            </span>
            {pollWhileActive && !loading && (
              <span className="inline-flex h-6 min-w-12 items-center justify-center rounded bg-[var(--ansi-blue)]/10 px-1.5 text-[var(--ansi-blue)]">
                Live
              </span>
            )}
            {pollWhileActive && loading && (
              <span className="inline-flex h-6 min-w-12 items-center justify-center rounded bg-muted/25 px-1.5 text-muted-foreground/50">
                Live
              </span>
            )}
          </div>
        </div>
        <div className="min-h-0 flex-1 p-2">
          <StageAssetCoveragePanel
            snapshot={snapshot}
            loading={loading}
            error={error}
            workItems={workItems}
            defaultBodyHeight={panelBodyHeight}
            fillHeight
            resizable={false}
          />
        </div>
      </section>
    );
  }

  return (
    <section className={cn("rounded-md border border-border/30 bg-background/25", className)}>
      <button
        type="button"
        className="flex min-h-10 w-full min-w-0 items-center justify-between gap-3 px-2.5 py-1.5 text-left hover:bg-muted/15"
        onClick={() => {
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
          <span className="inline-flex h-6 min-w-24 items-center justify-center rounded bg-muted/35 px-1.5 font-medium tabular-nums text-foreground/70">
            {summaryText}
          </span>
          {summary && summary.new_assets > 0 && (
            <span className="inline-flex h-6 min-w-8 items-center justify-center rounded bg-sky-500/15 px-1.5 tabular-nums text-sky-300">
              下批 {summary.new_assets}
            </span>
          )}
          {summary && summary.blocked_assets > 0 && (
            <span className="inline-flex h-6 min-w-16 items-center justify-center rounded bg-amber-500/15 px-1.5 tabular-nums text-amber-300">
              {summary.blocked_assets} 需处理
            </span>
          )}
          <span
            className={cn(
              "inline-flex h-6 min-w-10 items-center justify-center gap-1 rounded bg-[var(--ansi-blue)]/10 px-1.5 tabular-nums text-[var(--ansi-blue)]",
              liveWorkCount === 0 && "invisible"
            )}
          >
            {liveWorkCount > 0 && (
              <>
                <Loader2 className="h-3 w-3 animate-spin" />
                {liveWorkCount}
              </>
            )}
          </span>
          {pollWhileActive && expanded && !loading && (
            <span className="inline-flex h-6 min-w-12 items-center justify-center rounded bg-[var(--ansi-blue)]/10 px-1.5 text-[var(--ansi-blue)]">
              Live
            </span>
          )}
        </div>
      </button>
      {expanded && (
        <div className="border-t border-border/20 p-2">
          <StageAssetCoveragePanel
            snapshot={snapshot}
            loading={loading}
            error={error}
            workItems={workItems}
          />
        </div>
      )}
    </section>
  );
}
