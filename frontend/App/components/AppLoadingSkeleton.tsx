import { Skeleton } from "../../components/ui/skeleton";

export function AppLoadingSkeleton() {
  return (
    <div className="h-screen w-screen bg-background flex overflow-hidden">
      <div className="w-[48px] flex-shrink-0 bg-background border-r border-[var(--border-subtle)] flex flex-col items-center gap-2 pt-12">
        <Skeleton className="h-8 w-8 rounded-md bg-muted" />
        <Skeleton className="h-8 w-8 rounded-md bg-muted" />
        <Skeleton className="h-8 w-8 rounded-md bg-muted" />
      </div>
      <div className="w-[220px] flex-shrink-0 bg-card border-r border-[var(--border-subtle)] p-3 space-y-3">
        <Skeleton className="h-6 w-16 bg-muted" />
        <Skeleton className="h-7 w-full bg-muted" />
        <Skeleton className="h-4 w-20 bg-muted" />
        <Skeleton className="h-4 w-24 bg-muted" />
      </div>
      <div className="flex-1 flex flex-col">
        <div className="flex items-center h-[34px] bg-card border-b border-[var(--border-subtle)] pl-2 pr-2 gap-2">
          <Skeleton className="h-5 w-20 bg-muted" />
          <Skeleton className="h-5 w-5 rounded bg-muted" />
        </div>
        <div className="flex-1 p-4 space-y-3">
          <Skeleton className="h-16 w-full bg-muted" />
          <Skeleton className="h-16 w-3/4 bg-muted" />
        </div>
      </div>
      <div className="w-[340px] flex-shrink-0 bg-card border-l border-[var(--border-subtle)] p-3 space-y-3">
        <Skeleton className="h-6 w-16 bg-muted" />
        <Skeleton className="h-20 w-full bg-muted" />
      </div>
    </div>
  );
}

export function AppErrorFallback({ error }: { error: string }) {
  return (
    <div className="flex items-center justify-center h-screen bg-background">
      <div className="text-[#f7768e] text-lg">Error: {error}</div>
    </div>
  );
}
