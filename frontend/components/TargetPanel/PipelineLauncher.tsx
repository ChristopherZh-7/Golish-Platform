import { useCallback, useEffect, useRef, useState } from "react";
import { copyToClipboard } from "@/lib/clipboard";
import { formatDurationLong } from "@/lib/time";
import { cn } from "@/lib/utils";
import { CustomSelect } from "@/components/ui/custom-select";
import { type StepDetail } from "./pipelineValidation";
import { usePipelineForm } from "./hooks/usePipelineForm";
import {
  ChevronDown,
  ChevronRight,
  GitBranch,
  Loader2,
  Play,
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Clock,
  SkipForward,
  Copy,
  Check,
} from "lucide-react";

interface PipelineLauncherProps {
  targetId: string;
  targetValue: string;
}

const STATUS_CONFIG = {
  running: {
    icon: (i: number) => <Loader2 key={i} className="w-3.5 h-3.5 animate-spin text-blue-400" />,
    border: "border-blue-500/30",
    bg: "bg-blue-500/[0.06]",
    text: "text-foreground",
    badge: "bg-blue-500/20 text-blue-300",
  },
  completed: {
    icon: (i: number) => <CheckCircle2 key={i} className="w-3.5 h-3.5 text-emerald-400" />,
    border: "border-emerald-500/20",
    bg: "bg-emerald-500/[0.04]",
    text: "text-foreground/90",
    badge: "bg-emerald-500/15 text-emerald-300",
  },
  error: {
    icon: (i: number) => <AlertTriangle key={i} className="w-3.5 h-3.5 text-red-400" />,
    border: "border-red-500/25",
    bg: "bg-red-500/[0.05]",
    text: "text-foreground/90",
    badge: "bg-red-500/15 text-red-300",
  },
  skipped: {
    icon: (i: number) => <SkipForward key={i} className="w-3.5 h-3.5 text-zinc-500" />,
    border: "border-zinc-700/30",
    bg: "bg-transparent",
    text: "text-zinc-500",
    badge: "bg-zinc-700/30 text-zinc-400",
  },
  pending: {
    icon: (i: number) => <div key={i} className="w-3.5 h-3.5 rounded-full border-2 border-zinc-600" />,
    border: "border-zinc-700/20",
    bg: "bg-transparent",
    text: "text-zinc-600",
    badge: "",
  },
} as const;

function RunningIndicator() {
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    const interval = setInterval(() => setElapsed((e) => e + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div className="flex items-center gap-3 px-3 py-3 rounded bg-blue-500/[0.04] mt-1">
      <div className="flex gap-1">
        <div className="w-1.5 h-1.5 rounded-full bg-blue-400 animate-bounce" style={{ animationDelay: "0ms" }} />
        <div className="w-1.5 h-1.5 rounded-full bg-blue-400 animate-bounce" style={{ animationDelay: "150ms" }} />
        <div className="w-1.5 h-1.5 rounded-full bg-blue-400 animate-bounce" style={{ animationDelay: "300ms" }} />
      </div>
      <span className="text-[10px] text-blue-300">Executing...</span>
      <span className="text-[10px] text-zinc-500 ml-auto font-mono">{elapsed}s</span>
    </div>
  );
}

function LiveOutput({ output, isLive, onCopy, copied }: {
  output: string; isLive: boolean; onCopy: () => void; copied: boolean;
}) {
  const preRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    if (isLive && preRef.current) {
      preRef.current.scrollTop = preRef.current.scrollHeight;
    }
  }, [output, isLive]);

  return (
    <div className="relative group mt-1">
      <div className="absolute right-1.5 top-1.5 z-10 opacity-0 group-hover:opacity-100 transition-opacity">
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onCopy();
          }}
          className="flex items-center gap-1 px-2 py-1 rounded text-[10px] bg-zinc-700/60 hover:bg-zinc-700/80 text-zinc-300 transition-colors"
        >
          {copied ? (
            <><Check className="w-3 h-3 text-emerald-400" /> Copied</>
          ) : (
            <><Copy className="w-3 h-3" /> Copy</>
          )}
        </button>
      </div>
      <pre
        ref={preRef}
        className={cn(
          "px-2.5 py-2 text-[10px] leading-[1.6] font-mono max-h-[250px] overflow-auto whitespace-pre-wrap break-all rounded",
          isLive
            ? "bg-zinc-900/80 text-emerald-300/90 border border-emerald-500/10"
            : "bg-black/20 text-zinc-300",
        )}
      >
        {output}
        {isLive && <span className="animate-pulse text-emerald-400">|</span>}
      </pre>
    </div>
  );
}

function StepRow({ step, index }: { step: StepDetail; index: number }) {
  const [manualExpanded, setManualExpanded] = useState<boolean | null>(null);
  const [copied, setCopied] = useState(false);
  const autoExpand = step.status === "running" && !!step.output;
  const expanded = manualExpanded ?? autoExpand;
  const hasContent = !!step.output || (!!step.message && (step.status === "error" || step.status === "skipped"));
  const isExpandable = hasContent && step.status !== "pending";
  const cfg = STATUS_CONFIG[step.status];

  const copyOutput = useCallback(async () => {
    if (step.output && await copyToClipboard(step.output)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  }, [step.output]);

  return (
    <div className={cn("rounded-md border transition-all", cfg.border, cfg.bg)}>
      <button
        type="button"
        onClick={() => isExpandable && setManualExpanded(!expanded)}
        className={cn(
          "flex items-center gap-2.5 w-full px-3 py-2 text-left transition-colors",
          isExpandable && "hover:bg-white/[0.03] cursor-pointer",
          !isExpandable && "cursor-default",
        )}
      >
        <span className={cn(
          "w-5 h-5 rounded-full flex items-center justify-center text-[9px] font-bold flex-shrink-0",
          step.status === "running" ? "bg-blue-500/20 text-blue-300" :
          step.status === "completed" ? "bg-emerald-500/15 text-emerald-400" :
          step.status === "error" ? "bg-red-500/15 text-red-400" :
          step.status === "skipped" ? "bg-zinc-700/30 text-zinc-500" :
          "bg-zinc-700/20 text-zinc-600",
        )}>
          {index + 1}
        </span>

        {cfg.icon(index)}

        <span className={cn("text-[11px] font-semibold min-w-0 truncate", cfg.text)}>
          {step.tool_name}
        </span>

        <div className="flex items-center gap-2 ml-auto flex-shrink-0">
          {step.status === "running" && (
            <span className="text-[10px] font-medium text-blue-400 bg-blue-500/15 px-2 py-0.5 rounded-full animate-pulse">
              Running...
            </span>
          )}
          {step.stored > 0 && (
            <span className="text-[10px] font-medium text-blue-300 bg-blue-500/15 px-2 py-0.5 rounded-full">
              +{step.stored} items
            </span>
          )}
          {step.exit_code != null && step.exit_code !== 0 && (
            <span className="text-[10px] font-medium text-red-300 bg-red-500/15 px-2 py-0.5 rounded-full">
              exit {step.exit_code}
            </span>
          )}
          {step.duration_ms != null && step.duration_ms > 0 && (
            <span className="text-[10px] text-zinc-400 flex items-center gap-1">
              <Clock className="w-3 h-3" />
              {formatDurationLong(step.duration_ms)}
            </span>
          )}
          {step.status === "skipped" && (
            <span className="text-[10px] text-zinc-500 italic">skipped</span>
          )}
          {isExpandable && (
            expanded
              ? <ChevronDown className="w-3.5 h-3.5 text-zinc-400" />
              : <ChevronRight className="w-3.5 h-3.5 text-zinc-500" />
          )}
        </div>
      </button>

      {step.status === "running" && (
        <div className="border-t border-blue-500/10 mx-2 mb-2">
          {step.output ? (
            <LiveOutput
              output={step.output}
              isLive={true}
              onCopy={copyOutput}
              copied={copied}
            />
          ) : (
            <RunningIndicator />
          )}
        </div>
      )}

      {expanded && hasContent && step.status !== "running" && (
        <div className="border-t border-white/[0.06] mx-2 mb-2">
          {step.message && step.status === "skipped" && (
            <div className="px-2 py-2 text-[10px] text-zinc-400 italic">
              {step.message}
            </div>
          )}
          {step.output && (
            <LiveOutput
              output={step.output}
              isLive={false}
              onCopy={copyOutput}
              copied={copied}
            />
          )}
          {step.message && step.status === "error" && (
            <div className="px-2.5 py-2 text-[10px] text-red-300 font-mono bg-red-500/[0.05] rounded mt-1">
              {step.message}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function PipelineLauncher({ targetValue }: PipelineLauncherProps) {
  const {
    expanded, setExpanded,
    pipelines, selected, setSelected,
    running, progress, steps, summary,
    runPipeline, cancelPipeline,
  } = usePipelineForm(targetValue);

  const hasResults = steps.length > 0;

  return (
    <div className="rounded-lg border border-zinc-700/30 overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-3 py-2.5 text-left hover:bg-white/[0.02] transition-colors"
      >
        {expanded ? (
          <ChevronDown className="w-3.5 h-3.5 text-zinc-400" />
        ) : (
          <ChevronRight className="w-3.5 h-3.5 text-zinc-500" />
        )}
        <GitBranch className="w-4 h-4 text-blue-400" />
        <span className="text-[12px] font-semibold text-foreground/90">Pipeline</span>
        {running && (
          <>
            <Loader2 className="w-3.5 h-3.5 animate-spin text-blue-400 ml-1" />
            {progress && (
              <span className="text-[10px] text-zinc-400 ml-auto">
                Step {progress.step}/{progress.total}: <span className="text-blue-300 font-medium">{progress.tool}</span>
              </span>
            )}
            <span className="text-[10px] font-medium text-blue-400 bg-blue-500/15 px-2 py-0.5 rounded-full ml-1">
              Running
            </span>
          </>
        )}
        {!running && summary && (
          <span className="text-[10px] ml-auto flex items-center gap-1.5">
            {summary.success ? (
              <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
            ) : (
              <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
            )}
            <span className="text-zinc-300 font-medium">{summary.total_stored} items stored</span>
          </span>
        )}
      </button>

      {expanded && (
        <div className="border-t border-zinc-700/20 px-3 py-2.5 space-y-2">
          <div className="flex items-center gap-2">
            <CustomSelect
              value={selected?.id ?? ""}
              onChange={(v) => {
                const p = pipelines.find((pp) => pp.id === v);
                if (p) setSelected(p);
              }}
              options={pipelines.map((p) => ({ value: p.id, label: `${p.name} (${p.steps.length} steps)` }))}
              className="flex-1"
              size="sm"
            />
            {running ? (
              <button
                type="button"
                onClick={cancelPipeline}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] font-semibold transition-colors text-red-400 hover:bg-red-500/10 border border-red-500/20"
              >
                <XCircle className="w-3.5 h-3.5" /> Cancel
              </button>
            ) : (
              <button
                type="button"
                disabled={!selected}
                onClick={runPipeline}
                className={cn(
                  "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] font-semibold transition-colors",
                  !selected
                    ? "bg-zinc-800/40 text-zinc-600 cursor-not-allowed"
                    : "bg-blue-500/15 text-blue-300 hover:bg-blue-500/25 border border-blue-500/25",
                )}
              >
                <Play className="w-3.5 h-3.5" /> Run
              </button>
            )}
          </div>

          {hasResults ? (
            <div className="space-y-1.5">
              {steps.map((s, i) => (
                <StepRow key={`${s.tool_name}-${i}`} step={s} index={i} />
              ))}
            </div>
          ) : selected && !running ? (
            <div className="flex flex-wrap gap-1.5">
              {selected.steps.map((s) => (
                <span
                  key={s.id}
                  className="px-2 py-1 text-[10px] rounded-md bg-zinc-800/40 text-zinc-400 border border-zinc-700/20 font-medium"
                >
                  {s.tool_name}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      )}
    </div>
  );
}
