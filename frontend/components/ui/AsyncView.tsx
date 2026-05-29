import { Loader2 } from "lucide-react";
import type { ReactNode } from "react";

export interface AsyncViewProps {
  /** Whether the async source is currently loading. */
  loading: boolean;
  /** Error message to show, or null/undefined when there is no error. */
  error?: string | null;
  /** Whether resolved data is empty (only consulted when not loading and no error). */
  isEmpty?: boolean;
  /** Custom node rendered while loading (defaults to a centered spinner). */
  loadingFallback?: ReactNode;
  /** Custom node rendered on error (defaults to the error message). */
  errorFallback?: ReactNode;
  /** Custom node rendered when empty (defaults to {@link emptyMessage}). */
  emptyFallback?: ReactNode;
  /** Message used by the default empty fallback. */
  emptyMessage?: string;
  /** Resolved content, rendered when not loading, no error, and not empty. */
  children: ReactNode;
}

/**
 * Tri-state container for async UI: renders loading / error / empty fallbacks,
 * otherwise the children. Defaults mirror the spinner + muted-empty styling used
 * across the app; pass `*Fallback` props to override any single state.
 */
export function AsyncView({
  loading,
  error,
  isEmpty = false,
  loadingFallback,
  errorFallback,
  emptyFallback,
  emptyMessage = "No data",
  children,
}: AsyncViewProps): ReactNode {
  if (loading) {
    return (
      loadingFallback ?? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" />
        </div>
      )
    );
  }

  if (error) {
    return (
      errorFallback ?? <div className="text-center text-[11px] text-red-400/70 py-12">{error}</div>
    );
  }

  if (isEmpty) {
    return (
      emptyFallback ?? (
        <div className="text-center text-[11px] text-muted-foreground/30 py-12">{emptyMessage}</div>
      )
    );
  }

  return <>{children}</>;
}
