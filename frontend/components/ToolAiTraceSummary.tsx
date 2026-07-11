import { ScanSearch } from "lucide-react";
import { cn } from "@/lib/utils";

type JsonRecord = Record<string, unknown>;

interface AiTraceSection {
  title: string;
  icon: "findings";
  chips: string[];
  reasons: string[];
  nextStep?: string;
  fileRows: Array<{
    source: string;
    meta: string[];
    lines: string[];
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

function asBool(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function recordArray(value: unknown, limit = 6): JsonRecord[] {
  return Array.isArray(value)
    ? value.filter((v): v is JsonRecord => isRecord(v)).slice(0, limit)
    : [];
}

function rawArrayLength(value: unknown): number | null {
  return Array.isArray(value) ? value.length : null;
}

function countChip(label: string, value: unknown): string | null {
  const n = asNumber(value);
  return n === null ? null : `${label} ${n}`;
}

function stringChip(label: string, value: unknown): string | null {
  const text = asString(value);
  return text ? `${label} ${text}` : null;
}

function compactPath(value: unknown): string | null {
  const text = asString(value);
  if (!text) return null;
  if (text.length <= 48) return text;
  return `...${text.slice(-45)}`;
}

function firstNumber(...values: unknown[]): number | null {
  for (const value of values) {
    const n = asNumber(value);
    if (n !== null) return n;
  }
  return null;
}

function pushUnique(target: string[], values: Array<string | null>) {
  for (const value of values) {
    if (value && !target.includes(value)) target.push(value);
  }
}

function pushBrowserFindings(
  result: JsonRecord,
  chips: string[],
  reasons: string[],
  fileRows: AiTraceSection["fileRows"]
) {
  const persistable = recordArray(result.persistable_api_requests, 5);
  const apiRequests = asNumber(result.api_requests_total);
  const persisted = asNumber(result.persisted_api_rows);
  const duplicates = asNumber(result.duplicate_api_rows);
  const scriptsSaved = asNumber(result.scripts_saved);
  const recipeRounds = asNumber(result.ai_recipe_rounds);

  pushUnique(chips, [
    countChip("runtime API", apiRequests),
    countChip("saved JS", scriptsSaved),
    countChip("runtime landed", persisted),
    recipeRounds && recipeRounds > 0 ? `AI recipe ${recipeRounds}` : null,
  ]);

  if (apiRequests !== null || persisted !== null || persistable.length > 0) {
    reasons.push(
      `Browser crawl saw ${apiRequests ?? persistable.length} API request(s); ${
        persisted ?? 0
      } landed in api_endpoints${duplicates ? `, ${duplicates} already existed` : ""}.`
    );
  }
  if (scriptsSaved !== null) {
    reasons.push(`Saved ${scriptsSaved} JavaScript file(s) for static extraction.`);
  }
  if (recipeRounds && recipeRounds > 0) {
    reasons.push(`AI recipe ran ${recipeRounds} round(s) to look for missed same-origin work.`);
  }

  for (const item of persistable) {
    const method = asString(item.method) ?? "API";
    const path = asString(item.path) ?? asString(item.url) ?? "runtime request";
    const status = firstNumber(item.status, item.status_code);
    fileRows.push({
      source: `${method.toUpperCase()} ${path}`,
      meta: [status !== null ? `status ${status}` : null, stringChip("source", item.source)].filter(
        (v): v is string => !!v
      ),
      lines: [],
    });
  }
}

function pushRouteFindings(
  result: JsonRecord,
  chips: string[],
  reasons: string[],
  fileRows: AiTraceSection["fileRows"]
) {
  const matches = recordArray(result.matches, 8);
  const requests = asNumber(result.requests_sent);
  const persisted = asNumber(result.persisted_directory_entries);
  const outcome = asString(result.outcome);
  const queueCompleted = asBool(result.queue_completed);
  const errors = firstNumber(result.errors_total, rawArrayLength(result.errors));
  const rejected = firstNumber(result.rejected_total, rawArrayLength(result.rejected_candidates));
  const foundPaths = firstNumber(persisted, result.matches_total, matches.length) ?? 0;

  pushUnique(chips, [
    `paths ${foundPaths}`,
    countChip("checked", requests),
    outcome ? `outcome ${outcome}` : null,
    queueCompleted === true
      ? "queue complete"
      : queueCompleted === false
        ? "queue incomplete"
        : null,
  ]);

  if (foundPaths > 0) {
    reasons.push(`Route probe found ${foundPaths} verified path(s).`);
  } else if (requests !== null) {
    reasons.push(`Route probe checked ${requests} request(s) and found no verified path.`);
  }
  if (rejected && rejected > 0) {
    reasons.push(`${rejected} soft/uniform candidate(s) were rejected and hidden from findings.`);
  }
  if (errors && errors > 0) {
    reasons.push(`${errors} request error(s) occurred; raw output keeps the details.`);
  }

  for (const match of matches.slice(0, 6)) {
    const url = asString(match.url) ?? asString(match.path) ?? "matched path";
    const status = firstNumber(match.status, match.status_code);
    fileRows.push({
      source: url,
      meta: [
        status !== null ? `status ${status}` : null,
        stringChip("class", match.candidate_class),
        stringChip("verdict", match.verdict),
      ].filter((v): v is string => !!v),
      lines: [],
    });
  }
}

function endpointLabel(endpoint: JsonRecord): string {
  const method = asString(endpoint.method) ?? "API";
  const path =
    asString(endpoint.path) ?? asString(endpoint.url) ?? asString(endpoint.endpoint) ?? "endpoint";
  return `${method.toUpperCase()} ${path}`;
}

function pushJsExtractFindings(
  result: JsonRecord,
  chips: string[],
  reasons: string[],
  fileRows: AiTraceSection["fileRows"]
) {
  const endpoints = recordArray(result.endpoints, 8);
  const endpointTotal = firstNumber(result.endpoints_total, endpoints.length);
  const endpointUnique = asNumber(result.endpoints_unique);
  const filesScanned = asNumber(result.files_scanned);
  const persisted = asNumber(result.persisted_endpoint_rows);
  const duplicates = asNumber(result.duplicate_endpoint_rows);
  const secrets = asNumber(result.secrets_total);
  const configs = asNumber(result.configs_total);
  const paramEndpoints = asNumber(result.param_endpoints);
  const paramHints = asNumber(result.param_hints_count);
  const summary = isRecord(result.summary) ? result.summary : {};
  const aiUsed = result.ai_used === true || summary.ai_used === true;
  const aiAdded = firstNumber(result.ai_endpoints_added, summary.ai_endpoints_added);
  const haeCandidates = firstNumber(
    result.hae_route_candidates_total,
    summary.hae_route_candidates
  );
  const haePromoted = firstNumber(result.hae_ai_promoted, summary.hae_ai_promoted);
  const jsapiOutcome = asString(result.jsapi_outcome);
  const paramOutcome = asString(result.param_outcome);

  pushUnique(chips, [
    countChip("JS API", endpointTotal),
    countChip("unique", endpointUnique),
    countChip("JS landed", persisted),
    paramEndpoints && paramEndpoints > 0
      ? `params ${paramEndpoints}`
      : paramHints && paramHints > 0
        ? `param hints ${paramHints}`
        : null,
    haeCandidates && haeCandidates > 0 ? `HAE candidates ${haeCandidates}` : null,
    haePromoted && haePromoted > 0 ? `HAE promoted ${haePromoted}` : null,
    secrets && secrets > 0 ? `secrets ${secrets}` : null,
    aiUsed ? `AI +${aiAdded ?? 0}` : null,
  ]);

  if (endpointTotal !== null) {
    reasons.push(
      `Static JS extraction found ${endpointTotal} API endpoint(s)${
        filesScanned !== null ? ` across ${filesScanned} JS file(s)` : ""
      }; ${persisted ?? 0} landed${duplicates ? ` and ${duplicates} already existed` : ""}.`
    );
  }
  if (paramEndpoints && paramEndpoints > 0) {
    reasons.push(`Parameter evidence landed for ${paramEndpoints} endpoint(s).`);
  } else if (paramHints && paramHints > 0) {
    reasons.push(`${paramHints} parameter hint(s) were reviewed for DB merge.`);
  }
  if (secrets && secrets > 0) {
    reasons.push(`${secrets} possible secret(s) need review.`);
  }
  if (configs && configs > 0) {
    reasons.push(`${configs} config value(s) were detected.`);
  }
  if (haeCandidates && haeCandidates > 0) {
    reasons.push(
      `HaE-style regex produced ${haeCandidates} route/path candidate(s); ${
        haePromoted ?? 0
      } were AI-promoted into the API set.`
    );
  }
  if (aiUsed) {
    reasons.push(`AI review ran and added ${aiAdded ?? 0} endpoint candidate(s).`);
  }
  if (jsapiOutcome) {
    reasons.push(`JSAPI outcome: ${jsapiOutcome}.`);
  }
  if (paramOutcome) {
    reasons.push(`PARAM outcome: ${paramOutcome}.`);
  }

  for (const endpoint of endpoints.slice(0, 6)) {
    fileRows.push({
      source: endpointLabel(endpoint),
      meta: [
        stringChip("source", endpoint.source),
        countChip("line", endpoint.line),
        stringChip("file", compactPath(endpoint.source_file)),
      ].filter((v): v is string => !!v),
      lines: [],
    });
  }
}

function buildKeyFindingsSection(result: JsonRecord): AiTraceSection | null {
  const chips: string[] = [];
  const reasons: string[] = [];
  const fileRows: AiTraceSection["fileRows"] = [];

  const isBrowserResult =
    "scripts_saved" in result || "api_requests_total" in result || "crawl_mode" in result;
  const isRouteProbe =
    "seed_paths" in result ||
    "wordlist" in result ||
    "queue_completed" in result ||
    "candidate_requests_sent" in result;
  const isJsExtract =
    "endpoints_total" in result ||
    "persisted_endpoint_rows" in result ||
    "jsapi_outcome" in result ||
    "ai_analysis" in result;

  if (isBrowserResult) pushBrowserFindings(result, chips, reasons, fileRows);
  if (isRouteProbe) pushRouteFindings(result, chips, reasons, fileRows);
  if (isJsExtract) pushJsExtractFindings(result, chips, reasons, fileRows);

  if (chips.length === 0 && reasons.length === 0 && fileRows.length === 0) return null;

  return {
    title: "Key Findings",
    icon: "findings",
    chips: chips.slice(0, 8),
    reasons: reasons.slice(0, 6),
    nextStep: fileRows.length === 0 ? "No concrete findings returned in this result." : undefined,
    fileRows,
  };
}

export function extractToolAiTraceSections(value: unknown): AiTraceSection[] {
  if (!isRecord(value)) return [];
  return [buildKeyFindingsSection(value)].filter((section): section is AiTraceSection => !!section);
}

function TraceIcon({ kind: _kind }: { kind: AiTraceSection["icon"] }) {
  return <ScanSearch className="h-3.5 w-3.5 text-foreground/70" />;
}

function ToolAiTraceSectionView({
  section,
  dense = false,
}: {
  section: AiTraceSection;
  dense?: boolean;
}) {
  return (
    <div className="rounded-md border border-border/60 bg-background/35">
      <div className="flex min-w-0 items-center gap-2 border-b border-border/50 px-2.5 py-1.5">
        <TraceIcon kind={section.icon} />
        <span className="text-[11px] font-semibold text-foreground/85">{section.title}</span>
        <div className="ml-auto flex min-w-0 flex-wrap justify-end gap-1">
          {section.chips.slice(0, dense ? 4 : 8).map((chip) => (
            <span
              key={chip}
              className="rounded border border-border/60 bg-muted/20 px-1.5 py-0.5 text-[9px] text-muted-foreground/85"
            >
              {chip}
            </span>
          ))}
        </div>
      </div>

      <div className={cn("space-y-2 px-2.5 py-2", dense && "space-y-1.5")}>
        {section.reasons.length > 0 && (
          <div className="space-y-1">
            {section.reasons.map((reason) => (
              <div key={reason} className="text-[10px] leading-relaxed text-foreground/75">
                {reason}
              </div>
            ))}
          </div>
        )}

        {section.nextStep && (
          <div className="rounded bg-muted/20 px-2 py-1 text-[10px] leading-relaxed text-muted-foreground">
            {section.nextStep}
          </div>
        )}

        {section.fileRows.length > 0 && (
          <div className="space-y-1">
            {section.fileRows.slice(0, dense ? 3 : 6).map((row) => (
              <div
                key={`${row.source}:${row.meta.join("|")}`}
                className="rounded bg-muted/15 px-2 py-1.5"
              >
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
              </div>
            ))}
          </div>
        )}
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
