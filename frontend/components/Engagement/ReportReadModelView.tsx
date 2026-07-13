import { AlertTriangle, CheckCircle2, FileCheck2, Loader2, RotateCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  buildReportReadModel,
  finalizeReportRevision,
  getReportReadModel,
  type ReportingFinalizeRequest,
  type ReportReadModelView as ReportReadModelData,
} from "@/lib/api/reporting";

export interface ReportingViewApi {
  getReadModel: (request: { operationId: string }) => Promise<ReportReadModelData | null>;
  buildReadModel: (request: { operationId: string }) => Promise<ReportReadModelData>;
  finalizeRevision: (request: ReportingFinalizeRequest) => Promise<ReportReadModelData>;
}

const defaultApi: ReportingViewApi = {
  getReadModel: getReportReadModel,
  buildReadModel: buildReportReadModel,
  finalizeRevision: finalizeReportRevision,
};

export interface ReportReadModelViewProps {
  operationId: string;
  api?: ReportingViewApi;
  refreshVersion?: number;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

interface FrozenFinalizeRequest {
  operationId: string;
  revisionId: string;
  sourceSetHash: string;
  rowVersion: number;
}

type BusyAction = "refreshing" | "building" | "finalizing";

function revisionMatchesFrozenRequest(
  model: ReportReadModelData | null | undefined,
  frozen: FrozenFinalizeRequest
): boolean {
  const current = model?.current;
  return (
    model?.operationId === frozen.operationId &&
    current?.revisionId === frozen.revisionId &&
    current.sourceSetHash === frozen.sourceSetHash &&
    current.rowVersion === frozen.rowVersion
  );
}

export function ReportReadModelView({
  operationId,
  api = defaultApi,
  refreshVersion = 0,
}: ReportReadModelViewProps) {
  const [modelState, setModelState] = useState<ReportReadModelData | null | undefined>(undefined);
  const [modelOperationId, setModelOperationId] = useState<string | null>(null);
  const [errorState, setErrorState] = useState<{
    operationId: string;
    text: string;
  } | null>(null);
  const [pendingFinalize, setPendingFinalize] = useState<FrozenFinalizeRequest | null>(null);
  const [busy, setBusy] = useState<BusyAction | null>(null);
  const requestSequence = useRef(0);

  const model = modelOperationId === operationId ? modelState : undefined;
  const error = errorState?.operationId === operationId ? errorState.text : null;
  const activePending = pendingFinalize?.operationId === operationId ? pendingFinalize : null;
  const building = busy === "building";
  const refreshing = busy === "refreshing";
  const finalizing = busy === "finalizing";

  const load = useCallback(async () => {
    const sequence = ++requestSequence.current;
    setPendingFinalize(null);
    setBusy("refreshing");
    setErrorState(null);
    try {
      const next = await api.getReadModel({ operationId });
      if (requestSequence.current !== sequence) return;
      setModelState(next);
      setModelOperationId(operationId);
    } catch (cause) {
      if (requestSequence.current !== sequence) return;
      setModelState(undefined);
      setModelOperationId(operationId);
      setErrorState({ operationId, text: message(cause) });
    } finally {
      if (requestSequence.current === sequence) setBusy(null);
    }
  }, [api, operationId]);

  useEffect(() => {
    void refreshVersion;
    void load();
    return () => {
      requestSequence.current += 1;
    };
  }, [load, refreshVersion]);

  const build = async () => {
    const sequence = ++requestSequence.current;
    setPendingFinalize(null);
    setBusy("building");
    setErrorState(null);
    try {
      const next = await api.buildReadModel({ operationId });
      if (requestSequence.current !== sequence) return;
      setModelState(next);
      setModelOperationId(operationId);
    } catch (cause) {
      if (requestSequence.current !== sequence) return;
      setErrorState({ operationId, text: message(cause) });
    } finally {
      if (requestSequence.current === sequence) setBusy(null);
    }
  };

  const finalize = async () => {
    const frozen = activePending;
    if (!frozen || busy !== null) return;
    if (!revisionMatchesFrozenRequest(model, frozen)) {
      setPendingFinalize(null);
      setErrorState({
        operationId,
        text: "The report revision changed before confirmation. Review the current revision and start final publication again.",
      });
      return;
    }

    const sequence = ++requestSequence.current;
    setBusy("finalizing");
    setErrorState(null);
    try {
      const next = await api.finalizeRevision({
        operationId: frozen.operationId,
        revisionId: frozen.revisionId,
        expectedSourceHash: frozen.sourceSetHash,
        expectedRevisionVersion: frozen.rowVersion,
        confirmFinalPublish: true,
      });
      if (requestSequence.current !== sequence) return;
      setModelState(next);
      setModelOperationId(frozen.operationId);
      setPendingFinalize(null);
    } catch (cause) {
      if (requestSequence.current !== sequence) return;
      setPendingFinalize(null);
      setErrorState({ operationId: frozen.operationId, text: message(cause) });
    } finally {
      if (requestSequence.current === sequence) setBusy(null);
    }
  };

  if (model === undefined && error === null) {
    return (
      <div className="flex items-center gap-2 rounded border border-border/30 p-3 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading cited report…
      </div>
    );
  }

  if (model === undefined) {
    return (
      <div className="rounded border border-red-500/30 bg-red-500/5 p-3 text-xs">
        <div role="alert" className="flex items-center gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" /> {error ?? "Report state is unavailable."}
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

  if (model === null) {
    return (
      <div className="rounded border border-border/30 p-4 text-center text-xs text-muted-foreground">
        <p>No report revision has been built for this operation.</p>
        {error && (
          <div role="alert" className="mt-2 text-red-300">
            {error}
          </div>
        )}
        <button
          type="button"
          disabled={busy !== null}
          className="mt-3 rounded border border-accent/40 px-2 py-1 text-accent disabled:opacity-50"
          onClick={() => void build()}
        >
          {building ? "Building cited report…" : "Build cited report"}
        </button>
      </div>
    );
  }

  const current = model.current;
  const isFinalizable =
    current?.validationStatus === "validated" && current.publicationStatus === "unpublished";
  const canBeginFinalize = isFinalizable && busy === null;

  const beginFinalizeConfirmation = () => {
    if (!canBeginFinalize || !current) return;
    setPendingFinalize({
      operationId,
      revisionId: current.revisionId,
      sourceSetHash: current.sourceSetHash,
      rowVersion: current.rowVersion,
    });
  };

  return (
    <section className="space-y-3 rounded border border-border/30 bg-muted/10 p-3">
      <header className="flex flex-wrap items-center gap-2">
        {current?.validationStatus === "validated" ? (
          <CheckCircle2 className="h-4 w-4 text-emerald-300" />
        ) : (
          <AlertTriangle className="h-4 w-4 text-amber-300" />
        )}
        <h3 className="text-xs font-semibold">Cited report</h3>
        <span className="rounded border border-border/30 px-1.5 py-0.5 text-[10px]">
          {current?.publicationStatus === "final" ? "Final artifact" : "Validated draft"}
        </span>
        <button
          type="button"
          disabled={busy !== null}
          className="ml-auto rounded border border-border/30 px-2 py-1 text-[10px] disabled:opacity-50"
          onClick={() => void build()}
        >
          {building ? "Rebuilding…" : "Rebuild from DB truth"}
        </button>
        <button
          type="button"
          aria-label="Refresh cited report"
          disabled={busy !== null}
          onClick={() => void load()}
        >
          <RotateCw className={`h-3.5 w-3.5 ${refreshing ? "animate-spin" : ""}`} />
        </button>
      </header>

      {error && (
        <div role="alert" className="rounded border border-red-500/30 p-2 text-[11px] text-red-300">
          {error}
        </div>
      )}

      <div className="flex flex-wrap gap-1.5 text-[10px] text-muted-foreground">
        {model.revisions.map((revision) => (
          <span key={revision.revisionId} className="rounded border border-border/30 px-1.5 py-0.5">
            rev {revision.revisionNumber} · {revision.validationStatus} ·{" "}
            {revision.publicationStatus}
            {revision.publicationStatus === "superseded" ? " · superseded" : ""}
          </span>
        ))}
      </div>

      <div className="space-y-2">
        {model.sections.map((section) => (
          <article
            key={section.sectionId}
            className="rounded border border-border/30 p-2.5 text-[11px]"
          >
            <div className="flex flex-wrap items-center gap-2 font-medium">
              <span>{section.organizationNameAtSnapshot ?? section.sectionKind}</span>
              <span className="text-muted-foreground">{section.sectionKind}</span>
            </div>
            {section.renderedContent && (
              <p className="mt-2 whitespace-pre-wrap">{section.renderedContent}</p>
            )}
            <div className="mt-2 space-y-2">
              {section.claims.map((claim) => (
                <div key={claim.claimId} className="rounded bg-muted/20 p-2">
                  <div>
                    <span className="rounded border border-border/30 px-1 py-0.5 text-[9px]">
                      {claim.claimKind}
                    </span>{" "}
                    {claim.subjectRef} {claim.predicate}
                  </div>
                  <pre className="mt-1 overflow-auto text-[10px]">
                    {JSON.stringify(claim.value, null, 2)}
                  </pre>
                  <ul className="mt-1 space-y-0.5 text-[10px] text-muted-foreground">
                    {claim.citations.map((citation) => (
                      <li key={citation.citationId}>
                        Evidence {citation.evidenceAuditId} · {citation.sourceKind}:
                        {citation.sourceIdValue}
                        @v{citation.sourceRowVersion}
                      </li>
                    ))}
                  </ul>
                </div>
              ))}
            </div>
          </article>
        ))}
      </div>

      {model.artifacts.length > 0 && (
        <div className="space-y-1 text-[10px]">
          {model.artifacts.map((artifact) => (
            <div
              key={`${artifact.revisionId}:${artifact.artifactKind}`}
              className="flex items-center gap-2"
            >
              <FileCheck2 className="h-3.5 w-3.5 text-emerald-300" />
              <span>{artifact.artifactKind}</span>
              <span className="font-mono text-muted-foreground">{artifact.contentKey}</span>
            </div>
          ))}
        </div>
      )}

      {canBeginFinalize && !activePending && (
        <button
          type="button"
          className="rounded border border-emerald-500/30 px-2 py-1 text-[11px] text-emerald-200"
          onClick={beginFinalizeConfirmation}
        >
          Finalize report
        </button>
      )}
      {isFinalizable && activePending && (
        <div
          role="status"
          className="flex flex-wrap items-center gap-2 rounded border border-amber-500/30 p-2 text-[11px]"
        >
          <span>
            Final publication pending for revision {activePending.revisionId}, row version{" "}
            {activePending.rowVersion}, source {activePending.sourceSetHash}. Refresh, rebuild, or
            an operation change cancels this frozen request.
          </span>
          <button
            type="button"
            disabled={finalizing}
            className="ml-auto rounded border border-emerald-500/30 px-2 py-1 text-emerald-200 disabled:opacity-50"
            onClick={() => void finalize()}
          >
            {finalizing ? "Finalizing…" : "Confirm final publish"}
          </button>
          <button type="button" disabled={finalizing} onClick={() => setPendingFinalize(null)}>
            Cancel
          </button>
        </div>
      )}
    </section>
  );
}
