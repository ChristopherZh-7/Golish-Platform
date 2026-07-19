import { AlertTriangle, CheckCircle2, CircleHelp, MinusCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WhatWebAssessment } from "./whatWebAssessment";

type DisplayState = WhatWebAssessment["state"] | "not_assessed";

const LABELS: Record<DisplayState, string> = {
  not_assessed: "Not assessed",
  fingerprint_found: "Fingerprint found",
  checked_empty: "Checked empty",
  retry_pending: "Retry pending",
  producer_blocked: "WhatWeb stopped",
  state_error: "Evidence error",
};

const STYLES: Record<DisplayState, string> = {
  not_assessed: "border-border/30 bg-muted/15 text-muted-foreground",
  fingerprint_found: "border-green-500/25 bg-green-500/10 text-green-300",
  checked_empty: "border-sky-500/25 bg-sky-500/10 text-sky-300",
  retry_pending: "border-red-500/25 bg-red-500/10 text-red-300",
  producer_blocked: "border-amber-500/25 bg-amber-500/10 text-amber-300",
  state_error: "border-red-500/30 bg-red-500/10 text-red-300",
};

function StateIcon({ state }: { state: DisplayState }) {
  if (state === "fingerprint_found") return <CheckCircle2 className="h-3 w-3" />;
  if (state === "checked_empty") return <MinusCircle className="h-3 w-3" />;
  if (state === "retry_pending" || state === "producer_blocked" || state === "state_error") {
    return <AlertTriangle className="h-3 w-3" />;
  }
  return <CircleHelp className="h-3 w-3" />;
}

function titleFor(assessment: WhatWebAssessment | null | undefined): string {
  if (!assessment) {
    return "No authoritative WhatWeb evidence is recorded for this exact Web Origin.";
  }
  if (assessment.state === "fingerprint_found") {
    return `${assessment.origin}: WhatWeb produced fingerprint evidence.`;
  }
  if (assessment.state === "checked_empty") {
    return `${assessment.origin}: WhatWeb completed and recorded an explicit checked-empty result.`;
  }
  if (assessment.state === "retry_pending") {
    return `${assessment.origin}: ${assessment.failureClass}; retry ${assessment.attempt}/3 is pending. The Gate remains open.`;
  }
  if (assessment.state === "producer_blocked") {
    return `${assessment.origin}: ${assessment.failureClass}; WhatWeb stopped after 3 attempts. The Target and open-port facts remain. Enumeration decides exact-origin eligibility from server-side operation state.`;
  }
  return `${assessment.origin}: the evidence row is inconsistent or malformed and is not treated as found, checked empty, or excluded.`;
}

export function WhatWebAssessmentBadge({
  assessment,
}: {
  assessment: WhatWebAssessment | null | undefined;
}) {
  const state: DisplayState = assessment?.state ?? "not_assessed";
  const suffix = assessment?.state === "retry_pending" ? `${assessment.attempt}/3` : null;
  return (
    <span
      className={cn(
        "inline-flex max-w-full items-center gap-1 rounded border px-1.5 py-0.5 text-[9px] font-medium",
        STYLES[state]
      )}
      title={titleFor(assessment)}
    >
      <StateIcon state={state} />
      <span className="truncate">{LABELS[state]}</span>
      {suffix && <span className="font-mono opacity-80">{suffix}</span>}
    </span>
  );
}
