import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  Loader2,
  Wrench,
} from "lucide-react";
import { useId, useState } from "react";
import { JsonView } from "@/components/JsonView/JsonView";
import { stripAnsiForDisplay } from "@/lib/ansi";
import { copyToClipboard } from "@/lib/clipboard";
import { cn } from "@/lib/utils";
import type { SubAgentToolCall } from "@/store";
import {
  type HttpExecutionPresentation,
  type HttpRequestPresentation,
  presentToolActivity,
  summarizeToolActivities,
} from "./toolActivityPresentation";

function displayTime(value: string | number | undefined): string {
  if (value === undefined) return "";
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function toolStatusLabel(status: SubAgentToolCall["status"]): string {
  if (status === "backgrounded") return "Backgrounded";
  if (status === "running") return "Running";
  if (status === "completed") return "Completed";
  if (status === "interrupted") return "Interrupted";
  return "Failed";
}

function toolStatusClass(status: SubAgentToolCall["status"]): string {
  if (status === "error" || status === "interrupted") {
    return "border-rose-400/25 bg-rose-400/10 text-rose-200";
  }
  if (status === "running" || status === "backgrounded") {
    return "border-sky-400/25 bg-sky-400/10 text-sky-200";
  }
  return "border-emerald-400/25 bg-emerald-400/10 text-emerald-200";
}

function ToolStatusIcon({ status }: { status: SubAgentToolCall["status"] }) {
  if (status === "running" || status === "backgrounded") {
    return <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />;
  }
  if (status === "error" || status === "interrupted") {
    return <AlertTriangle className="h-3 w-3" aria-hidden="true" />;
  }
  return <CheckCircle2 className="h-3 w-3" aria-hidden="true" />;
}

function groupStatus(tools: readonly SubAgentToolCall[]): SubAgentToolCall["status"] {
  if (tools.some((tool) => tool.status === "error")) return "error";
  if (tools.some((tool) => tool.status === "interrupted")) return "interrupted";
  if (tools.some((tool) => tool.status === "running")) return "running";
  if (tools.some((tool) => tool.status === "backgrounded")) return "backgrounded";
  return "completed";
}

function humanizeToken(value: string): string {
  const text = value.replace(/[_-]/g, " ").trim();
  return text ? `${text[0]?.toUpperCase() ?? ""}${text.slice(1)}` : value;
}

function httpResultLabel(request: HttpRequestPresentation): string {
  if (request.statusCode !== null) return String(request.statusCode);
  if (request.errorClass) return humanizeToken(request.errorClass);
  if (request.networkAttempted === false) return "Not sent";
  return "No response";
}

function networkAttemptLabel(value: boolean | null): string {
  if (value === true) return "Attempted";
  if (value === false) return "Not attempted";
  return "Unknown";
}

function HttpRequestRow({ request }: { request: HttpRequestPresentation }) {
  const [expanded, setExpanded] = useState(false);
  const detailsId = useId();
  const response = request.response;

  return (
    <div className="border-t border-white/10 first:border-t-0">
      <button
        type="button"
        aria-label={`${request.method} ${request.path} HTTP observation`}
        aria-expanded={expanded}
        aria-controls={detailsId}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-white/[0.04]"
        onClick={() => setExpanded((current) => !current)}
      >
        <code className="w-10 shrink-0 font-mono text-[10px] font-semibold text-sky-200/90">
          {request.method}
        </code>
        <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-slate-200">
          {request.path}
        </code>
        <span className="shrink-0 text-[10px] text-slate-400">{httpResultLabel(request)}</span>
        {request.verdict && (
          <span
            className={cn(
              "shrink-0 rounded border px-1.5 py-0.5 text-[9px]",
              request.verdict === "suspicious"
                ? "border-amber-300/25 bg-amber-300/10 text-amber-100"
                : request.verdict === "controlled"
                  ? "border-emerald-300/25 bg-emerald-300/10 text-emerald-100"
                  : "border-slate-400/20 bg-slate-400/10 text-slate-300"
            )}
          >
            {humanizeToken(request.verdict)}
          </span>
        )}
        {expanded ? (
          <ChevronDown className="h-3 w-3 shrink-0 text-slate-500" aria-hidden="true" />
        ) : (
          <ChevronRight className="h-3 w-3 shrink-0 text-slate-500" aria-hidden="true" />
        )}
      </button>
      {expanded && (
        <section
          id={detailsId}
          aria-label={`${request.method} ${request.path} HTTP details`}
          className="space-y-2 border-t border-white/[0.06] bg-black/20 px-3 py-2 text-[10px] text-slate-400"
        >
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <span>
              Network{" "}
              <strong className="font-medium text-slate-300">
                {networkAttemptLabel(request.networkAttempted)}
              </strong>
            </span>
            {request.errorClass && (
              <span>
                Error <code className="font-mono text-rose-200/80">{request.errorClass}</code>
              </span>
            )}
          </div>
          {request.queryBindings.length > 0 && (
            <div>
              <div className="mb-1 text-[9px] font-medium uppercase tracking-wide text-slate-500">
                Query overrides
              </div>
              <div className="flex flex-wrap gap-1.5">
                {request.queryBindings.map((binding) => (
                  <code
                    key={`${binding.name}\u0000${binding.value}`}
                    className="rounded bg-white/[0.05] px-1.5 py-0.5 font-mono text-slate-300"
                  >
                    {binding.name}=<span className="text-sky-200/80">{binding.value}</span>
                  </code>
                ))}
              </div>
            </div>
          )}
          {response && (
            <div>
              <div className="mb-1 text-[9px] font-medium uppercase tracking-wide text-slate-500">
                Response fingerprint
              </div>
              <div className="space-y-1 font-mono text-slate-300">
                <div className="flex flex-wrap gap-x-3 gap-y-1">
                  {response.contentTypeFamily && <span>{response.contentTypeFamily}</span>}
                  {response.capturedLength !== null && (
                    <span>{response.capturedLength.toLocaleString("en-US")} bytes captured</span>
                  )}
                  {response.declaredLength !== null && (
                    <span>{response.declaredLength.toLocaleString("en-US")} bytes declared</span>
                  )}
                  {response.truncated === true && (
                    <span className="text-amber-200/80">Truncated</span>
                  )}
                </div>
                {response.prefixSha256 && (
                  <div className="break-all text-slate-500">
                    SHA-256 <span className="text-slate-400">{response.prefixSha256}</span>
                  </div>
                )}
              </div>
            </div>
          )}
        </section>
      )}
    </div>
  );
}

function HttpExecutionPanel({
  action,
  execution,
}: {
  action: string;
  execution: HttpExecutionPresentation;
}) {
  return (
    <section
      aria-label={`${action} HTTP requests`}
      className="overflow-hidden rounded-md border border-border/50 bg-[#090b0e]"
    >
      <div className="flex items-center gap-2 border-b border-white/10 px-3 py-2">
        <span className="text-[10px] font-medium uppercase tracking-wide text-slate-300">
          HTTP requests
        </span>
        <span className="rounded border border-sky-300/20 bg-sky-300/10 px-1.5 py-0.5 text-[9px] text-sky-200/80">
          In process
        </span>
        {execution.selectedCount !== null && (
          <span className="ml-auto text-[9px] text-slate-500">
            {execution.selectedCount} {execution.selectedCount === 1 ? "endpoint" : "endpoints"}{" "}
            selected
          </span>
        )}
      </div>
      {execution.origin && (
        <div className="flex gap-2 border-b border-white/10 px-3 py-2 text-[10px]">
          <span className="shrink-0 text-slate-500">Origin</span>
          <code className="min-w-0 break-all font-mono text-sky-200/80">{execution.origin}</code>
        </div>
      )}
      {execution.requests.length > 0 ? (
        <div>
          {execution.requests.map((request) => (
            <HttpRequestRow key={request.endpointId} request={request} />
          ))}
        </div>
      ) : (
        <div className="px-3 py-3 text-[10px] text-slate-400">
          <p>No HTTP requests were sent</p>
        </div>
      )}
    </section>
  );
}

function ToolActivityRow({ tool }: { tool: SubAgentToolCall }) {
  const [expanded, setExpanded] = useState(false);
  const [rawExpanded, setRawExpanded] = useState(false);
  const disclosureId = useId();
  const rawId = useId();
  const presentation = presentToolActivity(tool);
  const active = tool.status === "running" || tool.status === "backgrounded";
  const action = active ? presentation.action : presentation.completedAction;
  const statusLabel = toolStatusLabel(tool.status);
  const runnerLabel = presentation.runner ?? "AI Tool";
  const hasTerminalDetails = Boolean(
    presentation.command ||
      presentation.stdout ||
      presentation.stderr ||
      presentation.jobId ||
      presentation.hint
  );

  return (
    <article className="border-t border-border/35 first:border-t-0">
      <button
        type="button"
        aria-label={`${action} · ${runnerLabel} · ${statusLabel}`}
        aria-expanded={expanded}
        aria-controls={disclosureId}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-accent/20"
        onClick={() => setExpanded((current) => !current)}
      >
        <span
          className={cn(
            "grid h-5 w-5 shrink-0 place-items-center rounded-full border",
            toolStatusClass(tool.status)
          )}
        >
          <ToolStatusIcon status={tool.status} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[11px] font-medium text-foreground/90">
            {action}
          </span>
          <span className="mt-0.5 block truncate text-[10px] text-muted-foreground">
            {runnerLabel}
            {presentation.subject ? ` · ${presentation.subject}` : ""}
          </span>
        </span>
        <span
          role="status"
          className={cn(
            "shrink-0 rounded border px-1.5 py-0.5 text-[9px]",
            toolStatusClass(tool.status)
          )}
        >
          {statusLabel}
        </span>
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        )}
      </button>
      {expanded && (
        <div id={disclosureId} className="space-y-2 border-t border-border/25 bg-black/10 p-2.5">
          {presentation.execution?.kind === "http" && (
            <HttpExecutionPanel action={presentation.action} execution={presentation.execution} />
          )}
          {hasTerminalDetails && (
            <section
              aria-label={
                presentation.command
                  ? `${presentation.action} command and output`
                  : `${presentation.action} execution details`
              }
              className="overflow-hidden rounded-md border border-border/50 bg-[#090b0e]"
            >
              {presentation.command && (
                <div className="relative border-b border-white/10 px-3 py-2 pr-10">
                  {presentation.commandProvenance === "requested" && (
                    <div className="mb-1 text-[9px] font-medium uppercase tracking-wide text-amber-200/70">
                      Requested command
                    </div>
                  )}
                  <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-slate-200">
                    <span className="select-none text-emerald-300/70">$ </span>
                    {presentation.command}
                  </pre>
                  <button
                    type="button"
                    aria-label={`Copy ${presentation.action} command`}
                    className="absolute top-2 right-2 rounded p-1 text-slate-400 hover:bg-white/10 hover:text-slate-100"
                    onClick={() => void copyToClipboard(presentation.command ?? "")}
                  >
                    <Copy className="h-3 w-3" aria-hidden="true" />
                  </button>
                </div>
              )}
              {presentation.stdout && (
                <div className="border-b border-white/10 px-3 py-2 last:border-b-0">
                  <div className="mb-1 text-[9px] font-medium uppercase tracking-wide text-slate-500">
                    Output
                  </div>
                  <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-slate-300">
                    {stripAnsiForDisplay(presentation.stdout)}
                  </pre>
                </div>
              )}
              {presentation.stderr && (
                <div className="border-b border-white/10 px-3 py-2 last:border-b-0">
                  <div className="mb-1 text-[9px] font-medium uppercase tracking-wide text-rose-300/70">
                    Stderr
                  </div>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-rose-100/80">
                    {stripAnsiForDisplay(presentation.stderr)}
                  </pre>
                </div>
              )}
              {(presentation.jobId || presentation.hint) && (
                <div className="space-y-1 px-3 py-2 text-[10px] text-slate-400">
                  {presentation.jobId && (
                    <div>
                      <span className="text-slate-500">Job</span>{" "}
                      <code className="font-mono text-sky-200/80">{presentation.jobId}</code>
                    </div>
                  )}
                  {presentation.hint && <p className="whitespace-pre-wrap">{presentation.hint}</p>}
                </div>
              )}
            </section>
          )}
          <div className="overflow-hidden rounded-md border border-border/40 bg-background/40">
            <button
              type="button"
              aria-expanded={rawExpanded}
              aria-controls={rawId}
              className="flex w-full items-center gap-1.5 px-2.5 py-2 text-left text-[10px] text-muted-foreground hover:bg-accent/20 hover:text-foreground"
              onClick={() => setRawExpanded((current) => !current)}
            >
              {rawExpanded ? (
                <ChevronDown className="h-3 w-3" aria-hidden="true" />
              ) : (
                <ChevronRight className="h-3 w-3" aria-hidden="true" />
              )}
              <span>AI Tool raw data</span>
            </button>
            {rawExpanded && (
              <section
                id={rawId}
                aria-label={`${presentation.action} raw tool data`}
                className="space-y-3 border-t border-border/30 px-2.5 py-2"
              >
                <div>
                  <div className="mb-1 flex items-center gap-2 text-[9px] font-medium uppercase tracking-wide text-muted-foreground/70">
                    <span>Input</span>
                    <code className="normal-case text-muted-foreground/50">{tool.name}</code>
                  </div>
                  <JsonView value={tool.args} />
                </div>
                {tool.result !== undefined && (
                  <div>
                    <div className="mb-1 text-[9px] font-medium uppercase tracking-wide text-muted-foreground/70">
                      Result
                    </div>
                    <JsonView value={tool.result} />
                  </div>
                )}
              </section>
            )}
          </div>
        </div>
      )}
    </article>
  );
}

export function ToolActivityGroup({
  tools,
  actorLabel,
}: {
  tools: readonly SubAgentToolCall[];
  actorLabel: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const disclosureId = useId();
  const summary = summarizeToolActivities(tools);
  const status = groupStatus(tools);
  const statusLabel = toolStatusLabel(status);
  const startedAt = tools[0]?.startedAt;

  return (
    <article
      data-testid="tool-activity-group"
      className="mx-4 my-1.5 overflow-hidden rounded-md border border-border/45 bg-background/35"
    >
      <button
        type="button"
        aria-label={`${summary} · ${statusLabel}`}
        aria-expanded={expanded}
        aria-controls={disclosureId}
        className="flex w-full items-center gap-2.5 px-3 py-2.5 text-left hover:bg-accent/20"
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-sky-400/10 text-sky-200">
          <Wrench className="h-3.5 w-3.5" aria-hidden="true" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs font-medium text-foreground/85">{summary}</span>
          <span className="mt-0.5 block truncate text-[10px] text-muted-foreground/70">
            {actorLabel}
            {startedAt ? ` · ${displayTime(startedAt)}` : ""}
            {tools.length > 1 ? ` · ${tools.length} tools` : ""}
          </span>
        </span>
        <span
          role="status"
          className={cn(
            "inline-flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[9px]",
            toolStatusClass(status)
          )}
        >
          <ToolStatusIcon status={status} />
          {statusLabel}
        </span>
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        )}
      </button>
      {expanded && (
        <div id={disclosureId} className="border-t border-border/35 bg-background/25">
          {tools.map((tool) => (
            <ToolActivityRow key={tool.id} tool={tool} />
          ))}
        </div>
      )}
    </article>
  );
}
