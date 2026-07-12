import { Activity, AlertTriangle, CheckCircle2, Database } from "lucide-react";
import type { Target } from "@/lib/pentest/types";
import type { AuditRow, TimelineEntry } from "@/lib/security-analysis";
import { EmptyInline, Section } from "../SurfaceParts";
import { parseWebOrigin, type SurfaceHierarchyVM, type WebOriginVM } from "../surfaceHierarchy";
import { formatTime } from "../surfaceModel";

function stringDetail(detail: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = detail[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

type WhatWebTransportEvidence = {
  attempt: number;
  failureClass: string;
  producerOutcome: "error" | "blocked";
  independentlyConfirmed: boolean;
  subject: string | null;
};

function recordValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function whatWebTransportEvidence(
  detail: Record<string, unknown>
): WhatWebTransportEvidence | null {
  if (detail.kind !== "eas.fingerprint_web_stack" || typeof detail.raw_output !== "string") {
    return null;
  }

  let raw: Record<string, unknown> | null = null;
  try {
    raw = recordValue(JSON.parse(detail.raw_output));
  } catch {
    return null;
  }
  if (!raw) return null;

  const failureClass = stringDetail(raw, ["failure_class"]);
  const producerOutcome = stringDetail(raw, ["producer_outcome"]);
  const attempt = raw.attempt;
  if (
    !failureClass ||
    (producerOutcome !== "error" && producerOutcome !== "blocked") ||
    typeof attempt !== "number" ||
    !Number.isInteger(attempt) ||
    attempt < 1
  ) {
    return null;
  }

  return {
    attempt,
    failureClass,
    producerOutcome,
    independentlyConfirmed: raw.independently_confirmed === true,
    subject: stringDetail(detail, ["subject"]),
  };
}

function originFromCapturePath(
  capturePath: string | null | undefined,
  origins: WebOriginVM[]
): WebOriginVM | null {
  const match = (capturePath ?? "").match(/(?:^|\/)captures\/([^/]+)\/(\d+)\//);
  if (!match) return null;
  const host = match[1].toLowerCase();
  const port = Number(match[2]);
  const matches = origins.filter((origin) => origin.host === host && origin.port === port);
  return matches.length === 1 ? matches[0] : null;
}

function evidenceOrigin(
  detail: Record<string, unknown>,
  hierarchy?: SurfaceHierarchyVM
): { origin: WebOriginVM; confidence: "confirmed" | "inferred" } | null {
  if (!hierarchy) return null;
  const url = stringDetail(detail, [
    "url",
    "endpoint",
    "origin",
    "request_url",
    "requestUrl",
    "subject",
  ]);
  const parsed = parseWebOrigin(url);
  if (parsed) {
    const origin = hierarchy.webOrigins.find((candidate) => candidate.id === parsed.id);
    if (origin) return { origin, confidence: "confirmed" };
  }
  const capturePath = stringDetail(detail, [
    "capturePath",
    "capture_path",
    "filePath",
    "file_path",
  ]);
  const captured = originFromCapturePath(capturePath, hierarchy.webOrigins);
  return captured ? { origin: captured, confidence: "inferred" } : null;
}

function WhatWebTransportStatus({ detail }: { detail: Record<string, unknown> }) {
  const evidence = whatWebTransportEvidence(detail);
  if (!evidence) return null;

  const stopped = evidence.producerOutcome === "blocked" && evidence.attempt >= 3;
  return (
    <div className="mt-1.5 rounded border border-red-500/25 bg-red-500/5 px-2 py-1.5 text-[10px]">
      <div className="flex flex-wrap items-center gap-1.5">
        <AlertTriangle className="h-3.5 w-3.5 text-red-300" />
        <span className="font-medium text-red-200">
          {stopped ? "WhatWeb stopped after 3 network failures" : "WhatWeb network error"}
        </span>
        <span
          className={
            evidence.producerOutcome === "blocked"
              ? "rounded bg-amber-500/15 px-1.5 py-0.5 text-amber-300"
              : "rounded bg-red-500/15 px-1.5 py-0.5 text-red-300"
          }
        >
          {evidence.producerOutcome}
        </span>
        <span className="rounded bg-muted/25 px-1.5 py-0.5 text-muted-foreground">
          Attempt {evidence.attempt}/3
        </span>
        <span className="rounded bg-muted/25 px-1.5 py-0.5 font-mono text-muted-foreground">
          {evidence.failureClass}
        </span>
      </div>
      {stopped && evidence.independentlyConfirmed && evidence.subject && (
        <p className="mt-1 font-mono text-[9px] text-amber-200">
          Excluded exact origin from Enumeration: {evidence.subject}
        </p>
      )}
    </div>
  );
}

function OriginRelationBadge({
  relation,
}: {
  relation: { origin: WebOriginVM; confidence: "confirmed" | "inferred" } | null;
}) {
  if (!relation) {
    return (
      <span className="rounded bg-muted/20 px-1.5 py-0.5 text-[9px] text-muted-foreground">
        Target-level
      </span>
    );
  }
  return (
    <span className="max-w-[220px] truncate rounded bg-accent/10 px-1.5 py-0.5 font-mono text-[9px] text-accent">
      {relation.origin.origin} · {relation.confidence}
    </span>
  );
}

export function EvidenceTab({
  target,
  timeline,
  logs,
  loading,
  hierarchy,
}: {
  target: Target;
  timeline: TimelineEntry[];
  logs: AuditRow[];
  loading: boolean;
  hierarchy?: SurfaceHierarchyVM;
}) {
  return (
    <div className="space-y-2.5">
      <Section title="Source Evidence" subtitle="why this target exists">
        <div className="rounded border border-border/20 bg-muted/5 px-2 py-1.5 text-[11px]">
          <div className="flex items-center gap-2">
            <CheckCircle2 className="w-3.5 h-3.5 text-green-300" />
            <span className="text-foreground">{target.source || "manual"}</span>
            <span className="text-muted-foreground">· scope={target.scope}</span>
          </div>
        </div>
      </Section>
      <Section title="Operation Logs" subtitle={`${logs.length} latest event(s)`}>
        {timeline.length === 0 && logs.length === 0 ? (
          <EmptyInline loading={loading} label="No operation logs for this target yet" />
        ) : (
          <div className="space-y-1">
            {timeline.slice(0, 30).map((entry, index) => (
              <div
                key={`${entry.source}:${entry.event}:${entry.createdAt}:${index}`}
                className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
              >
                <div className="flex items-center gap-2 text-[11px]">
                  <Activity className="w-3.5 h-3.5 text-accent/70" />
                  <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                    {entry.source}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-foreground/80">
                    {entry.details || entry.event}
                  </span>
                  {entry.toolName && (
                    <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[9px] text-accent">
                      {entry.toolName}
                    </span>
                  )}
                  <OriginRelationBadge relation={evidenceOrigin(entry.detail, hierarchy)} />
                  <span className="text-[9px] text-muted-foreground">
                    {formatTime(entry.createdAt)}
                  </span>
                </div>
                <WhatWebTransportStatus detail={entry.detail} />
              </div>
            ))}
            {timeline.length === 0 &&
              logs.map((log) => (
                <div
                  key={log.id}
                  className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
                >
                  <div className="flex items-center gap-2 text-[11px]">
                    <Database className="w-3.5 h-3.5 text-accent/70" />
                    <span className="min-w-0 flex-1 truncate text-foreground/80">{log.action}</span>
                    {log.toolName && (
                      <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[9px] text-accent">
                        {log.toolName}
                      </span>
                    )}
                    <OriginRelationBadge relation={evidenceOrigin(log.detail, hierarchy)} />
                    <span className="text-[9px] text-muted-foreground">
                      {new Date(log.createdAt).toLocaleTimeString()}
                    </span>
                  </div>
                  <WhatWebTransportStatus detail={log.detail} />
                </div>
              ))}
          </div>
        )}
      </Section>
    </div>
  );
}
