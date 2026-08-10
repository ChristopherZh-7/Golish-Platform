import { AlertTriangle, Database, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { InvestigationHypothesisListView } from "@/lib/api/investigation";
import type { InvestigationProjectionEnvelope } from "@/lib/generated/InvestigationProjectionEnvelope";
import { cn } from "@/lib/utils";

export interface HypothesisRegistryAuditApi {
  getSummary: (request: LegacyRegistryScope) => Promise<LegacyRegistrySummaryView>;
  listHypotheses: (request: LegacyRegistryListRequest) => Promise<InvestigationHypothesisListView>;
  getHypothesis: (request: LegacyRegistryDetailRequest) => Promise<LegacyHypothesisDetailView>;
}

interface LegacyRegistryScope {
  sessionId: string;
  operationId: string;
}

interface LegacyRegistryListRequest extends LegacyRegistryScope {
  organizationIds: string[];
  epistemicStates: string[];
  readinessStates: string[];
  capabilityStates: string[];
  sourceKinds: string[];
  cursor: string | null;
  expectedChangeSeq: number | null;
  pageSize: number;
}

interface LegacyRegistryDetailRequest extends LegacyRegistryScope {
  revisionId: string;
}

interface LegacyRegistrySummaryView {
  envelope: InvestigationProjectionEnvelope;
  activeGenerationId: string | null;
  activeGenerationSealHash: string | null;
  currentHypothesisCount: number;
  closedHypothesisCount: number;
  contestedHypothesisCount: number;
  residualCount: number;
}

type LegacyHypothesisListItem = InvestigationHypothesisListView["hypotheses"][number];

interface LegacyHypothesisDetailView {
  envelope: InvestigationProjectionEnvelope;
  hypothesis: LegacyHypothesisListItem;
  predecessorRevisionId: string | null;
  lineageRevisionIds: string[];
  supportRefIds: string[];
  contradictionRefIds: string[];
  applicationContextRefIds: string[];
  gapRefIds: string[];
  verificationObjectiveSummaries: string[];
  legacyUnavailableFields: string[];
}

function retiredProductionRead(): Promise<never> {
  return Promise.reject(
    new Error("The operation-only Registry audit adapter is retired from production routing.")
  );
}

const defaultApi: HypothesisRegistryAuditApi = {
  getSummary: retiredProductionRead,
  listHypotheses: retiredProductionRead,
  getHypothesis: retiredProductionRead,
};

export interface HypothesisRegistryAuditProps {
  sessionId: string;
  operationId: string;
  api?: HypothesisRegistryAuditApi;
}

type ScopedData<T> = { scopeKey: string; data: T } | null;
type ScopedError = { scopeKey: string; message: string } | null;

const MODE_BADGE_STYLES: Record<string, string> = {
  legacy_only: "border-slate-400/35 bg-slate-400/10 text-slate-300",
  shadow_registry: "border-violet-400/35 bg-violet-400/10 text-violet-300",
  dual_read_compare: "border-sky-400/35 bg-sky-400/10 text-sky-300",
  registry_authoritative_legacy_projection:
    "border-emerald-400/35 bg-emerald-400/10 text-emerald-300",
  new_only: "border-cyan-400/35 bg-cyan-400/10 text-cyan-300",
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function modeBadgeClass(mode: string): string {
  return MODE_BADGE_STYLES[mode] ?? "border-amber-400/35 bg-amber-400/10 text-amber-300";
}

function StateChip({ children }: { children: string }) {
  return (
    <span className="rounded border border-border/35 bg-background/35 px-1.5 py-0.5 font-mono text-[10px] text-foreground/75">
      {children}
    </span>
  );
}

function ErrorPanel({
  message,
  retryLabel,
  onRetry,
}: {
  message: string;
  retryLabel: string;
  onRetry: () => void;
}) {
  return (
    <div className="rounded border border-red-500/30 bg-red-500/[0.06] p-3 text-xs">
      <div role="alert" className="flex items-start gap-2 text-red-300">
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <span className="break-words">{message}</span>
      </div>
      <button type="button" className="mt-2 text-accent hover:underline" onClick={onRetry}>
        {retryLabel}
      </button>
    </div>
  );
}

function SummaryPanel({ summary }: { summary: LegacyRegistrySummaryView }) {
  const { envelope } = summary;
  const mode = envelope.investigationRolloutMode;
  const empty =
    summary.activeGenerationId === null &&
    summary.currentHypothesisCount === 0 &&
    summary.closedHypothesisCount === 0 &&
    summary.contestedHypothesisCount === 0 &&
    summary.residualCount === 0;
  return (
    <section className="space-y-3 rounded border border-border/30 bg-background/25 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Frozen mode
        </span>
        <span
          className={cn("rounded border px-2 py-0.5 font-mono text-[10px]", modeBadgeClass(mode))}
        >
          {mode}
        </span>
        <span className="ml-auto text-[10px] tabular-nums text-muted-foreground">
          projection #{envelope.changeSeq}
        </span>
      </div>

      <dl className="grid gap-2 text-[11px] sm:grid-cols-2">
        <div className="rounded bg-muted/20 px-2.5 py-2">
          <dt className="text-muted-foreground">Generation seal</dt>
          <dd className="mt-0.5 break-all font-mono text-foreground/85">
            {summary.activeGenerationSealHash ?? "unavailable"}
          </dd>
        </div>
        <div className="rounded bg-muted/20 px-2.5 py-2">
          <dt className="text-muted-foreground">Generation</dt>
          <dd className="mt-0.5 break-all font-mono text-foreground/85">
            {summary.activeGenerationId ?? "unavailable"}
          </dd>
        </div>
      </dl>

      <div className="grid grid-cols-2 gap-2 text-center text-[10px] sm:grid-cols-4">
        {[
          ["Current", summary.currentHypothesisCount],
          ["Closed", summary.closedHypothesisCount],
          ["Contested", summary.contestedHypothesisCount],
          ["Residuals", summary.residualCount],
        ].map(([label, count]) => (
          <div key={label} className="rounded border border-border/25 bg-muted/10 px-2 py-1.5">
            <div className="text-sm font-semibold tabular-nums text-foreground/90">{count}</div>
            <div className="text-muted-foreground">{label}</div>
          </div>
        ))}
      </div>

      {empty && (
        <div className="rounded border border-border/25 bg-muted/10 px-3 py-2 text-center text-xs text-muted-foreground">
          No active hypothesis generation.
        </div>
      )}
    </section>
  );
}

type HypothesisListItem = LegacyHypothesisListItem;

function HypothesisRow({ item, onOpen }: { item: HypothesisListItem; onOpen: () => void }) {
  const legacyStatus = item.legacyProjectionStatus ?? "legacy_unavailable";
  return (
    <button
      type="button"
      aria-label={`Open ${item.predicateSummary}`}
      className="w-full rounded border border-border/30 bg-background/25 p-3 text-left transition-colors hover:border-cyan-400/35 hover:bg-cyan-500/[0.03]"
      onClick={onOpen}
    >
      <div className="flex min-w-0 items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="break-words text-xs font-medium text-foreground/90">
            {item.predicateSummary}
          </div>
          <div className="mt-1 break-all text-[10px] text-muted-foreground">
            At-time subject · {item.targetTypeAtTime} · {item.targetValueAtTime}
          </div>
        </div>
        <StateChip>{legacyStatus}</StateChip>
      </div>

      <div role="group" className="mt-2 flex flex-wrap gap-1.5" aria-label="Hypothesis states">
        <StateChip>{item.epistemicState}</StateChip>
        <StateChip>{item.lifecycleState}</StateChip>
        <StateChip>{item.planningReadiness}</StateChip>
      </div>

      <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[10px] tabular-nums text-muted-foreground">
        <span>Support {item.supportCount}</span>
        <span>Conflict {item.contradictionCount}</span>
        <span>Gap {item.gapCount}</span>
      </div>

      {item.residualCodes.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {item.residualCodes.map((code: string) => (
            <span
              key={code}
              className="rounded border border-amber-400/30 bg-amber-400/[0.07] px-1.5 py-0.5 font-mono text-[10px] text-amber-200"
            >
              {code}
            </span>
          ))}
        </div>
      )}
    </button>
  );
}

function DetailPanel({ detail }: { detail: LegacyHypothesisDetailView }) {
  const { hypothesis } = detail;
  const references = [
    ["Support refs", detail.supportRefIds],
    ["Conflict refs", detail.contradictionRefIds],
    ["Context refs", detail.applicationContextRefIds],
    ["Gap refs", detail.gapRefIds],
    ["Lineage", detail.lineageRevisionIds],
  ] as const;

  return (
    <section className="space-y-3 rounded border border-cyan-500/25 bg-cyan-500/[0.025] p-3">
      <div>
        <h4 className="text-xs font-semibold text-foreground/90">Hypothesis detail</h4>
        <p className="mt-1 break-words text-[11px] text-muted-foreground">
          {hypothesis.predicateSummary}
        </p>
        <p className="mt-1 break-all text-[10px] text-muted-foreground/75">
          At-time subject · {hypothesis.targetTypeAtTime} · {hypothesis.targetValueAtTime}
        </p>
      </div>

      {detail.verificationObjectiveSummaries.length > 0 && (
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Verification objectives
          </div>
          <ul className="mt-1 space-y-1 text-[11px] text-foreground/80">
            {detail.verificationObjectiveSummaries.map((objective: string) => (
              <li key={objective} className="rounded bg-muted/20 px-2 py-1.5">
                {objective}
              </li>
            ))}
          </ul>
        </div>
      )}

      {detail.legacyUnavailableFields.length > 0 && (
        <div>
          <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Legacy unavailable
          </div>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {detail.legacyUnavailableFields.map((field: string) => (
              <StateChip key={field}>{field}</StateChip>
            ))}
          </div>
        </div>
      )}

      <dl className="grid gap-1 text-[10px] text-muted-foreground sm:grid-cols-2">
        {references.map(([label, values]) => (
          <div key={label} className="rounded bg-muted/15 px-2 py-1.5">
            <dt>{label}</dt>
            <dd className="mt-0.5 break-all font-mono text-foreground/75">
              {values.length > 0 ? values.join(", ") : "none"}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

export function HypothesisRegistryAudit({
  sessionId,
  operationId,
  api = defaultApi,
}: HypothesisRegistryAuditProps) {
  const scopeKey = `${sessionId}:${operationId}`;
  const [summaryState, setSummaryState] = useState<ScopedData<LegacyRegistrySummaryView>>(null);
  const [listState, setListState] = useState<ScopedData<InvestigationHypothesisListView>>(null);
  const [detailState, setDetailState] = useState<{
    scopeKey: string;
    revisionId: string;
    data: LegacyHypothesisDetailView;
  } | null>(null);
  const [summaryError, setSummaryError] = useState<ScopedError>(null);
  const [listError, setListError] = useState<ScopedError>(null);
  const [detailError, setDetailError] = useState<{
    scopeKey: string;
    revisionId: string;
    message: string;
  } | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  const [listLoading, setListLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [selectedRevisionId, setSelectedRevisionId] = useState<string | null>(null);
  const summarySequence = useRef(0);
  const listSequence = useRef(0);
  const detailSequence = useRef(0);

  const summary = summaryState?.scopeKey === scopeKey ? summaryState.data : null;
  const list = listState?.scopeKey === scopeKey ? listState.data : null;
  const detail =
    detailState?.scopeKey === scopeKey && detailState.revisionId === selectedRevisionId
      ? detailState.data
      : null;
  const activeSummaryError = summaryError?.scopeKey === scopeKey ? summaryError.message : null;
  const activeListError = listError?.scopeKey === scopeKey ? listError.message : null;
  const activeDetailError =
    detailError?.scopeKey === scopeKey && detailError.revisionId === selectedRevisionId
      ? detailError.message
      : null;

  const loadSummary = useCallback(async () => {
    const sequence = ++summarySequence.current;
    setSummaryLoading(true);
    setSummaryError(null);
    try {
      const data = await api.getSummary({ sessionId, operationId });
      if (summarySequence.current !== sequence) return;
      setSummaryState({ scopeKey, data });
    } catch (error) {
      if (summarySequence.current !== sequence) return;
      setSummaryError({ scopeKey, message: errorMessage(error) });
    } finally {
      if (summarySequence.current === sequence) setSummaryLoading(false);
    }
  }, [api, operationId, scopeKey, sessionId]);

  const loadList = useCallback(async () => {
    const sequence = ++listSequence.current;
    setListLoading(true);
    setListError(null);
    try {
      const data = await api.listHypotheses({
        sessionId,
        operationId,
        organizationIds: [],
        epistemicStates: [],
        readinessStates: [],
        capabilityStates: [],
        sourceKinds: [],
        cursor: null,
        expectedChangeSeq: null,
        pageSize: 100,
      });
      if (listSequence.current !== sequence) return;
      setListState({ scopeKey, data });
    } catch (error) {
      if (listSequence.current !== sequence) return;
      setListError({ scopeKey, message: errorMessage(error) });
    } finally {
      if (listSequence.current === sequence) setListLoading(false);
    }
  }, [api, operationId, scopeKey, sessionId]);

  const loadDetail = useCallback(
    async (revisionId: string) => {
      const sequence = ++detailSequence.current;
      setDetailLoading(true);
      setDetailError(null);
      try {
        const data = await api.getHypothesis({ sessionId, operationId, revisionId });
        if (detailSequence.current !== sequence) return;
        setDetailState({ scopeKey, revisionId, data });
      } catch (error) {
        if (detailSequence.current !== sequence) return;
        setDetailError({ scopeKey, revisionId, message: errorMessage(error) });
      } finally {
        if (detailSequence.current === sequence) setDetailLoading(false);
      }
    },
    [api, operationId, scopeKey, sessionId]
  );

  useEffect(() => {
    setSelectedRevisionId(null);
    setDetailState(null);
    setDetailError(null);
    setDetailLoading(false);
    detailSequence.current += 1;
  }, [scopeKey]);

  useEffect(() => {
    void loadSummary();
    void loadList();
    return () => {
      summarySequence.current += 1;
      listSequence.current += 1;
    };
  }, [loadList, loadSummary]);

  const openDetail = (revisionId: string) => {
    setSelectedRevisionId(revisionId);
    void loadDetail(revisionId);
  };

  const refresh = () => {
    void loadSummary();
    void loadList();
    if (selectedRevisionId) void loadDetail(selectedRevisionId);
  };

  const stale = Boolean((summary && summaryLoading) || (list && listLoading));

  return (
    <section
      className="space-y-3 rounded-lg border border-cyan-500/20 bg-cyan-500/[0.02] p-3"
      data-testid="hypothesis-registry-audit"
    >
      <header className="flex items-center gap-2">
        <Database className="h-4 w-4 text-cyan-300" />
        <div className="min-w-0">
          <h3 className="text-xs font-semibold text-foreground/90">Hypothesis Registry Audit</h3>
          <p className="truncate font-mono text-[10px] text-muted-foreground" title={operationId}>
            {operationId}
          </p>
        </div>
        {stale && (
          <span className="ml-auto rounded border border-amber-400/30 bg-amber-400/10 px-1.5 py-0.5 text-[10px] text-amber-300">
            stale
          </span>
        )}
        <button
          type="button"
          aria-label="Refresh audit"
          disabled={summaryLoading || listLoading}
          className={cn(
            "rounded border border-border/30 p-1.5 text-muted-foreground hover:text-foreground disabled:opacity-50",
            !stale && "ml-auto"
          )}
          onClick={refresh}
        >
          <RefreshCw className={cn("h-3.5 w-3.5", stale && "animate-spin")} />
        </button>
      </header>

      {summary ? (
        <>
          <SummaryPanel summary={summary} />
          {activeSummaryError && (
            <ErrorPanel
              message={activeSummaryError}
              retryLabel="Retry summary"
              onRetry={() => void loadSummary()}
            />
          )}
        </>
      ) : summaryLoading ? (
        <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading registry summary…
        </div>
      ) : activeSummaryError ? (
        <ErrorPanel
          message={activeSummaryError}
          retryLabel="Retry summary"
          onRetry={() => void loadSummary()}
        />
      ) : (
        <div className="rounded border border-border/30 p-3 text-xs text-muted-foreground">
          Registry summary is empty.
        </div>
      )}

      <section className="space-y-2">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Hypotheses
        </div>
        {list ? (
          <>
            {activeListError && (
              <ErrorPanel
                message={activeListError}
                retryLabel="Retry hypotheses"
                onRetry={() => void loadList()}
              />
            )}
            {list.hypotheses.length === 0 ? (
              <div className="rounded border border-border/30 p-4 text-center text-xs text-muted-foreground">
                No hypotheses in this projection.
              </div>
            ) : (
              <div className="space-y-2">
                {list.hypotheses.map((item: HypothesisListItem) => (
                  <HypothesisRow
                    key={item.revisionId}
                    item={item}
                    onOpen={() => openDetail(item.revisionId)}
                  />
                ))}
              </div>
            )}
          </>
        ) : listLoading ? (
          <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
            <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading hypotheses…
          </div>
        ) : activeListError ? (
          <ErrorPanel
            message={activeListError}
            retryLabel="Retry hypotheses"
            onRetry={() => void loadList()}
          />
        ) : (
          <div className="rounded border border-border/30 p-4 text-center text-xs text-muted-foreground">
            No hypotheses in this projection.
          </div>
        )}
      </section>

      {selectedRevisionId && (
        <section aria-label="Selected hypothesis detail">
          {detail ? (
            <>
              <DetailPanel detail={detail} />
              {activeDetailError && (
                <div className="mt-2">
                  <ErrorPanel
                    message={activeDetailError}
                    retryLabel="Retry hypothesis detail"
                    onRetry={() => void loadDetail(selectedRevisionId)}
                  />
                </div>
              )}
            </>
          ) : detailLoading ? (
            <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading hypothesis detail…
            </div>
          ) : activeDetailError ? (
            <ErrorPanel
              message={activeDetailError}
              retryLabel="Retry hypothesis detail"
              onRetry={() => void loadDetail(selectedRevisionId)}
            />
          ) : null}
        </section>
      )}
    </section>
  );
}
