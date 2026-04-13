/**
 * Listen for `pipeline-event` from the Rust backend and bridge them
 * into the Zustand store as PipelineProgressBlock entries in the timeline.
 *
 * - On "started": create a full PipelineProgressBlock with all steps as pending.
 * - On "running" / "completed" / "error" / "skipped": update the matching step.
 * - When all steps are done: mark the pipeline block as completed/failed.
 */

import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import type { PipelineExecution, PipelineStepExecution } from "@/store";
import { useStore } from "@/store";

interface PipelineStepInfo {
  id: string;
  tool_name: string;
  command_template: string;
}

interface PipelineEventPayload {
  pipeline_id: string;
  step_id: string;
  step_index: number;
  total_steps: number;
  status: string;
  tool_name: string;
  message?: string;
  store_stats?: { stored_count: number };
  pipeline_name?: string;
  target?: string;
  all_steps?: PipelineStepInfo[];
  output?: string;
  duration_ms?: number;
  exit_code?: number;
}

function humanizeId(id: string): string {
  return id
    .replace(/_/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

/**
 * Map pipeline_id → timeline block id for routing subsequent events.
 */
const pipelineBlockMap = new Map<string, string>();

/**
 * Session ID where the most recent `run_pipeline` tool was dispatched.
 * Set by the tool_request handler to avoid a race between buffered AI
 * events and direct pipeline-event Tauri events.
 */
let pendingPipelineSessionId: string | null = null;

/** Called from tool-handlers when a run_pipeline (action=run) tool_request arrives. */
export function setPipelineSession(sessionId: string) {
  pendingPipelineSessionId = sessionId;
}

function resolveSessionForPipeline(): string | null {
  if (pendingPipelineSessionId) return pendingPipelineSessionId;
  const state = useStore.getState();
  for (const [sessionId, calls] of Object.entries(state.activeToolCalls)) {
    if (calls?.some((tc) => tc.name === "run_pipeline" && tc.status === "running")) {
      return sessionId;
    }
  }
  return state.activeSessionId ?? null;
}

export function usePipelineEvents() {
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let isMounted = true;

    console.log("[usePipelineEvents] Hook initialized, setting up listener");
    const setup = async () => {
      const unlisten = await listen<PipelineEventPayload>("pipeline-event", (event) => {
        const p = event.payload;
        const state = useStore.getState();

        console.log("[usePipelineEvents] Received pipeline-event:", p.status, p.pipeline_id, p.tool_name);

        const sessionId = resolveSessionForPipeline();
        if (!sessionId) {
          console.warn("[usePipelineEvents] No session found for pipeline event");
          return;
        }
        console.log("[usePipelineEvents] Resolved session:", sessionId);

        if (p.status === "started") {
          console.log("[usePipelineEvents] Started event, all_steps:", p.all_steps);
          if (!p.all_steps || p.all_steps.length === 0) {
            console.warn("[usePipelineEvents] No steps in started event, skipping");
            return;
          }

          const blockId = `pipeline-${p.pipeline_id}-${Date.now()}`;
          pipelineBlockMap.set(p.pipeline_id, blockId);

          const steps: PipelineStepExecution[] = p.all_steps.map((s) => ({
            stepId: s.id,
            name: humanizeId(s.id),
            command: s.tool_name,
            status: "pending" as const,
          }));

          const execution: PipelineExecution = {
            pipelineId: p.pipeline_id,
            pipelineName: p.pipeline_name ?? p.pipeline_id,
            target: p.target ?? "",
            steps,
            status: "running",
            startedAt: new Date().toISOString(),
          };

          console.log("[usePipelineEvents] Creating pipeline block:", blockId, "steps:", steps.length, "session:", sessionId);
          state.startPipelineExecution(sessionId, execution, blockId);

          // Verify it was added
          const verifyTimeline = useStore.getState().timelines[sessionId];
          console.log("[usePipelineEvents] Timeline after add:", verifyTimeline?.length, "blocks, types:", verifyTimeline?.map(b => b.type));
          return;
        }

        // For step-level events, find the block
        const blockId = pipelineBlockMap.get(p.pipeline_id);
        if (!blockId) return;

        const statusMap: Record<string, PipelineStepExecution["status"]> = {
          running: "running",
          completed: "success",
          error: "failed",
          skipped: "skipped",
        };
        const stepStatus = statusMap[p.status];
        if (!stepStatus) return;

        state.updatePipelineStep(sessionId, blockId, p.step_id, {
          status: stepStatus,
          name: p.tool_name || undefined,
          output: p.output ?? undefined,
          durationMs: p.duration_ms ?? undefined,
          exitCode: p.exit_code ?? undefined,
        });

        // Check if pipeline is finished (all steps completed/failed/skipped)
        const timeline = state.timelines[sessionId];
        if (timeline) {
          const block = timeline.find((b) => b.id === blockId);
          if (block && block.type === "pipeline_progress") {
            const allDone = block.data.steps.every(
              (s) => s.status === "success" || s.status === "failed" || s.status === "skipped"
            );
            if (allDone) {
              const hasFailed = block.data.steps.some((s) => s.status === "failed");
              state.completePipelineExecution(
                sessionId,
                blockId,
                hasFailed ? "failed" : "completed"
              );
              pipelineBlockMap.delete(p.pipeline_id);
              pendingPipelineSessionId = null;
            }
          }
        }
      });

      if (isMounted) {
        unlistenRef.current = unlisten;
      } else {
        unlisten();
      }
    };

    setup();

    return () => {
      isMounted = false;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, []);
}
