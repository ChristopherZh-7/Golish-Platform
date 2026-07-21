export type BackgroundJobOrigin =
  | { kind: "main_tool"; requestId: string }
  | { kind: "sub_agent_tool"; parentRequestId: string; requestId: string };

/** Lifecycle metadata retained on the originating tool after the live job leaves the registry. */
export interface BackgroundRunMeta {
  jobId: string;
  backgroundedAt: number;
  softTimeoutMs?: number;
  hardTimeoutMs?: number;
}

/** A session-attributed background job that is still active or stopping. */
export interface BackgroundJob extends BackgroundRunMeta {
  command: string;
  toolName: string;
  origin: BackgroundJobOrigin;
  /** Approximate command start, derived from the soft timeout when available. */
  startedAt: number;
  lastOutputAt?: number;
  state: "running" | "stopping";
}
