export interface StepDetail {
  tool_name: string;
  status: "running" | "completed" | "error" | "skipped" | "pending";
  stored: number;
  output?: string;
  message?: string;
  exit_code?: number | null;
  duration_ms?: number;
  command?: string;
}

export type StepStatus = StepDetail["status"];

export interface PipelineSummary {
  total_stored: number;
  success: boolean;
}

export function ensureStepsLength(prev: StepDetail[], idx: number, toolName: string): StepDetail[] {
  const next = [...prev];
  while (next.length <= idx) {
    next.push({ tool_name: "...", status: "pending", stored: 0 });
  }
  if (toolName) next[idx] = { ...next[idx], tool_name: toolName };
  return next;
}

export function computeSummary(steps: StepDetail[]): PipelineSummary {
  const totalStored = steps.reduce((sum, s) => sum + s.stored, 0);
  const allOk = steps.every((s) => s.status === "completed" || s.status === "skipped");
  return { total_stored: totalStored, success: allOk };
}

export function appendOutput(existing: string | undefined, newOutput: string, isStderr: boolean): string {
  const prefix = isStderr ? `[stderr] ${newOutput}` : newOutput;
  const MAX_LIVE = 8192;
  let updated = (existing ?? "") + prefix;
  if (updated.length > MAX_LIVE) {
    updated = updated.slice(updated.length - MAX_LIVE);
  }
  return updated;
}
