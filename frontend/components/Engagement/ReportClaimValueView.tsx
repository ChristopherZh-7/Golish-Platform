import { AlertTriangle, ClipboardCheck, Eye, ShieldCheck, ShieldX } from "lucide-react";
import type { ReportClaimValue } from "@/lib/api/reporting";

function assertNever(value: never): never {
  throw new Error(`unsupported_report_claim_kind:${String(value)}`);
}

function Metric({ label, value }: { label: string; value: number | bigint }) {
  return (
    <div className="rounded border border-border/25 bg-background/30 px-2 py-1.5">
      <dt className="text-[9px] text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-sm font-semibold tabular-nums">{String(value)}</dd>
    </div>
  );
}

function renderReportClaimValue(value: ReportClaimValue) {
  switch (value.kind) {
    case "security_verdict": {
      if (value.authority.contract === "legacy_attempt_v1") {
        return (
          <div className="space-y-2 rounded border border-amber-400/25 bg-amber-400/[0.05] p-2.5">
            <div className="flex flex-wrap items-center gap-2">
              <AlertTriangle className="h-3.5 w-3.5 text-amber-300" />
              <span className="text-xs font-medium">Legacy authority · {value.verdict}</span>
              <span className="rounded border border-amber-300/30 px-1.5 py-0.5 text-[9px] text-amber-200">
                Coverage unavailable
              </span>
            </div>
            <p className="text-[10px] text-muted-foreground">
              Grandfathered terminal Attempt authority; it does not establish Campaign or global coverage.
            </p>
          </div>
        );
      }
      return (
        <div className="flex items-center gap-2 rounded border border-cyan-400/25 bg-cyan-400/[0.04] p-2.5">
          {value.verdict === "verified" ? (
            <ShieldCheck className="h-4 w-4 text-cyan-300" />
          ) : (
            <ShieldX className="h-4 w-4 text-sky-300" />
          )}
          <div>
            <div className="text-xs font-semibold">Revision {value.verdict}</div>
            <div className="text-[10px] text-muted-foreground">
              Revision-level adjudication and terminal-decision authority
            </div>
          </div>
        </div>
      );
    }
    case "coverage": {
      const hasGaps = value.testedDegraded + value.untested + value.blocked > 0;
      return (
        <div className="space-y-2 rounded border border-border/30 p-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <span className={`rounded border px-1.5 py-0.5 text-[9px] ${hasGaps ? "border-amber-400/30 text-amber-200" : "border-cyan-400/30 text-cyan-200"}`}>
              {hasGaps ? "Declared coverage with gaps" : "Declared coverage complete"}
            </span>
            <span className="rounded border border-slate-400/30 px-1.5 py-0.5 text-[9px] text-slate-300">
              Global sufficiency not assessed
            </span>
          </div>
          <dl className="grid grid-cols-2 gap-1.5 sm:grid-cols-5">
            <Metric label="Planned" value={value.planned} />
            <Metric label="Tested complete" value={value.testedComplete} />
            <Metric label="Tested degraded" value={value.testedDegraded} />
            <Metric label="Untested" value={value.untested} />
            <Metric label="Blocked" value={value.blocked} />
          </dl>
          <p className="text-[10px] text-muted-foreground">
            {value.residualIds.length} residual member{value.residualIds.length === 1 ? "" : "s"}.
            Denominator closure does not imply global detection sufficiency.
          </p>
        </div>
      );
    }
    case "observation_audit":
      return (
        <div className="flex items-start gap-2 rounded border border-border/25 bg-muted/10 p-2.5">
          <Eye className="mt-0.5 h-3.5 w-3.5 text-muted-foreground" />
          <div>
            <div className="text-xs font-medium">Observation audit · {value.outcomeCode}</div>
            <div className="text-[10px] text-muted-foreground">{value.provenance}</div>
          </div>
        </div>
      );
    case "method_audit":
      return (
        <div className="flex items-center gap-2 rounded border border-border/25 bg-muted/10 p-2.5">
          <ClipboardCheck className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="text-xs">Method audit · {value.methodCode} · {value.dispositionCode}</span>
        </div>
      );
    case "authorization_audit":
      return (
        <div className="flex items-center gap-2 rounded border border-border/25 bg-muted/10 p-2.5">
          <ClipboardCheck className="h-3.5 w-3.5 text-muted-foreground" />
          <span className="text-xs">Authorization audit · {value.riskTier} · {value.decisionCode}</span>
        </div>
      );
    case "limitation":
      return (
        <details className="rounded border border-amber-400/25 bg-amber-400/[0.04] p-2.5">
          <summary className="cursor-pointer text-xs font-medium text-amber-200">
            Residual limitation · {value.reasonCode}
          </summary>
          <dl className="mt-2 grid gap-2 text-[10px] sm:grid-cols-2">
            <div><dt className="text-muted-foreground">Affected inputs</dt><dd>{value.affectedInputIds.join(", ") || "not recorded"}</dd></div>
            <div><dt className="text-muted-foreground">Owner</dt><dd>{value.ownerCode}</dd></div>
            <div><dt className="text-muted-foreground">Next action</dt><dd>{value.nextActionCode}</dd></div>
            <div><dt className="text-muted-foreground">Residual members</dt><dd>{value.residualIds.length}</dd></div>
          </dl>
        </details>
      );
    default:
      return assertNever(value);
  }
}

export function ReportClaimValueView({ value }: { value: ReportClaimValue }) {
  try {
    return renderReportClaimValue(value);
  } catch {
    return (
      <div role="alert" className="rounded border border-red-500/30 p-2 text-[10px] text-red-300">
        Unsupported typed report claim. Raw claim data was not rendered.
      </div>
    );
  }
}
