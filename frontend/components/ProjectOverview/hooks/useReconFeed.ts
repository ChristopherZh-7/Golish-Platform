/* ── Recon-pipeline activity feed hook ────────────────────────────────
 *
 * Owns all of the reactive plumbing for the project-overview activity feed:
 * the running list of feed entries, the pipeline-progress snapshot, and the
 * Tauri `ai-event` listener that translates backend events into feed
 * mutations. Extracted from the original `ProjectOverview.tsx` (840 lines)
 * so the visual component can be rendered as a thin shell.
 *
 * Behaviour is intentionally identical to the previous inline implementation
 * — the only thing that has changed is where the code lives.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { triggerAutoRecon } from "@/lib/ai";
import { onEvent } from "@/lib/events";
import { logger } from "@/lib/logger";
import {
  type ActivityItem,
  type FeedEntry,
  MAX_ENTRIES,
  type PipelineProgress,
  RECON_STEPS,
  type StepGroup,
} from "../types";
import { friendly } from "../utils";

interface UseReconFeedResult {
  feed: FeedEntry[];
  progress: PipelineProgress | null;
  pipelineActive: boolean;
}

/**
 * Wires up the activity feed and pipeline-progress state for a project
 * overview tab. Side-effects:
 *  - subscribes to Tauri `ai-event` for the lifetime of the component
 *  - kicks off any pending auto-recon stashed on `window.__PENDING_RECON__`
 *    by `HomeView` immediately after project creation
 *  - re-fetches targets when relevant pipeline events finish (via the
 *    caller-provided `onTargetsRefresh` callback)
 */
export function useReconFeed(
  sessionId: string,
  onTargetsRefresh: () => void | Promise<void>
): UseReconFeedResult {
  const [feed, setFeed] = useState<FeedEntry[]>([]);
  const [progress, setProgress] = useState<PipelineProgress | null>(null);
  const [pipelineActive, setPipelineActive] = useState(false);
  const seqRef = useRef(0);
  const activeStepRef = useRef<string | null>(null);

  const nextId = useCallback(() => `e-${++seqRef.current}`, []);

  const pushItem = useCallback(
    (item: Omit<ActivityItem, "id">) => {
      const id = nextId();
      const entry: ActivityItem = { ...item, id };

      setFeed((prev) => {
        const activeStepId = activeStepRef.current;
        if (activeStepId) {
          return prev.map((e) => {
            if (e.type === "step" && e.data.id === activeStepId) {
              return { ...e, data: { ...e.data, children: [...e.data.children, entry] } };
            }
            return e;
          });
        }
        const next = [...prev, { type: "item" as const, data: entry }];
        return next.length > MAX_ENTRIES ? next.slice(-MAX_ENTRIES) : next;
      });
    },
    [nextId]
  );

  const pushStep = useCallback(
    (stepName: string, stepIndex: number, totalSteps: number) => {
      setFeed((prev) => {
        // Find existing pending step and activate it
        const idx = prev.findIndex(
          (e) => e.type === "step" && e.data.stepName === stepName && e.data.status === "pending"
        );
        if (idx >= 0) {
          const entry = prev[idx] as { type: "step"; data: StepGroup };
          activeStepRef.current = entry.data.id;
          const updated = [...prev];
          updated[idx] = {
            type: "step",
            data: { ...entry.data, status: "running", startTs: Date.now() },
          };
          return updated;
        }
        // Fallback: create new step if not pre-populated
        const id = nextId();
        activeStepRef.current = id;
        const step: StepGroup = {
          id,
          stepName,
          status: "running",
          startTs: Date.now(),
          children: [],
        };
        const next = [...prev, { type: "step" as const, data: step }];
        return next.length > MAX_ENTRIES ? next.slice(-MAX_ENTRIES) : next;
      });

      setProgress((prev) => {
        const stepNames = prev?.stepNames ?? RECON_STEPS.slice(0, totalSteps);
        return {
          status: "running",
          totalSteps,
          completedSteps: stepIndex,
          currentStepIndex: stepIndex,
          currentStepName: stepName,
          stepNames,
        };
      });
    },
    [nextId]
  );

  const completeStep = useCallback((stepName: string, output?: string, durationMs?: number) => {
    activeStepRef.current = null;
    setFeed((prev) =>
      prev.map((e) => {
        if (e.type === "step" && e.data.stepName === stepName && e.data.status === "running") {
          return { ...e, data: { ...e.data, status: "completed", output, durationMs } };
        }
        return e;
      })
    );
    setProgress((prev) => (prev ? { ...prev, completedSteps: prev.completedSteps + 1 } : prev));
  }, []);

  const failStep = useCallback((stepName: string) => {
    activeStepRef.current = null;
    setFeed((prev) =>
      prev.map((e) => {
        if (e.type === "step" && e.data.stepName === stepName && e.data.status === "running") {
          return { ...e, data: { ...e.data, status: "failed" } };
        }
        return e;
      })
    );
  }, []);

  // Subscribe to backend events; identical wiring to the original component.
  // sessionId is captured by the closure, but the listener doesn't filter by
  // sessionId — preserving the prior behaviour. onTargetsRefresh is read via
  // a ref so we don't re-subscribe whenever the parent re-renders.
  const refreshRef = useRef(onTargetsRefresh);
  refreshRef.current = onTargetsRefresh;

  useEffect(() => {
    let cancelled = false;
    const cleanups: (() => void)[] = [];

    (async () => {
      const unlisten = await onEvent("ai-event", (payload) => {
        if (cancelled) return;
        const p = payload as unknown as Record<string, unknown>;
        const now = Date.now();

        switch (p.type) {
          case "started":
            pushItem({ kind: "agent_thinking", label: "AI is thinking...", ts: now });
            break;

          case "tool_request":
          case "tool_auto_approved": {
            const name = friendly(p.tool_name as string);
            let detail: string | undefined;
            if (p.args && typeof p.args === "object") {
              const args = p.args as Record<string, unknown>;
              const val = args.command ?? args.query ?? args.path ?? args.url ?? args.target;
              if (val) detail = String(val).slice(0, 120);
            }
            pushItem({ kind: "tool_start", label: name, detail, ts: now });
            break;
          }

          case "tool_result": {
            const name = friendly(p.tool_name as string);
            const ok = p.success as boolean;
            let detail: string | undefined;
            if (typeof p.result === "string") detail = (p.result as string).slice(0, 150);
            pushItem({
              kind: ok ? "tool_done" : "tool_error",
              label: `${name} ${ok ? "done" : "failed"}`,
              detail,
              ts: now,
            });
            break;
          }

          case "completed":
            pushItem({
              kind: "agent_done",
              label: "AI turn done",
              ts: now,
              durationMs: p.duration_ms as number | undefined,
            });
            break;

          case "error":
            pushItem({
              kind: "tool_error",
              label: `Error: ${(p.message as string)?.slice(0, 80)}`,
              ts: now,
            });
            break;

          case "sub_agent_started":
            pushItem({
              kind: "sub_agent_start",
              label: `Sub-agent: ${p.agent_name}`,
              detail: p.task as string,
              ts: now,
            });
            break;

          case "sub_agent_completed":
            pushItem({
              kind: "sub_agent_done",
              label: "Sub-agent done",
              ts: now,
              durationMs: p.duration_ms as number | undefined,
            });
            break;

          case "workflow_started": {
            setPipelineActive(true);
            setProgress({
              status: "running",
              totalSteps: RECON_STEPS.length,
              completedSteps: 0,
              currentStepIndex: 0,
              currentStepName: RECON_STEPS[0],
              stepNames: [...RECON_STEPS],
            });
            // Pre-populate all steps as "pending" so user can see the full plan
            const pendingSteps: FeedEntry[] = RECON_STEPS.map((name) => ({
              type: "step" as const,
              data: {
                id: `step-${name}-${now}`,
                stepName: name,
                status: "pending" as const,
                startTs: now,
                children: [],
              },
            }));
            setFeed((prev) => [
              ...prev,
              {
                type: "item",
                data: {
                  id: nextId(),
                  kind: "pipeline_start",
                  label: `Pipeline: ${p.workflow_name ?? "Recon"}`,
                  ts: now,
                },
              },
              ...pendingSteps,
            ]);
            break;
          }

          case "workflow_step_started":
            pushStep(
              p.step_name as string,
              (p.step_index as number) ?? 0,
              (p.total_steps as number) ?? RECON_STEPS.length
            );
            break;

          case "workflow_step_completed":
            completeStep(
              p.step_name as string,
              (p.output as string | null) ?? undefined,
              p.duration_ms as number | undefined
            );
            break;

          case "workflow_completed":
            setPipelineActive(false);
            setProgress((prev) =>
              prev ? { ...prev, status: "completed", completedSteps: prev.totalSteps } : prev
            );
            pushItem({
              kind: "pipeline_done",
              label: "Pipeline complete",
              ts: now,
              durationMs: p.total_duration_ms as number | undefined,
            });
            void refreshRef.current();
            break;

          case "workflow_error":
          case "workflow_failed":
            setPipelineActive(false);
            setProgress((prev) => (prev ? { ...prev, status: "failed" } : prev));
            if (activeStepRef.current) failStep((p.step_name as string) ?? "");
            pushItem({
              kind: "pipeline_error",
              label: `Pipeline error: ${(p.error as string)?.slice(0, 80) ?? "unknown"}`,
              ts: now,
            });
            break;

          case "server_tool_started":
            pushItem({ kind: "tool_start", label: friendly(p.tool_name as string), ts: now });
            break;

          case "web_search_result":
            pushItem({ kind: "tool_done", label: "Web search done", ts: now });
            break;

          case "web_fetch_result":
            pushItem({
              kind: "tool_done",
              label: `Fetched: ${(p.url as string)?.slice(0, 80)}`,
              ts: now,
            });
            break;
        }
      });
      if (cancelled) {
        unlisten();
        return;
      }
      cleanups.push(unlisten);

      // Pick up pending recon from project creation (set by HomeView)
      const pending = window.__PENDING_RECON__;
      if (pending && !cancelled) {
        delete window.__PENDING_RECON__;
        triggerAutoRecon(
          pending.sessionId,
          pending.targets,
          pending.projectName,
          pending.projectPath
        ).catch((e) => logger.error("Failed to run pending recon:", e));
      }
    })();

    return () => {
      cancelled = true;
      cleanups.forEach((fn) => {
        fn();
      });
    };
    // sessionId is intentionally part of the deps so a switch resets the
    // listener; nextId/pushItem/pushStep/completeStep/failStep are stable.
  }, [sessionId, nextId, pushItem, pushStep, completeStep, failStep]);

  return { feed, progress, pipelineActive };
}
