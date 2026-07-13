import { KeyRound, List, ListChecks, MessageSquare, Pencil, ShieldQuestion } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Markdown } from "@/components/Markdown";
import {
  listOrganizationCandidates,
  type UnitReviewDecisionRow,
  type UnitReviewSubmission,
} from "@/lib/api/organizations";
import { cn } from "@/lib/utils";
import {
  candidatesToUnitRows,
  parseBulkRows,
  type ScopeReviewHandle,
  type ScopeReviewKind,
  ScopeReviewTable,
} from "./ScopeReviewTable";

export const ASK_HUMAN_INPUT_TYPES = [
  "credentials",
  "choice",
  "freetext",
  "confirmation",
  "scope_review",
  "unit_review",
] as const;
export type AskHumanInputType = (typeof ASK_HUMAN_INPUT_TYPES)[number];

export interface AskHumanState {
  requestId: string;
  sessionId: string;
  question: string;
  inputType: AskHumanInputType;
  options: string[];
  context: string;
}

/**
 * Resolve the raw `input_type` a model put on an `ask_human` call into the
 * effective UI mode. Models frequently supply `options` but leave `input_type`
 * at the default "freetext" (or send an unrecognised value); when options are
 * present we surface a selectable "choice" picker instead of silently dropping
 * them into a dead-end text box.
 */
export function resolveAskHumanInputType(
  rawInputType: string | null | undefined,
  options: readonly string[]
): AskHumanInputType {
  const known = (ASK_HUMAN_INPUT_TYPES as readonly string[]).includes(rawInputType ?? "")
    ? (rawInputType as AskHumanInputType)
    : "freetext";
  if (options.length > 0 && known === "freetext") return "choice";
  return known;
}

const INPUT_TYPE_ICONS: Record<string, typeof KeyRound> = {
  credentials: KeyRound,
  choice: List,
  freetext: MessageSquare,
  confirmation: ShieldQuestion,
  scope_review: ListChecks,
  unit_review: ListChecks,
};

/** Where a review table's initial rows come from. */
export type ReviewSource =
  | { kind: "org"; organizationId: string }
  | { kind: "rows"; rows: unknown }
  | { kind: "bulk"; text: string };

/**
 * Decide where the review table's initial rows come from. Priority:
 * 1. context carries an `organization_id` → fetch that org's discovered
 *    candidates from the DB (robust: the agent only had to copy a small id, not
 *    a fragile 10-18 item array that textual-tool-call models mangle);
 * 2. context is (or stringifies to) an array / `{items|candidates|units|
 *    organizations: [...]}` → use it directly (back-compat with the old array
 *    contract and scope_review);
 * 3. otherwise treat context as bulk text (one entry per line).
 *
 * Tolerant of double-encoded JSON (a JSON string whose value is itself JSON),
 * which is how some models escape structured arguments.
 */
export function parseReviewContext(context: string): ReviewSource {
  const raw = context.trim();
  if (!raw) return { kind: "rows", rows: [] };
  let v: unknown;
  try {
    v = JSON.parse(raw);
    if (typeof v === "string") v = JSON.parse(v);
  } catch {
    return { kind: "bulk", text: raw };
  }
  if (v && typeof v === "object" && !Array.isArray(v)) {
    const obj = v as Record<string, unknown>;
    const orgId = obj.organization_id ?? obj.organizationId;
    if (typeof orgId === "string" && orgId.trim()) {
      return { kind: "org", organizationId: orgId.trim() };
    }
    const arr = obj.items ?? obj.candidates ?? obj.units ?? obj.organizations;
    if (Array.isArray(arr)) return { kind: "rows", rows: arr };
  }
  if (Array.isArray(v)) return { kind: "rows", rows: v };
  return { kind: "bulk", text: raw };
}

/** A-Z badges for the first 26 options, then 1-based numbers as a fallback. */
function optionLabel(index: number): string {
  return index < 26 ? String.fromCharCode(65 + index) : String(index + 1);
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
/** A real organization UUID — guards the unit_review DB fetch so a placeholder
 * (`"<id>"`) or a Python-ish `"None"` the model emits never gets queried. */
function isUuid(value: string | null | undefined): value is string {
  return !!value && UUID_RE.test(value.trim());
}

/** Subsidiary inclusion changes the authorized organization boundary. It must
 * stay pending until a person clicks a choice; the generic convenience timer
 * must never turn the first option into security authorization. */
export function isSubsidiaryScopeDecision(request: AskHumanState): boolean {
  if (request.inputType !== "choice") return false;

  try {
    let parsed: unknown = JSON.parse(request.context);
    if (typeof parsed === "string") parsed = JSON.parse(parsed);
    if (
      parsed &&
      typeof parsed === "object" &&
      !Array.isArray(parsed) &&
      (parsed as Record<string, unknown>).decision === "subsidiary_scope"
    ) {
      return true;
    }
  } catch {
    // Legacy prompts used human-readable context; the guarded fallback below
    // keeps already-running sessions from silently auto-approving them.
  }

  const prompt = [request.context, request.question, ...request.options].join(" ").toLowerCase();
  const namesSubsidiaries =
    prompt.includes("subsidiar") || prompt.includes("子公司") || prompt.includes("分支机构");
  const hasScopeOption = request.options.some((option) => {
    const normalized = option.toLowerCase();
    return (
      normalized.includes("不纳入子公司") ||
      normalized.includes("纳入子公司") ||
      normalized.includes("no subsidiaries") ||
      normalized.includes("include subsidiaries") ||
      normalized.includes("parent company only") ||
      normalized.includes("root only")
    );
  });
  return namesSubsidiaries && hasScopeOption;
}

/**
 * How long lightweight ask_human boxes wait before auto-running their default
 * action. Scope/unit reviews and subsidiary-scope choices are deliberate
 * security boundaries and never auto-confirm.
 */
export const ASK_HUMAN_COUNTDOWN_MS = 60_000;
const COUNTDOWN_TICK_MS = 100;

/**
 * Pausable countdown that fires `onExpire` exactly once when it reaches zero.
 * `paused` freezes the remaining time (hover / focus); `resetKey` restarts it
 * (a new request id). Returns the milliseconds left so the caller can render a
 * progress bar. The latest `onExpire` is kept in a ref so re-creating the
 * callback every render (it closes over editable form state) never restarts or
 * double-fires the timer.
 */
function useAutoConfirmCountdown(
  durationMs: number,
  paused: boolean,
  resetKey: string,
  onExpire: () => void
): number {
  const [remaining, setRemaining] = useState(durationMs);
  const firedRef = useRef(false);
  const onExpireRef = useRef(onExpire);
  onExpireRef.current = onExpire;

  useEffect(() => {
    firedRef.current = false;
    setRemaining(durationMs);
  }, [resetKey, durationMs]);

  const expired = remaining <= 0;

  // Depend on the `expired` boolean (not `remaining`) so the interval lives
  // across ticks and is torn down exactly once when the clock hits zero.
  useEffect(() => {
    if (paused || expired) return;
    const id = setInterval(() => {
      setRemaining((r) => Math.max(0, r - COUNTDOWN_TICK_MS));
    }, COUNTDOWN_TICK_MS);
    return () => clearInterval(id);
  }, [paused, expired]);

  useEffect(() => {
    if (expired && !firedRef.current) {
      firedRef.current = true;
      onExpireRef.current();
    }
  }, [expired]);

  return remaining;
}

export function AskHumanInline({
  request,
  onSubmit,
  onSkip,
  fallbackOrgId,
  minOwnershipPercent,
}: {
  request: AskHumanState;
  onSubmit: (response: string) => void;
  onSkip: () => void;
  /** Authoritative engagement org id captured from the `recon_discover_subsidiaries`
   * call, used to source unit_review candidates when the model didn't thread a
   * valid `organization_id` into the ask_human context. */
  fallbackOrgId?: string | null;
  /** Ownership threshold from the discovery call — unit_review candidates below it
   * are hidden so the user doesn't hand-delete sub-threshold rows. */
  minOwnershipPercent?: number | null;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [freetext, setFreetext] = useState("");
  // Cursor-style quick replies: each option submits on a single click. The
  // "Other" affordance reveals a free-text field for a custom answer. When a
  // choice request ships with no options we open that field straight away so the
  // user is never stuck with a dead end.
  const [showOther, setShowOther] = useState(
    request.inputType === "choice" && request.options.length === 0
  );
  const [otherText, setOtherText] = useState("");

  const Icon = INPUT_TYPE_ICONS[request.inputType] || MessageSquare;
  const isReviewTable = request.inputType === "scope_review" || request.inputType === "unit_review";
  const isScopeBoundaryDecision = isSubsidiaryScopeDecision(request);

  // Resolve where the review table's rows come from (org id → DB, an array, or
  // bulk text). Memoized so the fetch effect below doesn't re-run every render.
  const reviewSource = useMemo<ReviewSource>(
    () => (isReviewTable ? parseReviewContext(request.context) : { kind: "rows", rows: [] }),
    [isReviewTable, request.context]
  );
  // Resolve which org to source candidates from. Prefer a VALID uuid the model
  // put in context; otherwise (unit_review only) fall back to the org subsidiary
  // discovery actually ran for. This makes the table robust to the model passing
  // a placeholder / "None" / nothing — the proximate cause of the empty table.
  const contextOrgId =
    reviewSource.kind === "org" && isUuid(reviewSource.organizationId)
      ? reviewSource.organizationId
      : null;
  const orgId =
    contextOrgId ??
    (request.inputType === "unit_review" && isUuid(fallbackOrgId) ? fallbackOrgId : null);
  // For the org-sourced path the candidates load asynchronously from the DB;
  // null = still loading, [] = loaded-empty / failed. The table is remounted via
  // `key` once these arrive so its seeded textarea picks them up.
  const [dbRows, setDbRows] = useState<UnitReviewDecisionRow[] | null>(null);
  useEffect(() => {
    if (!orgId) return;
    let alive = true;
    listOrganizationCandidates(orgId)
      .then((c) => {
        // Filter to the discovery threshold only for unit_review (subsidiary
        // ownership); scope_review rows have no ownership semantics.
        const threshold = request.inputType === "unit_review" ? minOwnershipPercent : null;
        if (alive) setDbRows(candidatesToUnitRows(c.organizations, threshold));
      })
      .catch(() => {
        if (alive) setDbRows([]);
      });
    return () => {
      alive = false;
    };
  }, [orgId, minOwnershipPercent, request.inputType]);

  const submitOther = () => {
    const trimmed = otherText.trim();
    if (trimmed) onSubmit(trimmed);
  };

  const handleSubmit = () => {
    switch (request.inputType) {
      case "credentials":
        onSubmit(JSON.stringify({ username, password }));
        break;
      case "freetext":
        onSubmit(freetext);
        break;
      case "confirmation":
        onSubmit("yes");
        break;
    }
  };

  // Auto-confirm countdown for lightweight prompts. Review tables and scope
  // boundary decisions stay visible until the user confirms or skips.
  const reviewTableRef = useRef<ScopeReviewHandle>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const paused = hovered || focused;
  const autoConfirmEnabled = !isReviewTable && !isScopeBoundaryDecision;

  // Some inputs (the options-less "Other" field) autofocus on mount. That mount
  // focus must NOT pause the clock, otherwise an unattended box would dangle
  // forever — the exact bug this countdown fixes. Arm focus-pausing one tick
  // after mount so only focus/typing the user actually triggers counts.
  const focusArmedRef = useRef(false);
  useEffect(() => {
    const id = setTimeout(() => {
      focusArmedRef.current = true;
    }, 0);
    return () => clearTimeout(id);
  }, []);

  // When the clock runs out, perform the box's primary affirmative action — the
  // same thing the user clicking the main button (or the first choice) would do.
  // If that action is currently a no-op (an empty "Other" field), fall back to
  // Skip so the request still resolves and the box clears.
  const performDefaultAction = useCallback(() => {
    switch (request.inputType) {
      case "scope_review":
      case "unit_review":
        reviewTableRef.current?.confirm();
        break;
      case "choice":
        if (request.options.length > 0) onSubmit(request.options[0]);
        else if (otherText.trim()) onSubmit(otherText.trim());
        else onSkip();
        break;
      case "confirmation":
        onSubmit("yes");
        break;
      case "freetext":
        onSubmit(freetext);
        break;
      case "credentials":
        onSubmit(JSON.stringify({ username, password }));
        break;
      default:
        onSkip();
    }
  }, [
    request.inputType,
    request.options,
    otherText,
    freetext,
    username,
    password,
    onSubmit,
    onSkip,
  ]);

  const remainingMs = useAutoConfirmCountdown(
    ASK_HUMAN_COUNTDOWN_MS,
    paused || !autoConfirmEnabled,
    request.requestId,
    performDefaultAction
  );
  const countdownPct = Math.max(0, Math.min(100, (remainingMs / ASK_HUMAN_COUNTDOWN_MS) * 100));
  const countdownSeconds = Math.ceil(remainingMs / 1000);

  return (
    <div
      className="mx-4 my-2 rounded-lg border border-[#e0af68]/30 bg-[#e0af68]/5 p-3"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocusCapture={() => {
        if (focusArmedRef.current) setFocused(true);
      }}
      // Typing in an already-(auto)focused field also means the user is editing,
      // so pause even if no fresh focus event fired.
      onKeyDownCapture={() => setFocused(true)}
      onBlurCapture={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
      }}
    >
      <div className="flex items-center gap-2 text-[12px] font-medium text-[#e0af68] mb-2">
        <Icon className="w-3.5 h-3.5" />
        AI Needs Your Input
      </div>
      {/* Render the prompt as Markdown so tables / lists / headings the model
          emits (e.g. the scoping target table) display structured instead of as
          raw pipe-delimited text. */}
      <div className="mb-2 text-[13px]">
        <Markdown content={request.question} sessionId={request.sessionId} />
      </div>
      {request.context && !isReviewTable && (
        <p className="text-[11px] text-muted-foreground/60 mb-2 italic">{request.context}</p>
      )}

      {isReviewTable && (
        <ScopeReviewTable
          ref={reviewTableRef}
          // Remount when async DB rows arrive so the table re-seeds its textarea
          // from the freshly-loaded candidates (it reads `initial` only on mount).
          key={orgId ? `org-${dbRows ? dbRows.length : "loading"}` : "ctx"}
          kind={request.inputType as ScopeReviewKind}
          initial={
            orgId
              ? (dbRows ?? [])
              : reviewSource.kind === "rows"
                ? reviewSource.rows
                : reviewSource.kind === "bulk"
                  ? parseBulkRows(request.inputType as ScopeReviewKind, reviewSource.text)
                  : []
          }
          onConfirm={(rows) => {
            if (request.inputType === "unit_review") {
              const submission: UnitReviewSubmission = {
                rows: rows as UnitReviewDecisionRow[],
              };
              onSubmit(JSON.stringify(submission));
            } else {
              onSubmit(JSON.stringify(rows));
            }
          }}
          onSkip={onSkip}
        />
      )}

      {request.inputType === "credentials" && (
        <div className="space-y-2 mb-2">
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
            placeholder="Username..."
          />
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
            placeholder="Password..."
            onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
          />
        </div>
      )}

      {request.inputType === "choice" && (
        <div className="space-y-1 mb-2">
          {request.options.map((opt, i) => (
            <button
              key={opt}
              type="button"
              onClick={() => onSubmit(opt)}
              className="group w-full text-left px-2.5 py-1.5 rounded-md border border-border/50 bg-background text-[12px] flex items-center gap-2 hover:border-accent/50 hover:bg-accent/10 transition-colors"
            >
              <span className="flex h-4 w-4 flex-shrink-0 items-center justify-center rounded border border-border/60 text-[10px] font-semibold text-muted-foreground group-hover:border-accent/50 group-hover:text-accent">
                {optionLabel(i)}
              </span>
              <span className="flex-1">{opt}</span>
            </button>
          ))}

          {showOther ? (
            <div className="flex items-center gap-1.5 pt-0.5">
              <input
                type="text"
                // biome-ignore lint/a11y/noAutofocus: focus the field the user just revealed
                autoFocus
                value={otherText}
                onChange={(e) => setOtherText(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && submitOther()}
                className="flex-1 px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
                placeholder="Type your own answer..."
              />
              <button
                type="button"
                onClick={submitOther}
                disabled={!otherText.trim()}
                className="px-3 py-1.5 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              >
                Send
              </button>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => setShowOther(true)}
              className="w-full text-left px-2.5 py-1.5 rounded-md border border-dashed border-border/50 text-[12px] text-muted-foreground flex items-center gap-2 hover:border-accent/40 hover:text-foreground transition-colors"
            >
              <Pencil className="w-3 h-3 flex-shrink-0" />
              Other (type your own)...
            </button>
          )}
        </div>
      )}

      {request.inputType === "freetext" && (
        <textarea
          value={freetext}
          onChange={(e) => setFreetext(e.target.value)}
          className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent min-h-[60px] resize-y mb-2"
          placeholder="Type your response..."
        />
      )}

      {/* The review-table branch renders its own Confirm / Skip controls. */}
      {!isReviewTable && (
        <div className="flex items-center gap-2">
          {/* Choice options self-submit on click, so no generic Submit button there. */}
          {request.inputType !== "choice" && (
            <button
              type="button"
              onClick={handleSubmit}
              className="px-3 py-1 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors"
            >
              {request.inputType === "confirmation" ? "Confirm" : "Submit"}
            </button>
          )}
          <button
            type="button"
            onClick={onSkip}
            className="px-3 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:bg-muted/50 transition-colors"
          >
            Skip
          </button>
        </div>
      )}

      {autoConfirmEnabled ? (
        <div className="mt-2.5 select-none">
          <div className="h-1 w-full overflow-hidden rounded-full bg-border/40">
            <div
              className={cn(
                "h-full rounded-full transition-[width] duration-100 ease-linear",
                paused ? "bg-muted-foreground/40" : "bg-[#e0af68]"
              )}
              style={{ width: `${countdownPct}%` }}
            />
          </div>
          <div className="mt-1 text-[10px] text-muted-foreground/60">
            {paused
              ? "Paused — auto-confirm resumes when you move away"
              : `Auto-confirming in ${countdownSeconds}s`}
          </div>
        </div>
      ) : (
        <div className="mt-2.5 rounded-md border border-[#e0af68]/20 bg-background/30 px-2 py-1 text-[10px] text-muted-foreground/70">
          {isScopeBoundaryDecision ? "Waiting for your scope decision" : "Waiting for your review"}
        </div>
      )}
    </div>
  );
}
