/**
 * JsonView — a compact, AI/human-friendly renderer for arbitrary JSON tool I/O.
 *
 * Replaces the raw `<pre>{JSON.stringify(...)}</pre>` dump in tool detail panels
 * with:
 *  - (A) a collapsible, syntax-highlighted tree (keys / strings / numbers /
 *        booleans / null are color-coded; objects & arrays fold);
 *  - (B) an automatic table when a value is an array of flat objects (e.g. a
 *        providers / records list), which is far easier to scan than nested JSON.
 *
 * Pure presentational; no app state. Drop it anywhere a JSON value is shown.
 */
import { ChevronDown, ChevronRight } from "lucide-react";
import { memo, useState } from "react";
import { stripAnsiForDisplay } from "@/lib/ansi";
import { cn } from "@/lib/utils";

const KEY_CLASS = "text-[var(--ansi-cyan)]/80";
const STRING_CLASS = "text-[var(--ansi-green)]/85";
const NUMBER_CLASS = "text-[var(--ansi-yellow)]/90";
const BOOL_CLASS = "text-[var(--ansi-magenta)]/90";
const NULL_CLASS = "text-muted-foreground/60";

const MAX_TABLE_COLUMNS = 12;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * If `arr` is a non-empty array of flat-ish objects, return the union of their
 * keys (so it can render as a table); otherwise `null` (fall back to the tree).
 */
function tableColumns(arr: unknown[]): string[] | null {
  if (arr.length === 0 || !arr.every(isPlainObject)) return null;
  const columns: string[] = [];
  for (const row of arr as Record<string, unknown>[]) {
    for (const key of Object.keys(row)) {
      if (!columns.includes(key)) columns.push(key);
    }
  }
  return columns.length > 0 && columns.length <= MAX_TABLE_COLUMNS ? columns : null;
}

function cellText(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "object") return JSON.stringify(value);
  return typeof value === "string" ? stripAnsiForDisplay(value) : String(value);
}

const Primitive = memo(function Primitive({ value }: { value: unknown }) {
  if (value === null || value === undefined) return <span className={NULL_CLASS}>null</span>;
  switch (typeof value) {
    case "string":
      return <span className={cn(STRING_CLASS, "break-words")}>{stripAnsiForDisplay(value)}</span>;
    case "number":
      return <span className={NUMBER_CLASS}>{String(value)}</span>;
    case "boolean":
      return <span className={BOOL_CLASS}>{String(value)}</span>;
    default:
      return <span className="text-foreground/80 break-words">{String(value)}</span>;
  }
});

function ObjectArrayTable({
  rows,
  columns,
}: {
  rows: Record<string, unknown>[];
  columns: string[];
}) {
  return (
    <div className="overflow-auto max-h-64 rounded border border-border/20">
      <table className="w-full border-collapse text-[11px] font-mono">
        <thead>
          <tr className="bg-muted/40">
            {columns.map((col) => (
              <th
                key={col}
                className={cn(
                  "border-b border-border/20 px-2 py-1 text-left font-medium",
                  KEY_CLASS
                )}
              >
                {col}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex} className="even:bg-muted/15">
              {columns.map((col) => {
                const value = row[col];
                const text = cellText(value);
                return (
                  <td
                    key={col}
                    className="max-w-[280px] truncate border-b border-border/10 px-2 py-1 align-top"
                    title={text}
                  >
                    {typeof value === "boolean" ? <Primitive value={value} /> : text}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function JsonNode({ keyName, value, depth }: { keyName?: string; value: unknown; depth: number }) {
  const isArray = Array.isArray(value);
  const isObject = isPlainObject(value);
  const [open, setOpen] = useState(depth < 2);

  if (!isArray && !isObject) {
    return (
      <div className="flex items-baseline gap-2 py-0.5">
        {keyName !== undefined && (
          <span className={cn("flex-shrink-0", KEY_CLASS)}>{keyName}:</span>
        )}
        <Primitive value={value} />
      </div>
    );
  }

  const columns = isArray ? tableColumns(value as unknown[]) : null;
  const count = isArray ? (value as unknown[]).length : Object.keys(value as object).length;
  const summary = isArray ? `[${count}]` : `{${count}}`;

  return (
    <div className="py-0.5">
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="flex items-center gap-1 text-left hover:text-foreground"
      >
        {open ? (
          <ChevronDown className="h-3 w-3 flex-shrink-0 opacity-60" />
        ) : (
          <ChevronRight className="h-3 w-3 flex-shrink-0 opacity-60" />
        )}
        {keyName !== undefined && <span className={KEY_CLASS}>{keyName}</span>}
        <span className="text-muted-foreground/50">{summary}</span>
      </button>
      {open && (
        <div className="ml-3 mt-0.5 border-l border-border/15 pl-2">
          {isArray && columns ? (
            <ObjectArrayTable rows={value as Record<string, unknown>[]} columns={columns} />
          ) : isArray ? (
            (value as unknown[]).map((item, index) => (
              <JsonNode key={index} keyName={String(index)} value={item} depth={depth + 1} />
            ))
          ) : (
            Object.entries(value as Record<string, unknown>).map(([childKey, childValue]) => (
              <JsonNode key={childKey} keyName={childKey} value={childValue} depth={depth + 1} />
            ))
          )}
        </div>
      )}
    </div>
  );
}

export const JsonView = memo(function JsonView({
  value,
  className,
}: {
  value: unknown;
  className?: string;
}) {
  return (
    <div className={cn("text-[11px] font-mono leading-relaxed text-foreground/80", className)}>
      <JsonNode value={value} depth={0} />
    </div>
  );
});
