import { FileCode2, ScanSearch } from "lucide-react";
import { JsonView } from "@/components/JsonView/JsonView";
import { cn } from "@/lib/utils";

type JsonRecord = Record<string, unknown>;

interface AiTraceSection {
  title: string;
  icon: "assist" | "analysis";
  chips: string[];
  reasons: string[];
  nextStep?: string;
  fileRows: Array<{
    source: string;
    meta: string[];
    lines: string[];
  }>;
  samples: Array<{
    label: string;
    value: unknown;
  }>;
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function recordArray(value: unknown, limit = 6): JsonRecord[] {
  return Array.isArray(value)
    ? value.filter((v): v is JsonRecord => isRecord(v)).slice(0, limit)
    : [];
}

function countChip(label: string, value: unknown): string | null {
  const n = asNumber(value);
  return n === null ? null : `${label} ${n}`;
}

function lineHintText(value: unknown): string[] {
  return recordArray(value, 3)
    .map((hint) => {
      const start = asNumber(hint.line_start);
      const end = asNumber(hint.line_end);
      if (start === null) return null;
      return end !== null && end !== start ? `L${start}-${end}` : `L${start}`;
    })
    .filter((v): v is string => !!v);
}

function buildAnalysisSection(result: JsonRecord): AiTraceSection | null {
  const analysis = result.ai_analysis;
  if (!isRecord(analysis)) return null;

  const candidateFiles = recordArray(analysis.candidate_files, 6);
  const chips = [
    countChip("files", result.files_scanned ?? candidateFiles.length),
    countChip("endpoints", result.endpoints_total),
    countChip("secrets", result.secrets_total),
    countChip("rules", result.rule_matches_total),
    countChip("configs", result.configs_total),
    asString(analysis.api_base_path) ? `base ${asString(analysis.api_base_path)}` : null,
  ].filter((v): v is string => !!v);

  const fileRows = candidateFiles.map((file) => {
    const source = asString(file.source_file) ?? "source file";
    const meta = [
      countChip("endpoints", file.endpoints),
      countChip("secrets", file.secrets),
      countChip("configs", file.configs),
      countChip("rules", file.rule_matches),
    ].filter((v): v is string => !!v);
    return {
      source,
      meta,
      lines: lineHintText(file.line_hints),
    };
  });

  const samples: AiTraceSection["samples"] = [];
  for (const [label, key] of [
    ["Suggested ranges", "suggested_read_file_ranges"],
    ["Endpoint notes", "endpoint_notes"],
    ["Secret triage", "secret_triage"],
  ] as const) {
    const value = analysis[key];
    if (Array.isArray(value) && value.length > 0) {
      samples.push({ label, value: value.slice(0, 5) });
    }
  }

  return {
    title: "Static Analysis Hints",
    icon: "analysis",
    chips,
    reasons: [],
    nextStep: asString(analysis.next_step) ?? asString(analysis.summary) ?? undefined,
    fileRows,
    samples,
  };
}

export function extractToolAiTraceSections(value: unknown): AiTraceSection[] {
  if (!isRecord(value)) return [];
  // "Collector Hints" (ai_assist) was removed for a simpler tool detail view —
  // the live run window + result fields carry that context. Only the static
  // analysis handoff is surfaced now.
  return [buildAnalysisSection(value)].filter((section): section is AiTraceSection => !!section);
}

function TraceIcon({ kind }: { kind: AiTraceSection["icon"] }) {
  const Icon = kind === "analysis" ? FileCode2 : ScanSearch;
  return <Icon className="h-3.5 w-3.5 text-[var(--ansi-magenta)]/80" />;
}

function ToolAiTraceSectionView({
  section,
  dense = false,
}: {
  section: AiTraceSection;
  dense?: boolean;
}) {
  return (
    <div className="rounded-md border border-[var(--ansi-magenta)]/20 bg-[var(--ansi-magenta)]/[0.055]">
      <div className="flex min-w-0 items-center gap-2 border-b border-[var(--ansi-magenta)]/15 px-2.5 py-1.5">
        <TraceIcon kind={section.icon} />
        <span className="text-[11px] font-semibold text-foreground/85">{section.title}</span>
        <div className="ml-auto flex min-w-0 flex-wrap justify-end gap-1">
          {section.chips.slice(0, dense ? 4 : 8).map((chip) => (
            <span
              key={chip}
              className="rounded border border-[var(--ansi-magenta)]/15 bg-background/35 px-1.5 py-0.5 text-[9px] text-muted-foreground/85"
            >
              {chip}
            </span>
          ))}
        </div>
      </div>

      <div className={cn("space-y-2 px-2.5 py-2", dense && "space-y-1.5")}>
        {section.reasons.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {section.reasons.map((reason) => (
              <span
                key={reason}
                className="rounded bg-background/35 px-1.5 py-0.5 text-[10px] text-muted-foreground"
              >
                {reason}
              </span>
            ))}
          </div>
        )}

        {section.nextStep && (
          <div className="rounded bg-background/35 px-2 py-1 text-[10px] leading-relaxed text-foreground/75">
            {section.nextStep}
          </div>
        )}

        {section.fileRows.length > 0 && (
          <div className="space-y-1">
            {section.fileRows.slice(0, dense ? 3 : 6).map((row) => (
              <div key={row.source} className="rounded bg-background/30 px-2 py-1.5">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 truncate font-mono text-[10px] text-foreground/80">
                    {row.source}
                  </span>
                  {row.meta.length > 0 && (
                    <span className="shrink-0 text-[9px] text-muted-foreground/65">
                      {row.meta.join(" · ")}
                    </span>
                  )}
                </div>
                {row.lines.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {row.lines.map((line) => (
                      <span
                        key={line}
                        className="rounded bg-muted/25 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground/80"
                      >
                        {line}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}

        {!dense &&
          section.samples.map((sample) => (
            <details key={sample.label} className="rounded bg-background/30 px-2 py-1">
              <summary className="cursor-pointer text-[10px] text-muted-foreground/85">
                {sample.label}
              </summary>
              <div className="mt-1 max-h-40 overflow-auto">
                <JsonView value={sample.value} />
              </div>
            </details>
          ))}
      </div>
    </div>
  );
}

export function ToolAiTraceSummary({ value, dense = false }: { value: unknown; dense?: boolean }) {
  const sections = extractToolAiTraceSections(value);
  if (sections.length === 0) return null;

  return (
    <div className="space-y-2">
      {sections.map((section) => (
        <ToolAiTraceSectionView key={section.title} section={section} dense={dense} />
      ))}
    </div>
  );
}
