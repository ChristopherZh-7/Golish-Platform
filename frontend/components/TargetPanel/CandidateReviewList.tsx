/**
 * `CandidateReviewList` — the workspace "Candidates" tab.
 *
 * Extracted verbatim from `TargetGroupedView.tsx`'s `renderWorkspacePanel`. Shows
 * discovered organization/target candidates with approve / reject / promote
 * actions and an expandable evidence detail panel per candidate.
 */

import type { Dispatch, SetStateAction } from "react";
import type { OrganizationCandidate } from "@/lib/api/organizations";
import { getEvidenceRawRows } from "@/lib/target-panel/asset-intel";
import { translateWithFallback } from "@/lib/target-panel/org-fields";
import { cn } from "@/lib/utils";

interface CandidateReviewListProps {
  t: (key: string) => string;
  candidateCounts: { organizations: number; targets: number };
  organizationCandidates: OrganizationCandidate[];
  targetCandidates: OrganizationCandidate[];
  candidateUpdatingId: string | null;
  candidatePromotingId: string | null;
  expandedCandidateIds: Set<string>;
  setExpandedCandidateIds: Dispatch<SetStateAction<Set<string>>>;
  handlePromoteCandidate: (candidate: OrganizationCandidate) => void;
  handleCandidateStatus: (
    candidate: OrganizationCandidate,
    status: "approved" | "rejected"
  ) => void;
}

export function CandidateReviewList({
  t,
  candidateCounts,
  organizationCandidates,
  targetCandidates,
  candidateUpdatingId,
  candidatePromotingId,
  expandedCandidateIds,
  setExpandedCandidateIds,
  handlePromoteCandidate,
  handleCandidateStatus,
}: CandidateReviewListProps) {
  return (
    <section
      className={cn(
        "rounded border p-3",
        candidateCounts.organizations + candidateCounts.targets > 0
          ? "border-amber-500/30 bg-amber-500/5"
          : "border-border/40 bg-muted/5"
      )}
    >
      <h4 className="text-xs font-medium text-foreground">
        {translateWithFallback(t, "targetWorkspace.candidates.title", "Discovery candidates")}
      </h4>
      <p className="text-[10px] text-muted-foreground/70 mt-1">
        {translateWithFallback(
          t,
          "targetWorkspace.candidates.description",
          "Review discovered subsidiaries and assets before they become in-scope targets."
        )}
      </p>
      <div className="mt-3 grid grid-cols-2 gap-1.5">
        <div className="rounded border border-dashed border-border/40 p-2.5">
          <p className="text-[10px] text-muted-foreground/60">
            {translateWithFallback(
              t,
              "targetWorkspace.candidates.organizations",
              "Organization candidates"
            )}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {candidateCounts.organizations > 0
              ? translateWithFallback(
                  t,
                  "targetWorkspace.candidates.waiting",
                  "{{count}} candidate(s) waiting for review"
                ).replace("{{count}}", String(candidateCounts.organizations))
              : translateWithFallback(t, "targetWorkspace.candidates.empty", "No candidates yet.")}
          </p>
        </div>
        <div className="rounded border border-dashed border-border/40 p-2.5">
          <p className="text-[10px] text-muted-foreground/60">
            {translateWithFallback(t, "targetWorkspace.candidates.targets", "Target candidates")}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {candidateCounts.targets > 0
              ? translateWithFallback(
                  t,
                  "targetWorkspace.candidates.waiting",
                  "{{count}} candidate(s) waiting for review"
                ).replace("{{count}}", String(candidateCounts.targets))
              : translateWithFallback(t, "targetWorkspace.candidates.empty", "No candidates yet.")}
          </p>
        </div>
      </div>
      {organizationCandidates.length + targetCandidates.length > 0 && (
        <div className="mt-3 space-y-2">
          {[
            ["Organizations", organizationCandidates],
            ["Targets", targetCandidates],
          ].map(([title, items]) => (
            <div key={title as string} className="space-y-1">
              <p className="text-[10px] font-medium text-muted-foreground">{title as string}</p>
              {(items as OrganizationCandidate[]).slice(0, 6).map((candidate) => {
                const candidateKey =
                  candidate.id ?? `${candidate.kind}:${candidate.source}:${candidate.value}`;
                const updating = candidateUpdatingId === candidateKey;
                const promoting = candidatePromotingId === candidateKey;
                const evidenceRows = getEvidenceRawRows(candidate.evidence);
                const isExpanded = expandedCandidateIds.has(candidateKey);
                return (
                  <div
                    key={candidateKey}
                    className="rounded border border-border/30 bg-background/35 p-2"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="truncate text-[11px] text-foreground">
                          {candidate.label || candidate.value}
                        </p>
                        <p className="mt-0.5 truncate text-[10px] text-muted-foreground">
                          {candidate.value}
                        </p>
                      </div>
                      <span
                        className={cn(
                          "rounded px-1.5 py-0.5 text-[9px]",
                          candidate.status === "approved"
                            ? "bg-green-500/10 text-green-300"
                            : candidate.status === "rejected"
                              ? "bg-red-500/10 text-red-300"
                              : "bg-amber-500/10 text-amber-300"
                        )}
                      >
                        {candidate.status || "needs_review"}
                      </span>
                    </div>
                    <div className="mt-2 flex items-center justify-between gap-2">
                      <span className="text-[10px] text-muted-foreground">
                        {candidate.source || "provider"} ·{" "}
                        {Math.round((candidate.confidence ?? 0) * 100)}%
                      </span>
                      <div className="flex items-center gap-1">
                        {evidenceRows.length > 0 && (
                          <button
                            type="button"
                            aria-expanded={isExpanded}
                            className={cn(
                              "rounded border border-border/40 px-1.5 py-0.5 text-[9px] text-muted-foreground hover:bg-muted/30",
                              isExpanded && "border-accent/40 text-accent"
                            )}
                            onClick={() => {
                              setExpandedCandidateIds((prev) => {
                                const next = new Set(prev);
                                if (next.has(candidateKey)) next.delete(candidateKey);
                                else next.add(candidateKey);
                                return next;
                              });
                            }}
                          >
                            {isExpanded ? "Hide" : "Details"}
                          </button>
                        )}
                        {candidate.status === "approved" && (
                          <button
                            type="button"
                            className="rounded border border-blue-500/25 px-1.5 py-0.5 text-[9px] text-blue-300 hover:bg-blue-500/10 disabled:opacity-50"
                            disabled={promoting}
                            onClick={() => void handlePromoteCandidate(candidate)}
                          >
                            {promoting ? "Promoting" : "Promote"}
                          </button>
                        )}
                        <button
                          type="button"
                          className="rounded border border-green-500/25 px-1.5 py-0.5 text-[9px] text-green-300 hover:bg-green-500/10 disabled:opacity-50"
                          disabled={updating || candidate.status === "approved"}
                          onClick={() => void handleCandidateStatus(candidate, "approved")}
                        >
                          Approve
                        </button>
                        <button
                          type="button"
                          className="rounded border border-red-500/25 px-1.5 py-0.5 text-[9px] text-red-300 hover:bg-red-500/10 disabled:opacity-50"
                          disabled={updating || candidate.status === "rejected"}
                          onClick={() => void handleCandidateStatus(candidate, "rejected")}
                        >
                          Reject
                        </button>
                      </div>
                    </div>
                    {isExpanded && evidenceRows.length > 0 && (
                      <dl className="mt-2 grid grid-cols-[120px_1fr] gap-x-2 gap-y-1 rounded border border-dashed border-border/40 bg-muted/10 p-2 text-[10px]">
                        {evidenceRows.map((row) => (
                          <div key={`${candidateKey}:${row.field}`} className="contents">
                            <dt className="text-muted-foreground">{row.label}</dt>
                            <dd className="break-all text-foreground/85 font-mono">{row.value}</dd>
                          </div>
                        ))}
                      </dl>
                    )}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
