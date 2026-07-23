export type BackgroundJobOrigin =
  | { kind: "main_tool"; requestId: string }
  | { kind: "sub_agent_tool"; parentRequestId: string; requestId: string };

/** Lifecycle metadata retained on the originating tool after the live job leaves the registry. */
export interface BackgroundRunMeta {
  jobId: string;
  backgroundedAt: number;
  /** Transport diagnostic only; never a business deadline or process lifetime. */
  initialYieldMs?: number;
  /** Managed jobs never have an elapsed-time auto-kill deadline. */
  automaticKill?: false;
}

/** A session-attributed background job that is still active or stopping. */
export interface BackgroundJob extends BackgroundRunMeta {
  command: string;
  toolName: string;
  origin: BackgroundJobOrigin;
  /** Time the live handle became visible to this client. */
  startedAt: number;
  lastOutputAt?: number;
  state: "running" | "stopping";
}
