import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getProjectPath } from "@/lib/projects";
import { type PipelineSummary } from "@/lib/pentest/pipeline-types";
import {
  type StepDetail,
  type PipelineSummary as PipelineSummaryResult,
  ensureStepsLength,
  appendOutput,
  computeSummary,
} from "../pipelineValidation";

export function usePipelineForm(targetValue: string) {
  const [expanded, setExpanded] = useState(false);
  const [pipelines, setPipelines] = useState<PipelineSummary[]>([]);
  const [selected, setSelected] = useState<PipelineSummary | null>(null);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<{ step: number; total: number; tool: string } | null>(null);
  const [steps, setSteps] = useState<StepDetail[]>([]);
  const [summary, setSummary] = useState<PipelineSummaryResult | null>(null);
  const activeRunId = useRef<string | null>(null);

  useEffect(() => {
    if (!expanded) return;
    let cancelled = false;
    (async () => {
      try {
        const list = await invoke<PipelineSummary[]>("pipeline_list", {
          projectPath: getProjectPath(),
        });
        if (!cancelled) {
          setPipelines(Array.isArray(list) ? list : []);
          if (list.length > 0 && !selected) setSelected(list[0]);
        }
      } catch {
        if (!cancelled) setPipelines([]);
      }
    })();
    return () => { cancelled = true; };
  }, [expanded]);

  useEffect(() => {
    const unlistenPromise = listen<{
      run_id: string;
      step_index: number;
      total_steps: number;
      tool_name: string;
      status: string;
      store_stats?: { stored_count: number; parsed_count: number; skipped_count: number; errors: string[] };
      message?: string;
      output?: string;
      duration_ms?: number;
      exit_code?: number | null;
      pipeline_name?: string;
      target?: string;
      all_steps?: { id: string; tool_name: string; command_template: string }[];
    }>("pipeline-event", (event) => {
      const p = event.payload;

      if (p.status === "started" && p.all_steps) {
        if (p.target !== targetValue) return;
        activeRunId.current = p.run_id;
        setSteps(p.all_steps.map((s) => ({
          tool_name: s.tool_name,
          status: "pending",
          stored: 0,
          command: s.command_template,
        })));
        setRunning(true);
        setSummary(null);
        return;
      }

      if (p.run_id !== activeRunId.current) return;

      if (p.status === "output" && p.output) {
        setSteps((prev) => {
          const next = ensureStepsLength(prev, p.step_index, p.tool_name);
          next[p.step_index] = {
            ...next[p.step_index],
            output: appendOutput(next[p.step_index].output, p.output!, p.message === "stderr"),
          };
          return next;
        });
        return;
      }

      if (p.status === "running") {
        setRunning(true);
        setProgress({ step: p.step_index + 1, total: p.total_steps, tool: p.tool_name });
        setSteps((prev) => {
          const next = ensureStepsLength(prev, p.step_index, p.tool_name);
          next[p.step_index] = { ...next[p.step_index], status: "running" };
          return next;
        });
        return;
      }

      if (p.status === "skipped") {
        setSteps((prev) => {
          const next = ensureStepsLength(prev, p.step_index, p.tool_name);
          next[p.step_index] = {
            ...next[p.step_index],
            status: "skipped",
            message: p.message,
          };
          return next;
        });
        return;
      }

      if (p.status === "cancelled") {
        setRunning(false);
        setProgress(null);
        setSteps((prev) => prev.map((s) =>
          s.status === "running" || s.status === "pending"
            ? { ...s, status: "skipped" as const, message: "Cancelled" }
            : s,
        ));
        setSummary({ total_stored: 0, success: false });
        return;
      }

      if (p.status === "completed" || p.status === "error") {
        setSteps((prev) => {
          const next = ensureStepsLength(prev, p.step_index, p.tool_name);
          next[p.step_index] = {
            ...next[p.step_index],
            status: p.status as "completed" | "error",
            stored: p.store_stats?.stored_count ?? 0,
            output: p.output ?? next[p.step_index].output,
            message: p.message,
            exit_code: p.exit_code,
            duration_ms: p.duration_ms,
          };
          return next;
        });
        if (p.step_index + 1 >= p.total_steps) {
          setRunning(false);
          setProgress(null);
          setSteps((prev) => {
            setSummary(computeSummary(prev));
            return prev;
          });
        }
      }
    });
    return () => { unlistenPromise.then((f) => f()); };
  }, [targetValue]);

  const runPipeline = useCallback(async () => {
    if (!selected || running) return;
    setSteps([]);
    setSummary(null);
    setRunning(true);
    try {
      await invoke("pipeline_execute", {
        pipeline: selected,
        target: targetValue,
        projectPath: getProjectPath(),
      });
    } catch {
      setSummary({ total_stored: 0, success: false });
    }
    setRunning(false);
    setProgress(null);
  }, [selected, targetValue, running]);

  const cancelPipeline = useCallback(async () => {
    try { await invoke("pipeline_cancel"); } catch { /* ignore */ }
  }, []);

  return {
    expanded,
    setExpanded,
    pipelines,
    selected,
    setSelected,
    running,
    progress,
    steps,
    summary,
    runPipeline,
    cancelPipeline,
  };
}
