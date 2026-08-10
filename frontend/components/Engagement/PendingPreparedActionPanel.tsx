import {
  AlertTriangle,
  CheckCircle2,
  Loader2,
  RefreshCw,
  ShieldAlert,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError } from "@/lib/api";
import {
  type AttackPreparedActionDecision,
  type AttackPreparedActionReviewItem,
  decidePreparedAction,
  listPendingPreparedActions,
} from "@/lib/api/attack";
import { translateErrorCode } from "@/lib/api/error-codes";

export interface PendingPreparedActionPanelProps {
  operationId: string;
  campaignId?: string | null;
  refreshVersion?: number;
}

function displayError(error: unknown): string {
  if (error instanceof ApiError) return translateErrorCode(error.code, error.message);
  return error instanceof Error ? error.message : String(error);
}

function stableRequestId(): string {
  return (
    globalThis.crypto?.randomUUID?.() ??
    `prepared-action-${Date.now()}-${Math.random().toString(16).slice(2)}`
  );
}

function riskNeedsHuman(riskTier: string): boolean {
  return riskTier.toUpperCase() === "T2" || riskTier.toUpperCase() === "T3";
}

function stateLabel(value: string): string {
  return value.split("_").join(" ");
}

export function PendingPreparedActionPanel({
  operationId,
  campaignId = null,
  refreshVersion = 0,
}: PendingPreparedActionPanelProps) {
  const [items, setItems] = useState<AttackPreparedActionReviewItem[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const stableRequests = useRef(new Map<string, string>());

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(await listPendingPreparedActions({ operationId, campaignId }));
    } catch (cause) {
      setError(displayError(cause));
    } finally {
      setLoading(false);
    }
  }, [campaignId, operationId]);

  useEffect(() => {
    void refreshVersion;
    void load();
  }, [load, refreshVersion]);

  const decide = async (
    item: AttackPreparedActionReviewItem,
    decision: AttackPreparedActionDecision
  ) => {
    const requestKey = `${item.preparedActionId}:${decision}`;
    const requestId = stableRequests.current.get(requestKey) ?? stableRequestId();
    stableRequests.current.set(requestKey, requestId);
    setBusyId(item.preparedActionId);
    setError(null);
    try {
      await decidePreparedAction({
        operationId: item.operationId,
        campaignId: item.campaignId,
        preparedActionId: item.preparedActionId,
        decision,
        privateManifestHash: item.privateManifestHash,
        displayProjectionHash: item.displayProjectionHash,
        rendererVersion: item.rendererVersion,
        expectedRowVersion: item.rowVersion,
        stableRequestId: requestId,
        requestedExpiry: null,
      });
      stableRequests.current.delete(requestKey);
      await load();
    } catch (cause) {
      // Keep the stable request id for response-loss replay. Reload immediately
      // so CAS/hash/renderer/expiry drift disables the stale buttons.
      setError(displayError(cause));
      await load();
    } finally {
      setBusyId(null);
    }
  };

  if (loading && items === null) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-border/30 px-3 py-4 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        Loading prepared actions from the database…
      </div>
    );
  }

  if (items === null) {
    return (
      <div className="rounded-md border border-red-500/30 bg-red-500/5 p-3 text-xs">
        <div role="alert" className="flex items-center gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" />
          {error ?? "Prepared actions are unavailable."}
        </div>
        <button
          type="button"
          onClick={() => void load()}
          className="mt-2 text-accent hover:underline"
        >
          Retry DB read
        </button>
      </div>
    );
  }

  return (
    <section className="space-y-3 rounded-md border border-border/30 bg-muted/10 p-3">
      <div className="flex items-center gap-2">
        <ShieldAlert className="h-4 w-4 text-amber-300" />
        <h3 className="text-xs font-semibold text-foreground">Prepared action review</h3>
        <span className="ml-auto text-[10px] text-muted-foreground">
          {items.length} action{items.length === 1 ? "" : "s"}
        </span>
        <button
          type="button"
          aria-label="Refresh prepared actions"
          onClick={() => void load()}
          disabled={loading || busyId !== null}
          className="rounded p-1 text-muted-foreground hover:bg-muted/40 hover:text-foreground disabled:opacity-40"
        >
          <RefreshCw className={loading ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
        </button>
      </div>

      <p className="text-[11px] leading-relaxed text-muted-foreground">
        Review only this redacted, server-rendered packet. Approval is bound to the exact manifest,
        renderer, network policy, budget and row version shown here.
      </p>

      {error && (
        <div
          role="alert"
          className="flex items-start gap-2 rounded border border-red-500/30 bg-red-500/5 p-2 text-[11px] text-red-300"
        >
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          {error}
        </div>
      )}

      {items.length === 0 ? (
        <div className="rounded border border-border/30 px-3 py-4 text-center text-xs text-muted-foreground">
          No prepared actions require review.
        </div>
      ) : (
        <div className="space-y-2">
          {items.map((item) => {
            const display = item.displayProjection;
            const pending = item.reviewState === "pending";
            const humanDecision = riskNeedsHuman(item.riskTier);
            const disabled = busyId !== null || !pending || !humanDecision;
            return (
              <article
                key={item.preparedActionId}
                data-testid={`prepared-action-${item.preparedActionId}`}
                className="rounded border border-border/30 bg-card/40 p-2.5"
              >
                <div className="flex flex-wrap items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-xs text-foreground">
                        {display.targetAtTime}
                      </span>
                      <span className="rounded border border-amber-500/30 px-1.5 py-0.5 text-[10px] text-amber-300">
                        {item.riskTier}
                      </span>
                      <span className="rounded border border-border/35 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {stateLabel(item.reviewState)}
                      </span>
                    </div>
                    <div className="mt-1 text-[10px] text-muted-foreground">
                      {display.method} · {display.actionKind}
                    </div>
                  </div>
                  {item.reviewState === "authorized" && (
                    <CheckCircle2 className="h-4 w-4 text-emerald-300" />
                  )}
                  {item.reviewState === "denied" && <XCircle className="h-4 w-4 text-red-300" />}
                </div>

                <dl className="mt-2 grid gap-2 rounded bg-muted/25 p-2 text-[10px] sm:grid-cols-2">
                  <div>
                    <dt className="text-muted-foreground">Sequence</dt>
                    <dd className="text-foreground/85">{display.redactedSequence.join(" → ")}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">Expected control</dt>
                    <dd className="text-foreground/85">{display.expectedControl}</dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">Destination policy</dt>
                    <dd className="text-foreground/85">
                      {display.destinationScopeSummary} · {display.redirectPolicy} · max{" "}
                      {display.maxRedirectHops} redirects
                    </dd>
                  </div>
                  <div>
                    <dt className="text-muted-foreground">Budget</dt>
                    <dd className="text-foreground/85">
                      {display.plannedBudgetAxes
                        .map((axis) => `${axis.axis} ${axis.plannedLimit} ${axis.unit}`)
                        .join(" · ")}
                    </dd>
                  </div>
                  {display.cleanupSummary && (
                    <div className="sm:col-span-2">
                      <dt className="text-muted-foreground">Cleanup</dt>
                      <dd className="text-foreground/85">{display.cleanupSummary}</dd>
                    </div>
                  )}
                </dl>

                {item.expiresAt && (
                  <div className="mt-2 text-[10px] text-muted-foreground">
                    Review packet expires {new Date(item.expiresAt).toLocaleString()}
                  </div>
                )}

                {pending && humanDecision && (
                  <div className="mt-2 flex gap-2">
                    <button
                      type="button"
                      onClick={() => void decide(item, "approve")}
                      disabled={disabled}
                      className="rounded border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1.5 text-xs text-emerald-300 disabled:opacity-40"
                    >
                      {busyId === item.preparedActionId ? "Saving…" : "Approve exact action"}
                    </button>
                    <button
                      type="button"
                      onClick={() => void decide(item, "deny")}
                      disabled={disabled}
                      className="rounded border border-red-500/40 bg-red-500/10 px-2.5 py-1.5 text-xs text-red-300 disabled:opacity-40"
                    >
                      Deny
                    </button>
                  </div>
                )}

                {pending && !humanDecision && (
                  <div className="mt-2 text-[10px] text-muted-foreground">
                    T0/T1 policy decisions are server-owned; no human approval control is exposed.
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
