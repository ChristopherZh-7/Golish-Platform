import { AlertTriangle, Clock3, History } from "lucide-react";
import type { InvestigationTimelinePageResponse } from "@/lib/api/investigation";
import { InvestigationAuditDrawer } from "./InvestigationAuditDrawer";
import { WorkspaceAsyncState } from "./WorkspaceAsyncState";
import type { ProjectionResource } from "./useInvestigationProjection";

export function InvestigationTimelineTab({
  resource,
  onLoadMore,
  onRetry,
}: {
  resource: ProjectionResource<InvestigationTimelinePageResponse>;
  onLoadMore: () => void;
  onRetry: () => void;
}) {
  const events = resource.data?.events ?? [];
  if (!resource.data || events.length === 0) {
    return (
      <WorkspaceAsyncState
        resource={resource}
        label="timeline events"
        empty={resource.data !== undefined && events.length === 0}
        onRetry={onRetry}
      />
    );
  }

  return (
    <div className="h-full overflow-y-auto p-4">
      <ol className="mx-auto max-w-4xl space-y-2">
        {events.map((event) => (
          <li key={event.eventId} className="rounded border border-border/30 bg-muted/10 p-3">
            <div className="flex flex-wrap items-center gap-2">
              <History className="h-3.5 w-3.5 text-cyan-300" />
              <span className="text-xs font-medium">{event.eventKind}</span>
              <span className="rounded border border-border/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                {event.entityKind}
              </span>
              <span className="ml-auto text-[10px] tabular-nums text-muted-foreground">
                change {event.changeSeq}
              </span>
            </div>
            <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-muted-foreground">
              <span className="inline-flex items-center gap-1">
                <Clock3 className="h-3 w-3" />
                Source {event.sourceOccurredAt ?? "historical time unknown"}
              </span>
              <span>Projected {event.projectedAt}</span>
              <span>{event.sourceTimeStatus}</span>
            </div>
            {event.invalidationReason && (
              <div className="mt-2 flex items-center gap-1.5 rounded border border-amber-400/25 bg-amber-400/[0.05] px-2 py-1.5 text-[10px] text-amber-200">
                <AlertTriangle className="h-3 w-3" /> Invalidated · {event.invalidationReason}
              </div>
            )}
            <div className="mt-2">
              <InvestigationAuditDrawer
                title="Event audit"
                fields={[
                  { label: "Event id", value: event.eventId },
                  { label: "Entity id", value: event.entityId },
                  { label: "Entity version", value: event.entityVersion },
                  { label: "Observed as of", value: event.authorityTime.observedAsOf },
                  { label: "Effective valid until", value: event.authorityTime.effectiveValidUntil },
                  { label: "Authority epoch", value: event.authorityTime.authorityEpochHash },
                  { label: "Temporal status", value: event.authorityTime.temporalStatus },
                ]}
              />
            </div>
          </li>
        ))}
      </ol>
      {resource.nextCursor && (
        <div className="mt-3 text-center">
          <button
            type="button"
            className="rounded border border-border/30 px-3 py-1.5 text-[11px]"
            onClick={onLoadMore}
          >
            Load older history
          </button>
        </div>
      )}
    </div>
  );
}
