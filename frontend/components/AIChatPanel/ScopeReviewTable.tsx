import { Plus, Trash2 } from "lucide-react";
import { useRef, useState } from "react";

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

/**
 * Editable confirmation table for the scoping HITL flow (design
 * 2026-06-06-scoping-per-mode-gate-hitl §3.6). The user adds / removes / edits
 * the AI-proposed target list (`scope_review`) or candidate units
 * (`unit_review`); "Confirm" hands the edited rows back so the caller can submit
 * them as the `ask_human` response.
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
  const columns = COLUMNS[kind];
  // Stable per-row ids so editing/removing a row doesn't make React reconcile by
  // position (which would carry one row's input state onto its neighbour).
  const nextId = useRef(0);
  const [rows, setRows] = useState<{ id: number; cells: ScopeReviewRow }[]>(() =>
    normalizeScopeRows(kind, initial).map((cells) => ({ id: nextId.current++, cells }))
  );

  const updateCell = (id: number, key: string, value: string) => {
    setRows((prev) =>
      prev.map((row) => (row.id === id ? { ...row, cells: { ...row.cells, [key]: value } } : row))
    );
  };
  const addRow = () =>
    setRows((prev) => [...prev, { id: nextId.current++, cells: emptyRow(columns) }]);
  const removeRow = (id: number) => setRows((prev) => prev.filter((row) => row.id !== id));

  // Drop rows whose first column is blank so the caller never gets empty entries.
  const handleConfirm = () => {
    const firstKey = columns[0].key;
    onConfirm(
      rows.map((row) => row.cells).filter((cells) => (cells[firstKey] ?? "").trim() !== "")
    );
  };

  return (
    <div className="space-y-2">
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-muted-foreground">
            {columns.map((col) => (
              <th key={col.key} className="pb-1 pr-2 font-medium">
                {col.label}
              </th>
            ))}
            <th className="pb-1 w-8" aria-label="Remove" />
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={row.id}>
              {columns.map((col) => (
                <td key={col.key} className="py-0.5 pr-2 align-top">
                  {col.kind === "select" ? (
                    <select
                      aria-label={`${col.label} for row ${i + 1}`}
                      value={row.cells[col.key] ?? ""}
                      onChange={(e) => updateCell(row.id, col.key, e.target.value)}
                      className="w-full px-2 py-1 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
                    >
                      {col.options?.map((opt) => (
                        <option key={opt} value={opt}>
                          {opt}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      type="text"
                      aria-label={`${col.label} for row ${i + 1}`}
                      value={row.cells[col.key] ?? ""}
                      onChange={(e) => updateCell(row.id, col.key, e.target.value)}
                      className="w-full px-2 py-1 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
                    />
                  )}
                </td>
              ))}
              <td className="py-0.5 align-top">
                <button
                  type="button"
                  aria-label={`Remove row ${i + 1}`}
                  onClick={() => removeRow(row.id)}
                  className="p-1 rounded-md text-muted-foreground hover:text-red-400 hover:bg-red-400/10 transition-colors"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <button
        type="button"
        onClick={addRow}
        className="flex items-center gap-1 px-2 py-1 text-[11px] rounded-md border border-dashed border-border/50 text-muted-foreground hover:border-accent/40 hover:text-foreground transition-colors"
      >
        <Plus className="w-3 h-3" />
        Add row
      </button>

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
