import { ClipboardList, Upload } from "lucide-react";
import { useRef, useState } from "react";
import { cn } from "@/lib/utils";

export type ScopeReviewKind = "scope_review" | "unit_review";

/** A single editable row — a flat string map keyed by the active columns. */
export type ScopeReviewRow = Record<string, string>;

interface Column {
  key: string;
  label: string;
  /** "select" renders a dropdown constrained to `options`; otherwise a text input. */
  kind: "text" | "select";
  options?: string[];
}

const COLUMNS: Record<ScopeReviewKind, Column[]> = {
  scope_review: [
    { key: "value", label: "Target", kind: "text" },
    {
      key: "type",
      label: "Type",
      kind: "select",
      options: ["domain", "ip", "cidr", "url", "wildcard"],
    },
    { key: "scope", label: "Scope", kind: "select", options: ["in", "out"] },
  ],
  unit_review: [
    { key: "name", label: "Unit / Organization", kind: "text" },
    { key: "aliases", label: "Aliases", kind: "text" },
    { key: "domains", label: "Domains", kind: "text" },
  ],
};

function emptyRow(columns: Column[]): ScopeReviewRow {
  const row: ScopeReviewRow = {};
  for (const col of columns) {
    row[col.key] = col.kind === "select" ? (col.options?.[0] ?? "") : "";
  }
  return row;
}

/** Coerce a value of unknown shape (string / array / number / null) into the
 * flat string a cell edits. Arrays join with ", " so domains/aliases round-trip. */
function cellValue(raw: unknown, col: Column): string {
  if (typeof raw === "string") return raw;
  if (Array.isArray(raw)) return raw.map((v) => String(v)).join(", ");
  if (raw === null || raw === undefined) {
    return col.kind === "select" ? (col.options?.[0] ?? "") : "";
  }
  return String(raw);
}

/** Normalize the AI-proposed `initial` payload into editable rows. Non-array or
 * empty input yields a single blank row so the user is never stuck. */
export function normalizeScopeRows(kind: ScopeReviewKind, initial: unknown): ScopeReviewRow[] {
  const columns = COLUMNS[kind];
  if (!Array.isArray(initial) || initial.length === 0) {
    return [emptyRow(columns)];
  }
  return initial.map((item) => {
    const row: ScopeReviewRow = {};
    for (const col of columns) {
      row[col.key] = cellValue((item as Record<string, unknown> | null)?.[col.key], col);
    }
    return row;
  });
}

/** Heuristically classify a raw target string into one of the `scope_review`
 * `type` options so pasted / uploaded targets land on a sensible default. */
export function detectTargetType(raw: string): string {
  const v = raw.trim().toLowerCase();
  if (!v) return "domain";
  if (/^https?:\/\//.test(v)) return "url";
  if (v.startsWith("*.")) return "wildcard";
  if (v.includes("/")) {
    if (/^\d{1,3}(\.\d{1,3}){3}\/\d{1,2}$/.test(v)) return "cidr";
    if (v.includes(":") && /^[0-9a-f:]+\/\d{1,3}$/.test(v)) return "cidr";
    return "url";
  }
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(v)) return "ip";
  if (v.includes(":") && /^[0-9a-f:]+$/.test(v)) return "ip";
  return "domain";
}

/** Strip leading markdown list markers ("- ", "* ", "1. ", "•") from a line. */
function stripListMarkers(line: string): string {
  return line.replace(/^\s*(?:[-*•]|\d+[.)])\s+/, "").trim();
}

/** Pure markdown/punctuation noise that should never become a target. */
const JUNK_TOKEN = /^[-=#|*_`'"().,;:]+$/;
/** A scope target always carries a dot, colon or slash (FQDN / IP / CIDR / URL). */
const TARGET_LIKE = /[.:/]/;

/**
 * Parse the free-form pasted / uploaded text into rows for submission. For
 * `scope_review` every whitespace / comma / semicolon / pipe-separated token
 * that looks like a target becomes a row with an auto-detected type — so a
 * plain list, a CSV, or even a pasted markdown table all collapse cleanly. For
 * `unit_review` each non-empty line is treated as one organisation name.
 */
export function parseBulkRows(kind: ScopeReviewKind, text: string): ScopeReviewRow[] {
  const rows: ScopeReviewRow[] = [];
  const seen = new Set<string>();
  for (const rawLine of text.split(/\r?\n/)) {
    const line = stripListMarkers(rawLine);
    if (!line) continue;
    if (kind === "unit_review") {
      const key = line.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({ name: line, aliases: "", domains: "" });
      continue;
    }
    const tokens = line
      .split(/[\s,;|]+/)
      .map((t) =>
        t
          .replace(/^[`'"<([]+/, "")
          .replace(/[`'">)\].,;]+$/, "")
          .trim()
      )
      .filter((t) => t.length > 0 && TARGET_LIKE.test(t) && !JUNK_TOKEN.test(t));
    for (const token of tokens) {
      const key = token.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({ value: token, type: detectTargetType(token), scope: "in" });
    }
  }
  return rows;
}

/** Seed the editor textarea with the AI-proposed targets (one per line) so the
 * user edits them as free text instead of re-typing. Empty when nothing was
 * proposed. */
function initialBulkText(kind: ScopeReviewKind, initial: unknown): string {
  if (!Array.isArray(initial) || initial.length === 0) return "";
  const firstKey = COLUMNS[kind][0].key;
  return normalizeScopeRows(kind, initial)
    .map((row) => (row[firstKey] ?? "").trim())
    .filter((v) => v.length > 0)
    .join("\n");
}

/**
 * Confirmation editor for the scoping HITL flow (design
 * 2026-06-06-scoping-per-mode-gate-hitl §3.6). The user reviews / edits the
 * target list (`scope_review`) or candidate units (`unit_review`) as a single
 * free-form textarea — one entry per line, or comma / space separated — and can
 * paste, drag-drop, or upload a `.txt` / `.csv` file. "Confirm" parses the text
 * into rows (auto-detecting target type) and hands them back as the `ask_human`
 * response. The box is always expanded; the AI's proposed targets are seeded in
 * so editing is one keystroke away.
 */
export function ScopeReviewTable({
  kind,
  initial,
  onConfirm,
  onSkip,
}: {
  kind: ScopeReviewKind;
  initial: unknown;
  onConfirm: (rows: ScopeReviewRow[]) => void;
  onSkip: () => void;
}) {
  const [bulkText, setBulkText] = useState(() => initialBulkText(kind, initial));
  const [dragOver, setDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Read dropped / picked text files into the textarea, appending so multiple
  // files (or a file plus a manual paste) accumulate rather than overwrite.
  const handleFiles = async (files: FileList | null) => {
    if (!files || files.length === 0) return;
    try {
      const texts = await Promise.all(Array.from(files).map((file) => file.text()));
      const joined = texts.join("\n").trim();
      if (!joined) return;
      setBulkText((prev) => (prev.trim() ? `${prev}\n${joined}` : joined));
    } catch {
      /* ignore unreadable files */
    }
  };

  const handleConfirm = () => onConfirm(parseBulkRows(kind, bulkText));

  return (
    <div className="space-y-2">
      <div className="rounded-md border border-border/40 bg-background/40">
        <div className="flex items-center gap-1.5 px-2 py-1.5 text-[11px] font-medium text-muted-foreground">
          <ClipboardList className="w-3.5 h-3.5" />
          {kind === "scope_review"
            ? "Targets — paste a list or upload a file"
            : "Units / organizations"}
        </div>
        <div className="px-2 pb-2 space-y-1.5">
          <textarea
            aria-label="Bulk targets"
            value={bulkText}
            onChange={(e) => setBulkText(e.target.value)}
            onDragOver={(e) => {
              e.preventDefault();
              setDragOver(true);
            }}
            onDragLeave={() => setDragOver(false)}
            onDrop={(e) => {
              e.preventDefault();
              setDragOver(false);
              void handleFiles(e.dataTransfer.files);
            }}
            placeholder={
              kind === "scope_review"
                ? "One target per line, or comma-separated:\nexample.com\n*.example.com\n10.0.0.0/24\nhttps://app.example.com"
                : "One organization per line:\nAcme Corp\nAcme Subsidiary"
            }
            className={cn(
              "w-full px-2 py-1.5 rounded-md bg-background border text-[12px] font-mono focus:outline-none focus:border-accent min-h-[120px] resize-y",
              dragOver ? "border-accent border-dashed" : "border-border/50"
            )}
          />
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              className="flex items-center gap-1 px-2.5 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:border-accent/40 transition-colors"
            >
              <Upload className="w-3 h-3" />
              Upload file
            </button>
            <span className="text-[10px] text-muted-foreground/60">
              or drag &amp; drop a .txt / .csv
            </span>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              accept=".txt,.csv,.list,.text,text/plain,text/csv"
              className="hidden"
              onChange={(e) => {
                void handleFiles(e.target.files);
                e.target.value = "";
              }}
            />
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 pt-1">
        <button
          type="button"
          onClick={handleConfirm}
          className="px-3 py-1 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors"
        >
          Confirm
        </button>
        <button
          type="button"
          onClick={onSkip}
          className="px-3 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:bg-muted/50 transition-colors"
        >
          Skip
        </button>
      </div>
    </div>
  );
}
