import { AlertTriangle, Loader2 } from "lucide-react";
import type { ProjectionResource } from "./useInvestigationProjection";

export function WorkspaceAsyncState({
  resource,
  label,
  empty,
  onRetry,
}: {
  resource: ProjectionResource<unknown>;
  label: string;
  empty: boolean;
  onRetry: () => void;
}) {
  if (resource.status === "loading" && !resource.data) {
    return (
      <div className="flex h-full items-center justify-center gap-2 p-4 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading {label}…
      </div>
    );
  }
  if (resource.status === "error" && !resource.data) {
    return (
      <div className="space-y-2 p-4 text-xs">
        <div role="alert" className="flex gap-2 text-red-300">
          <AlertTriangle className="h-3.5 w-3.5" />
          {resource.errorMessage ?? `${label} are unavailable.`}
        </div>
        <button type="button" className="text-accent hover:underline" onClick={onRetry}>
          Retry {label}
        </button>
      </div>
    );
  }
  if (empty) {
    return (
      <div className="flex h-full items-center justify-center p-4 text-center text-xs text-muted-foreground">
        No {label} are present in this projection.
      </div>
    );
  }
  return null;
}
