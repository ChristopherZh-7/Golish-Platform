import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  Circle,
  GitBranch,
  Loader2,
  RefreshCw,
  ShieldCheck,
  Workflow,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ApiError } from "@/lib/api";
import { translateErrorCode } from "@/lib/api/error-codes";
import {
  type GetStageAssetCoverageArgs,
  getStageAssetCoverage,
  type StageAssetCoverageSnapshot,
} from "@/lib/api/stage-coverage";
import {
  getStageTeamReadModel,
  resolveStageTeamRecovery,
  type StageTeamReadModel,
  type StageTeamReadRequest,
  type StageTeamRecoveryResolveRequest,
  type StageTeamRecoveryResolveResponse,
} from "@/lib/api/stage-team";
import { cn } from "@/lib/utils";

export interface StageTeamReadApi {
  getReadModel: (request: StageTeamReadRequest) => Promise<StageTeamReadModel>;
  getCoverage?: (request: GetStageAssetCoverageArgs) => Promise<StageAssetCoverageSnapshot>;
  resolveRecovery: (
    request: StageTeamRecoveryResolveRequest
  ) => Promise<StageTeamRecoveryResolveResponse>;
}

const defaultApi: StageTeamReadApi = {
  getReadModel: getStageTeamReadModel,
  getCoverage: getStageAssetCoverage,
  resolveRecovery: resolveStageTeamRecovery,
};

interface RecoveryActionState {
  status: "loading" | "error" | "success";
  message: string;
}

export interface StageTeamRunViewProps {
  operationId: string;
  stageExecutionId: string;
  refreshVersion?: string | number;
  api?: StageTeamReadApi;
  agentRequestIdsByWorker?: Readonly<Record<string, string>>;
  onOpenAgent?: (agentRequestId: string) => void;
}

type StageTeamWorkItem = NonNullable<
  StageTeamReadModel["units"][number]["plan"]
>["workItems"][number];

interface RecoveryEntry {
  unit: StageTeamReadModel["units"][number];
  plan: NonNullable<StageTeamReadModel["units"][number]["plan"]>;
  item: StageTeamWorkItem;
  worker: StageTeamWorkItem["workers"][number];
}

function message(error: unknown): string {
  if (error instanceof ApiError) {
    return translateErrorCode(error.code, error.message);
  }
  return error instanceof Error ? error.message : String(error);
}

function short(value: string | null | undefined, size = 10): string {
  if (!value) return "—";
  return value.length <= size ? value : `${value.slice(0, size)}…`;
}

function countLabel(count: number, singular: string, plural = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

function newRecoveryRequestId(workerRunId: string): string {
  if (typeof globalThis.crypto?.randomUUID === "function") {
    return globalThis.crypto.randomUUID();
  }
  return `stage-team-recovery-${workerRunId}-${Date.now().toString(36)}`;
}

function StatusMark({ status }: { status: string }) {
  if (status === "passed" || status === "completed") {
    return <CheckCircle2 className="h-3.5 w-3.5 text-emerald-300" />;
  }
  if (status === "running" || status === "claimed") {
    return <Loader2 className="h-3.5 w-3.5 animate-spin text-sky-300" />;
  }
  if (status.includes("blocked") || status.includes("recovery") || status === "exhausted") {
    return <AlertTriangle className="h-3.5 w-3.5 text-amber-300" />;
  }
  return <Circle className="h-3.5 w-3.5 text-muted-foreground/60" />;
}

function isCompanyController(item: StageTeamWorkItem): boolean {
  return item.stableKey === "leader:primary";
}

function companyControllerState(item: StageTeamWorkItem): {
  label: string;
  status: string;
  className: string;
} {
  if (item.status === "running" || item.status === "claimed") {
    return { label: "Controller 运行中", status: "running", className: "text-cyan-300" };
  }
  if (item.status === "waiting_dependency") {
    return {
      label: "Controller 正在监控 SubAgent",
      status: "running",
      className: "text-cyan-300",
    };
  }
  if (item.status === "queued" || item.status === "retry_pending") {
    return { label: "Controller 排队中", status: "queued", className: "text-indigo-300" };
  }
  if (
    item.status.includes("blocked") ||
    item.status === "recovery_required" ||
    item.status === "exhausted"
  ) {
    return { label: "Controller 已阻塞", status: "blocked", className: "text-amber-300" };
  }
  if (item.status === "completed") {
    return { label: "Controller 已完成", status: "completed", className: "text-emerald-300" };
  }
  return { label: "Controller 等待启动", status: item.status, className: "text-muted-foreground" };
}

function companyGateState(unit: StageTeamReadModel["units"][number]): {
  label: string;
  className: string;
} {
  if (unit.status === "passed" && unit.gate.finalHandoffId) {
    return { label: "Gate 已通过", className: "text-emerald-300" };
  }
  if (unit.status.includes("blocked") || unit.gate.status.includes("blocked")) {
    return { label: "Gate 已阻塞", className: "text-amber-300" };
  }
  return { label: "Gate 未完成", className: "text-slate-300" };
}

function vulnGateState(unit: StageTeamReadModel["units"][number]): {
  label: string;
  className: string;
} {
  if (unit.status === "passed" && unit.gate.finalHandoffId) {
    return { label: "证据门禁已通过", className: "text-emerald-300" };
  }
  if (unit.status.includes("blocked") || unit.gate.status.includes("blocked")) {
    return { label: "证据门禁已阻塞", className: "text-amber-300" };
  }
  return { label: "证据门禁未完成", className: "text-slate-300" };
}

function vulnSchedulerState(
  unit: StageTeamReadModel["units"][number],
  children: StageTeamWorkItem[]
): { label: string; status: string; className: string } {
  if (unit.status === "passed" && unit.gate.finalHandoffId) {
    return { label: "扫描已完成", status: "completed", className: "text-emerald-300" };
  }
  if (
    children.some(
      (item) =>
        item.status.includes("blocked") ||
        item.status === "recovery_required" ||
        item.status === "exhausted"
    )
  ) {
    return { label: "扫描队列需要处理", status: "blocked", className: "text-amber-300" };
  }
  if (children.some((item) => item.status === "running" || item.status === "claimed")) {
    return { label: "正在执行扫描分片", status: "running", className: "text-violet-200" };
  }
  if (
    children.some((item) => ["queued", "retry_pending", "waiting_dependency"].includes(item.status))
  ) {
    return { label: "扫描分片排队中", status: "queued", className: "text-indigo-300" };
  }
  return { label: "正在核对证据门禁", status: "running", className: "text-violet-200" };
}

function currentWorker(item: StageTeamWorkItem) {
  return (
    item.workers.find((worker) =>
      ["running", "claimed", "recovery_required"].includes(worker.status)
    ) ?? item.workers[item.workers.length - 1]
  );
}

function workerAttemptLane(item: StageTeamWorkItem, worker: StageTeamWorkItem["workers"][number]) {
  if (worker.recoveryState === "manual_required" || worker.status === "recovery_required") {
    return "operator recovery";
  }
  if (
    ["running", "claimed"].includes(worker.status) &&
    (worker.generation > 1 || worker.attemptEpoch > 1 || item.status === "retry_pending")
  ) {
    return "current retry";
  }
  if (["failed", "exhausted", "killed", "cancelled"].includes(worker.status)) {
    return "historical attempt failed";
  }
  return null;
}

function UnsupportedLegacyTeamRun() {
  return (
    <div
      role="status"
      className="mt-2 flex items-start gap-2 rounded border border-amber-500/30 bg-amber-500/[0.06] px-2.5 py-2 text-[11px] text-amber-200"
    >
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <span>旧版固定 Team 运行已不再支持，请重新运行本阶段以启动 Company Controller。</span>
    </div>
  );
}

export function StageTeamRunView({
  operationId,
  stageExecutionId,
  refreshVersion = 0,
  api = defaultApi,
  agentRequestIdsByWorker = {},
  onOpenAgent,
}: StageTeamRunViewProps) {
  const [state, setState] = useState<StageTeamReadModel | undefined>();
  const [loadedIdentity, setLoadedIdentity] = useState<string | null>(null);
  const [errorState, setErrorState] = useState<{ identity: string; text: string } | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [coverageByOrg, setCoverageByOrg] = useState<Record<string, StageAssetCoverageSnapshot>>(
    {}
  );
  const [coverageLoading, setCoverageLoading] = useState(false);
  const [coverageError, setCoverageError] = useState<string | null>(null);
  const [showSchedulerDetails, setShowSchedulerDetails] = useState(false);
  const [recoveryActions, setRecoveryActions] = useState<Record<string, RecoveryActionState>>({});
  const [recoveryNotice, setRecoveryNotice] = useState<{ identity: string; text: string } | null>(
    null
  );
  const recoveryRequestIds = useRef(new Map<string, string>());
  const sequence = useRef(0);
  const identity = `${operationId}:${stageExecutionId}`;
  const model = loadedIdentity === identity ? state : undefined;
  const error = errorState?.identity === identity ? errorState.text : null;

  const load = useCallback(async () => {
    const current = ++sequence.current;
    setRefreshing(true);
    setErrorState(null);
    try {
      const next = await api.getReadModel({ operationId, stageExecutionId });
      if (sequence.current !== current) return;
      setState(next);
      setLoadedIdentity(identity);
      if (next.stageKind === "vuln_triage" && api.getCoverage) {
        setCoverageByOrg({});
        setCoverageLoading(true);
        setCoverageError(null);
        try {
          const snapshots = await Promise.all(
            next.units.map((unit) =>
              api.getCoverage!({
                operationId,
                organizationId: unit.organizationId,
                stage: next.stageKind,
                stageStartedAt: next.startedAt,
              })
            )
          );
          if (sequence.current !== current) return;
          setCoverageByOrg(
            Object.fromEntries(snapshots.map((snapshot) => [snapshot.organization_id, snapshot]))
          );
        } catch (cause) {
          if (sequence.current !== current) return;
          setCoverageByOrg({});
          setCoverageError(message(cause));
        } finally {
          if (sequence.current === current) setCoverageLoading(false);
        }
      } else {
        setCoverageByOrg({});
        setCoverageError(null);
        setCoverageLoading(false);
      }
    } catch (cause) {
      if (sequence.current !== current) return;
      setState(undefined);
      setLoadedIdentity(identity);
      setCoverageByOrg({});
      setCoverageLoading(false);
      setCoverageError(null);
      setErrorState({ identity, text: message(cause) });
    } finally {
      if (sequence.current === current) setRefreshing(false);
    }
  }, [api, identity, operationId, stageExecutionId]);

  const resolveRecovery = useCallback(
    async (
      unit: StageTeamReadModel["units"][number],
      plan: NonNullable<StageTeamReadModel["units"][number]["plan"]>,
      item: NonNullable<StageTeamReadModel["units"][number]["plan"]>["workItems"][number],
      worker: NonNullable<
        StageTeamReadModel["units"][number]["plan"]
      >["workItems"][number]["workers"][number]
    ) => {
      if (!worker.activeToolCallId || worker.recoveryState !== "manual_required") return;
      let requestId = recoveryRequestIds.current.get(worker.workerRunId);
      if (!requestId) {
        requestId = newRecoveryRequestId(worker.workerRunId);
        recoveryRequestIds.current.set(worker.workerRunId, requestId);
      }
      setRecoveryNotice(null);
      setRecoveryActions((current) => ({
        ...current,
        [worker.workerRunId]: {
          status: "loading",
          message: "正在安全封存这次没有确认结果的调用，不会自动重放工具…",
        },
      }));
      try {
        await api.resolveRecovery({
          requestId,
          operationId,
          stageExecutionId,
          stageRunUnitId: unit.stageRunUnitId,
          scopeSnapshotId: unit.scopeSnapshotId,
          stageTeamPlanId: plan.stageTeamPlanId,
          workItemId: item.workItemId,
          workerRunId: worker.workerRunId,
          toolCallRecordId: worker.activeToolCallId,
          expectedWorkItemRowVersion: item.rowVersion,
          expectedCheckpointVersion: worker.checkpointVersion,
          expectedAttemptEpoch: worker.attemptEpoch,
        });
        setRecoveryActions((current) => ({
          ...current,
          [worker.workerRunId]: {
            status: "success",
            message: "本项已安全记为结果未知，正在重新读取状态…",
          },
        }));
        setRecoveryNotice({
          identity,
          text: "本项已安全记为结果未知且不会重放。若仍有待恢复项，请逐一处理；全部处理后可发送“继续”恢复剩余任务，或使用“重置阶段”重新开始。",
        });
        await load();
      } catch (cause) {
        setRecoveryActions((current) => ({
          ...current,
          [worker.workerRunId]: {
            status: "error",
            message: message(cause),
          },
        }));
      }
    },
    [api, identity, load, operationId, stageExecutionId]
  );

  useEffect(() => {
    setRecoveryActions({});
    setRecoveryNotice(null);
    recoveryRequestIds.current.clear();
  }, [identity]);

  useEffect(() => {
    void refreshVersion;
    void load();
    return () => {
      sequence.current += 1;
    };
  }, [load, refreshVersion]);

  const summary = useMemo(() => {
    const plans = model?.units.flatMap((unit) => (unit.plan ? [unit.plan] : [])) ?? [];
    const items = plans
      .flatMap((plan) => plan.workItems)
      .filter((item) => !item.isAggregator && !isCompanyController(item));
    const isReturned = (item: StageTeamWorkItem) =>
      item.output?.businessDisposition === "found" ||
      item.output?.businessDisposition === "checked_empty";
    const isBlocked = (item: StageTeamWorkItem) =>
      item.output?.businessDisposition === "blocked" ||
      item.status.includes("blocked") ||
      item.status === "recovery_required" ||
      item.status === "exhausted";
    return {
      plans: plans.length,
      items: items.length,
      collecting: items.filter((item) => item.status === "running" || item.status === "claimed")
        .length,
      recovery: items.filter((item) => item.status === "recovery_required").length,
      returned: items.filter(isReturned).length,
      blocked: items.filter(isBlocked).length,
      stagePassed:
        (model?.units.length ?? 0) > 0 &&
        (model?.units.every(
          (unit) => unit.status === "passed" && Boolean(unit.gate.finalHandoffId)
        ) ??
          false),
      stageBlocked:
        model?.units.some(
          (unit) => unit.status.includes("blocked") || unit.gate.status.includes("blocked")
        ) ?? false,
    };
  }, [model]);
  const coverageSummary = useMemo(() => {
    const cells = Object.values(coverageByOrg).flatMap((snapshot) =>
      snapshot.assets.flatMap((asset) => asset.coverage)
    );
    const terminal = cells.filter((cell) =>
      ["found", "checked_empty", "blocked", "not_applicable"].includes(cell.state)
    ).length;
    return {
      total: cells.length,
      terminal,
      remaining: cells.length - terminal,
    };
  }, [coverageByOrg]);
  const isVulnStage = model?.stageKind === "vuln_triage";
  const schedulerDetailsVisible = !isVulnStage && (showSchedulerDetails || summary.recovery > 0);
  const recoveryEntries = useMemo<RecoveryEntry[]>(
    () =>
      model?.units.flatMap((unit) => {
        if (!unit.plan) return [];
        const plan = unit.plan;
        return plan.workItems.flatMap((item) =>
          item.workers
            .filter(
              (worker) =>
                worker.recoveryState === "manual_required" && worker.activeToolCallId !== null
            )
            .map((worker) => ({ unit, plan, item, worker }))
        );
      }) ?? [],
    [model]
  );
  const hasDurableRecoveryBlock =
    recoveryEntries.length === 0 &&
    !summary.stagePassed &&
    (model?.units.some((unit) =>
      unit.plan?.workItems.some((item) =>
        item.output?.blockerCodes.includes("STAGE_TEAM_ACTIVE_TOOL_RECOVERY_BLOCKED")
      )
    ) ??
      false);

  useEffect(() => {
    const activeWorkerIds = new Set(recoveryEntries.map(({ worker }) => worker.workerRunId));
    for (const workerRunId of recoveryRequestIds.current.keys()) {
      if (!activeWorkerIds.has(workerRunId)) recoveryRequestIds.current.delete(workerRunId);
    }
    setRecoveryActions((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([workerRunId]) => activeWorkerIds.has(workerRunId))
      )
    );
  }, [recoveryEntries]);

  if (model === undefined && error === null) {
    return (
      <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading durable Team scheduler…
      </div>
    );
  }

  if (model === undefined) {
    return (
      <div className="rounded border border-red-500/30 bg-red-500/5 p-3 text-xs">
        <div role="alert" className="flex items-center gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" />{" "}
          {error ?? "Team scheduler state is unavailable."}
        </div>
        <button
          type="button"
          className="mt-2 text-accent hover:underline"
          onClick={() => void load()}
        >
          Retry DB read
        </button>
      </div>
    );
  }

  if (model.units.length === 0) {
    return (
      <div className="rounded border border-border/30 p-4 text-center text-xs text-muted-foreground">
        <Workflow className="mx-auto mb-2 h-4 w-4" />
        No StageRunUnits exist for this exact operation and Stage execution.
      </div>
    );
  }

  return (
    <section className="space-y-2 rounded border border-border/30 bg-muted/10 p-2.5">
      <header className="flex flex-wrap items-center gap-2 text-[11px]">
        {isVulnStage ? (
          <ShieldCheck className="h-3.5 w-3.5 text-violet-300" />
        ) : (
          <Workflow className="h-3.5 w-3.5 text-cyan-300" />
        )}
        <h3 className="font-semibold">{isVulnStage ? "漏洞扫描进度" : "任务执行进度"}</h3>
        <span
          className={cn(
            "rounded px-1.5 py-0.5",
            isVulnStage ? "bg-violet-500/10 text-violet-200" : "bg-cyan-500/10 text-cyan-300"
          )}
        >
          {isVulnStage ? "扫描分片" : "采集"} {summary.returned}/{summary.items} 已返回
        </span>
        {model.stageKind === "vuln_triage" && coverageLoading && (
          <span className="inline-flex items-center gap-1 rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-300">
            <Loader2 className="h-3 w-3 animate-spin" /> 正在读取扫描进度
          </span>
        )}
        {model.stageKind === "vuln_triage" && coverageError && (
          <span role="alert" className="rounded bg-red-500/10 px-1.5 py-0.5 text-red-300">
            扫描进度读取失败：{coverageError}
          </span>
        )}
        {model.stageKind === "vuln_triage" && !coverageLoading && !coverageError && (
          <span className="rounded bg-violet-500/10 px-1.5 py-0.5 font-medium text-violet-200">
            {coverageSummary.total === 0
              ? "当前没有待扫描项"
              : `${coverageSummary.terminal}/${coverageSummary.total} cells 终态 · 剩余 ${coverageSummary.remaining}`}
          </span>
        )}
        {summary.collecting > 0 && (
          <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-300">
            {summary.collecting} 个{isVulnStage ? "执行中" : "采集中"}
          </span>
        )}
        {summary.blocked > 0 && (
          <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-300">
            {summary.blocked} 个{isVulnStage ? "需处理" : "阻塞"}
          </span>
        )}
        <span
          className={cn(
            "rounded px-1.5 py-0.5",
            summary.stagePassed
              ? "bg-emerald-500/10 text-emerald-300"
              : summary.stageBlocked
                ? "bg-amber-500/10 text-amber-300"
                : "bg-slate-500/10 text-slate-300"
          )}
        >
          {summary.stagePassed
            ? isVulnStage
              ? "证据门禁已通过"
              : "阶段已通过"
            : summary.stageBlocked
              ? isVulnStage
                ? "证据门禁已阻塞"
                : "阶段被 Gate 阻塞"
              : isVulnStage
                ? "证据门禁未完成"
                : "阶段未完成"}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          {!isVulnStage && (
            <button
              type="button"
              className="rounded border border-border/30 px-2 py-1 text-muted-foreground hover:text-foreground"
              aria-expanded={schedulerDetailsVisible}
              onClick={() => setShowSchedulerDetails((current) => !current)}
            >
              {schedulerDetailsVisible ? "收起调度详情" : "展开调度详情"}
            </button>
          )}
          <button
            type="button"
            aria-label="刷新任务进度"
            className="rounded border border-border/30 p-1"
            disabled={refreshing}
            onClick={() => void load()}
          >
            <RefreshCw className={cn("h-3 w-3", refreshing && "animate-spin")} />
          </button>
        </div>
      </header>

      {isVulnStage && !coverageLoading && !coverageError && coverageSummary.total > 0 && (
        <div className="rounded border border-violet-500/20 bg-violet-500/[0.04] px-2.5 py-2">
          <div className="mb-1.5 flex items-center gap-2 text-[10px]">
            <span className="font-medium text-violet-100">证据覆盖</span>
            <span className="text-muted-foreground">
              {coverageSummary.terminal}/{coverageSummary.total} cells
            </span>
            <span className="ml-auto text-violet-200">{coverageSummary.remaining} 待检查</span>
          </div>
          <div
            role="progressbar"
            aria-label="证据覆盖"
            aria-valuemin={0}
            aria-valuemax={coverageSummary.total}
            aria-valuenow={coverageSummary.terminal}
            className="h-1.5 overflow-hidden rounded-full bg-violet-950/70"
          >
            <div
              className="h-full rounded-full bg-gradient-to-r from-violet-500 to-cyan-400 transition-[width]"
              style={{
                width: `${Math.round((coverageSummary.terminal / coverageSummary.total) * 100)}%`,
              }}
            />
          </div>
        </div>
      )}

      {recoveryEntries.length > 0 && (
        <section
          aria-label="中断恢复"
          className="rounded border border-amber-500/35 bg-amber-500/[0.07] p-2.5 text-[11px]"
        >
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-300" />
            <div className="min-w-0 flex-1">
              <h4 className="font-semibold text-amber-100">检测到上次运行中断</h4>
              <p className="mt-1 text-amber-100/80">
                有工具在应用关闭前没有返回可确认的结果。请逐项解除中断状态；系统会把本次结果安全标记为未知，不会自动重放工具。
              </p>
              <div className="mt-2 space-y-2">
                {recoveryEntries.map(({ unit, plan, item, worker }) => {
                  const recoveryAction = recoveryActions[worker.workerRunId];
                  const isPending =
                    recoveryAction?.status === "loading" || recoveryAction?.status === "success";
                  return (
                    <div
                      key={worker.workerRunId}
                      className="rounded border border-amber-400/20 bg-background/35 p-2"
                    >
                      <div className="flex flex-wrap items-center gap-1.5 text-muted-foreground">
                        <span className="font-medium text-foreground/85">
                          {unit.organizationName}
                        </span>
                        <span>
                          {item.role} · {item.kind}
                        </span>
                        <span>Worker {short(worker.workerRunId)}</span>
                      </div>
                      <button
                        type="button"
                        className="mt-1.5 rounded border border-amber-400/40 px-2 py-1 text-left text-amber-200 disabled:cursor-wait disabled:opacity-60"
                        disabled={isPending}
                        onClick={() => void resolveRecovery(unit, plan, item, worker)}
                      >
                        {recoveryAction?.status === "loading"
                          ? "正在安全封存未知结果…"
                          : recoveryAction?.status === "error"
                            ? "重试解除中断状态"
                            : recoveryAction?.status === "success"
                              ? "正在确认中断状态…"
                              : "解除中断状态"}
                      </button>
                      {recoveryAction && (
                        <div
                          role={recoveryAction.status === "error" ? "alert" : "status"}
                          className={cn(
                            "mt-1",
                            recoveryAction.status === "error"
                              ? "text-red-300"
                              : "text-muted-foreground"
                          )}
                        >
                          {recoveryAction.message}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        </section>
      )}

      {(recoveryNotice?.identity === identity || hasDurableRecoveryBlock) && (
        <div
          role="status"
          className="rounded border border-emerald-500/30 bg-emerald-500/[0.06] px-2.5 py-2 text-[11px] text-emerald-200"
        >
          {recoveryNotice?.identity === identity
            ? recoveryNotice.text
            : "此前的中断项已安全记为结果未知且不会重放。现在可发送“继续”恢复剩余任务，或使用“重置阶段”重新开始。"}
        </div>
      )}

      <div className="space-y-2">
        {model.units.map((unit) => (
          <article
            key={unit.stageRunUnitId}
            className="rounded border border-border/30 bg-background/40 p-2.5"
          >
            <div className="flex flex-wrap items-center gap-2 text-[11px]">
              <StatusMark status={unit.status} />
              <span className="font-medium">{unit.organizationName}</span>
              {unit.gate.finalHandoffId && (
                <span className="inline-flex items-center gap-1 text-emerald-300">
                  <ShieldCheck className="h-3 w-3" /> 已完成并封存 ·{" "}
                  {unit.gate.finalHandoffEvidenceCount}
                  条证据
                </span>
              )}
            </div>

            {!unit.plan ? (
              <UnsupportedLegacyTeamRun />
            ) : (
              (() => {
                const controller = unit.plan.workItems.find(isCompanyController);
                if (controller) {
                  const children = unit.plan.workItems.filter(
                    (item) => !isCompanyController(item) && !item.isAggregator
                  );
                  const controllerWorker = currentWorker(controller);
                  const controllerRequestId = controllerWorker
                    ? agentRequestIdsByWorker[controllerWorker.workerRunId]
                    : undefined;
                  const activeChildren = children.filter(
                    (item) => item.status === "running" || item.status === "claimed"
                  ).length;
                  const queuedChildren = children.filter((item) =>
                    ["queued", "retry_pending", "waiting_dependency"].includes(item.status)
                  ).length;
                  const completedChildren = children.filter(
                    (item) => item.status === "completed"
                  ).length;
                  const blockedChildren = children.filter(
                    (item) =>
                      item.status.includes("blocked") ||
                      item.status === "recovery_required" ||
                      item.status === "exhausted"
                  ).length;
                  const attemptLanes = children.flatMap((item) =>
                    item.workers.map((worker) => workerAttemptLane(item, worker))
                  );
                  const workerHistoricalFailedAttempts = attemptLanes.filter(
                    (lane) => lane === "historical attempt failed"
                  ).length;
                  const currentRetries = attemptLanes.filter(
                    (lane) => lane === "current retry"
                  ).length;
                  const operatorRecoveries = attemptLanes.filter(
                    (lane) => lane === "operator recovery"
                  ).length;
                  const unitCoverage = coverageByOrg[unit.organizationId];
                  const unitCells = unitCoverage?.assets.flatMap((asset) => asset.coverage) ?? [];
                  const unitTerminalCells = unitCells.filter((cell) =>
                    ["found", "checked_empty", "blocked", "not_applicable"].includes(cell.state)
                  ).length;
                  const historicalFailedCells = unitCells.filter((cell) =>
                    ["partial", "error"].includes(cell.state)
                  ).length;
                  const historicalFailedAttempts = unitCoverage
                    ? historicalFailedCells
                    : workerHistoricalFailedAttempts;
                  const controllerDisplay = isVulnStage
                    ? vulnSchedulerState(unit, children)
                    : companyControllerState(controller);
                  const gateDisplay = isVulnStage ? vulnGateState(unit) : companyGateState(unit);
                  return (
                    <div
                      className={cn(
                        "mt-2 rounded px-2.5 py-2.5 text-[11px]",
                        isVulnStage
                          ? "border border-violet-500/25 bg-violet-500/[0.04]"
                          : "border border-cyan-500/25 bg-cyan-500/[0.04]"
                      )}
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <StatusMark status={controllerDisplay.status} />
                        {isVulnStage ? (
                          <Workflow className="h-3.5 w-3.5 text-violet-300" />
                        ) : (
                          <Bot className="h-3.5 w-3.5 text-cyan-300" />
                        )}
                        <span className="font-medium">
                          {isVulnStage ? "漏洞扫描调度器" : "Company Controller"}
                        </span>
                        <span
                          className={cn(
                            "rounded bg-muted/35 px-1.5 py-0.5",
                            controllerDisplay.className
                          )}
                        >
                          {controllerDisplay.label}
                        </span>
                        {onOpenAgent && controllerRequestId && (
                          <button
                            type="button"
                            aria-label={isVulnStage ? "查看 AI 运行流" : "查看 Controller 运行流"}
                            className={cn(
                              "rounded px-1.5 py-0.5",
                              isVulnStage
                                ? "border border-violet-400/30 text-violet-200 hover:text-violet-100"
                                : "border border-cyan-400/30 text-cyan-300 hover:text-cyan-200"
                            )}
                            onClick={() => onOpenAgent(controllerRequestId)}
                          >
                            {isVulnStage ? "查看 AI 运行流" : "查看 Controller 运行流"}
                          </button>
                        )}
                        <span className={cn("ml-auto font-medium", gateDisplay.className)}>
                          {gateDisplay.label}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-1.5 text-muted-foreground">
                        <span className="rounded bg-muted/30 px-1.5 py-0.5">
                          {children.length} 个{isVulnStage ? "扫描分片" : " SubAgent"}
                        </span>
                        {activeChildren > 0 && (
                          <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-300">
                            {activeChildren} 个{isVulnStage ? "执行中" : "运行中"}
                          </span>
                        )}
                        {completedChildren > 0 && (
                          <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-300">
                            {completedChildren} 个已完成
                          </span>
                        )}
                        {isVulnStage && queuedChildren > 0 && (
                          <span className="rounded bg-indigo-500/10 px-1.5 py-0.5 text-indigo-300">
                            {queuedChildren} 个排队中
                          </span>
                        )}
                        {blockedChildren > 0 && (
                          <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-300">
                            {blockedChildren} 个{isVulnStage ? "需处理" : "阻塞"}
                          </span>
                        )}
                        {unitCoverage && (
                          <span className="rounded bg-violet-500/10 px-1.5 py-0.5 font-medium text-violet-200">
                            {unitTerminalCells}/{unitCells.length} cells · 剩余{" "}
                            {unitCells.length - unitTerminalCells}
                          </span>
                        )}
                      </div>
                      {model.stageKind === "vuln_triage" && (
                        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                          {historicalFailedAttempts > 0 && (
                            <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-red-300">
                              历史失败 {historicalFailedAttempts}
                              {unitCoverage ? " cells" : ""}
                            </span>
                          )}
                          {currentRetries > 0 && (
                            <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-300">
                              自动重试 {currentRetries}
                            </span>
                          )}
                          {operatorRecoveries > 0 && (
                            <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-300">
                              待人工恢复 {operatorRecoveries}
                            </span>
                          )}
                        </div>
                      )}
                    </div>
                  );
                }
                return <UnsupportedLegacyTeamRun />;
              })()
            )}

            {schedulerDetailsVisible &&
              (!unit.plan ? (
                <div className="mt-2 rounded border border-dashed border-border/30 p-2 text-[10px] text-muted-foreground">
                  This Unit has no Company Controller plan; rerun is required.
                </div>
              ) : (
                <div className="mt-2 space-y-2">
                  <div className="rounded border border-cyan-500/20 bg-cyan-500/[0.04] p-2 text-[10px]">
                    <div className="flex flex-wrap items-center gap-2">
                      <GitBranch className="h-3 w-3 text-cyan-300" />
                      <span className="font-medium">Plan v{unit.plan.planVersion}</span>
                      <span className="text-muted-foreground" title={unit.plan.planSha256}>
                        {short(unit.plan.planSha256, 18)}
                      </span>
                      <span>{unit.plan.maxWorkersActive} active workers max</span>
                      <span>epoch {unit.plan.dispatchEpoch}</span>
                      <span>
                        {unit.plan.dynamicRequestsEnabled
                          ? "dynamic requests enabled"
                          : "static manifest"}
                      </span>
                    </div>
                    <div className="mt-1 flex flex-wrap gap-2 text-muted-foreground">
                      <span>Controller {unit.plan.leaderRole}</span>
                      <span>final submitter: same Controller</span>
                      <span>
                        {unit.plan.requestsClosedAt ? "request epoch closed" : "request epoch open"}
                      </span>
                    </div>
                    <div className="mt-1 flex flex-wrap gap-2">
                      <span
                        className={
                          unit.plan.barrier.readyToFinalize ? "text-emerald-300" : "text-amber-300"
                        }
                      >
                        Barrier {unit.plan.barrier.readyToFinalize ? "ready" : "waiting"}
                      </span>
                      <span>
                        {unit.plan.barrier.terminalRequiredWorkItems}/
                        {unit.plan.barrier.requiredWorkItems} SubAgents terminal
                      </span>
                      <span>{unit.plan.barrier.liveWorkers} live</span>
                      <span>{unit.plan.barrier.retryPendingWorkItems} retry</span>
                      <span>{unit.plan.barrier.recoveryRequiredWorkers} recovery</span>
                      <span>{unit.plan.barrier.missingOutputs} missing outputs</span>
                    </div>
                  </div>

                  {unit.plan.requests.length > 0 && (
                    <div className="rounded border border-border/25 p-2 text-[10px]">
                      <div className="mb-1 font-medium">Worker requests</div>
                      <div className="space-y-1 text-muted-foreground">
                        {unit.plan.requests.map((request) => (
                          <div key={request.requestId} className="flex flex-wrap gap-1.5">
                            <span>{request.status}</span>
                            <span>
                              {request.requestedRole}/{request.requestKind}
                            </span>
                            <span>{countLabel(request.subjectRefCount, "subject")}</span>
                            <span>reason {request.reasonCode}</span>
                            {request.decisionReasonCode && (
                              <span>decision {request.decisionReasonCode}</span>
                            )}
                          </div>
                        ))}
                      </div>
                    </div>
                  )}

                  <div className="space-y-1.5">
                    {unit.plan.workItems.map((item) => (
                      <div
                        key={item.workItemId}
                        className="rounded border border-border/25 p-2 text-[10px]"
                      >
                        <div className="flex flex-wrap items-center gap-1.5">
                          <StatusMark status={item.status} />
                          <span className="font-medium">{item.role}</span>
                          <span className="text-muted-foreground">
                            {item.kind} · {item.stableKey}
                          </span>
                          {isCompanyController(item) && (
                            <span className="rounded bg-cyan-500/15 px-1 text-cyan-300">
                              Controller
                            </span>
                          )}
                          <span className="ml-auto rounded bg-muted/40 px-1.5 py-0.5">
                            {item.status}
                          </span>
                        </div>
                        <div className="mt-1 flex flex-wrap gap-2 text-muted-foreground">
                          <span>{countLabel(item.subjectRefCount, "subject")}</span>
                          <span>
                            {countLabel(
                              item.dependencyWorkItemIds.length,
                              "dependency",
                              "dependencies"
                            )}
                          </span>
                          <span>
                            {item.requiredForBarrier ? "barrier-required" : "non-barrier"}
                          </span>
                          <span>schema {item.outputSchema}</span>
                          {item.maxAttempts != null && <span>max attempts {item.maxAttempts}</span>}
                        </div>

                        {item.workers.length > 0 && (
                          <div className="mt-1.5 space-y-1 border-l border-border/30 pl-2">
                            {item.workers.map((worker) => {
                              const attemptLane = workerAttemptLane(item, worker);
                              return (
                                <div key={worker.workerRunId} className="space-y-1">
                                  <div className="flex flex-wrap items-center gap-1.5 text-muted-foreground">
                                    <Bot className="h-3 w-3" />
                                    <span>Worker {short(worker.workerRunId)}</span>
                                    <span>{worker.status}</span>
                                    <span>epoch {worker.attemptEpoch}</span>
                                    <span>lease {worker.leaseState}</span>
                                    <span>recovery {worker.recoveryState}</span>
                                    {model.stageKind === "vuln_triage" && attemptLane && (
                                      <span
                                        className={cn(
                                          "rounded px-1.5 py-0.5 font-medium",
                                          attemptLane === "historical attempt failed"
                                            ? "bg-red-500/10 text-red-300"
                                            : attemptLane === "current retry"
                                              ? "bg-sky-500/10 text-sky-300"
                                              : "bg-amber-500/10 text-amber-300"
                                        )}
                                      >
                                        {attemptLane === "historical attempt failed"
                                          ? "历史 attempt 失败"
                                          : attemptLane === "current retry"
                                            ? "当前 retry"
                                            : "operator recovery"}
                                      </span>
                                    )}
                                    {worker.messageChainId && (
                                      <span>chain {short(worker.messageChainId)}</span>
                                    )}
                                    {worker.hasActiveTool && (
                                      <span className="text-sky-300">active tool</span>
                                    )}
                                    {isCompanyController(item) &&
                                      onOpenAgent &&
                                      agentRequestIdsByWorker[worker.workerRunId] && (
                                        <button
                                          type="button"
                                          className="rounded border border-border/30 px-1.5 py-0.5 text-cyan-300"
                                          onClick={() =>
                                            onOpenAgent(agentRequestIdsByWorker[worker.workerRunId])
                                          }
                                        >
                                          Controller 运行流
                                        </button>
                                      )}
                                  </div>
                                </div>
                              );
                            })}
                          </div>
                        )}

                        {item.output && (
                          <div className="mt-1.5 rounded bg-muted/25 px-2 py-1.5 text-muted-foreground">
                            <span className="font-medium text-foreground/80">
                              Output {item.output.businessDisposition}
                            </span>
                            {" · "}
                            {countLabel(item.output.evidenceIds.length, "evidence row")}
                            {" · "}
                            {countLabel(item.output.canonicalFactRefCount, "fact ref")}
                            {" · "}
                            {countLabel(item.output.checkedEmptyCellCount, "checked-empty cell")}
                            {item.output.blockerCodes.length > 0
                              ? ` · blocker ${item.output.blockerCodes.join(", ")}`
                              : ""}
                            <span title={item.output.outputSha256}>
                              {" "}
                              · {short(item.output.outputSha256, 18)}
                            </span>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              ))}
          </article>
        ))}
      </div>
    </section>
  );
}
