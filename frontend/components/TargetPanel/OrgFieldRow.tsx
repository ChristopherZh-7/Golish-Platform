/**
 * `OrgFieldRow` — renders a single organization intel field inside the
 * workspace "Fields" tab. Extracted from `TargetGroupedView.tsx`; all the
 * value-shaping logic lives in `@/lib/target-panel/org-fields`.
 */

import {
  getOrgFieldChips,
  getOrgFieldDisplayKind,
  getOrgFieldIntelRecords,
  ORG_FIELD_CHIP_INLINE_LIMIT,
  type OrgFieldView,
} from "@/lib/target-panel/org-fields";

export function OrgFieldRow({ field }: { field: OrgFieldView }) {
  if (!field.filled) {
    return (
      <div className="flex items-start justify-between gap-2 text-[10px]">
        <span className="text-muted-foreground/70">{field.label}</span>
        <span className="text-muted-foreground/40">—</span>
      </div>
    );
  }

  const kind = getOrgFieldDisplayKind(field);

  if (kind === "chips") {
    const chips = getOrgFieldChips(field.raw);
    const inline = chips.slice(0, ORG_FIELD_CHIP_INLINE_LIMIT);
    const more = chips.length - inline.length;
    return (
      <div className="text-[10px]">
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground">{field.label}</span>
          <span className="text-muted-foreground/60">{chips.length}</span>
        </div>
        <div className="mt-1 flex flex-wrap gap-1">
          {inline.map((chip, idx) => (
            <span
              key={`${field.key}-${idx}-${chip}`}
              className="rounded bg-muted/40 px-1.5 py-0.5 text-foreground break-all"
              title={chip}
            >
              {chip}
            </span>
          ))}
          {more > 0 && (
            <span className="rounded bg-muted/20 px-1.5 py-0.5 text-muted-foreground">
              +{more} more
            </span>
          )}
        </div>
      </div>
    );
  }

  if (kind === "records") {
    const records = getOrgFieldIntelRecords(field.raw);
    if (records.length === 0) {
      return (
        <div className="flex items-start justify-between gap-2 text-[10px]">
          <span className="text-muted-foreground/70">{field.label}</span>
          <span className="text-muted-foreground/40">—</span>
        </div>
      );
    }
    return (
      <div className="text-[10px]">
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground">{field.label}</span>
          <span className="text-muted-foreground/60">{records.length}</span>
        </div>
        <div className="mt-1 space-y-0.5">
          {records.map((entry) => (
            <div key={entry.key} className="flex items-start gap-2">
              <span className="text-muted-foreground/80 shrink-0 w-28">{entry.label}</span>
              <span className="text-foreground break-all">{entry.value}</span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-start justify-between gap-2 text-[10px]">
      <span className="text-muted-foreground shrink-0">{field.label}</span>
      <span className="text-foreground text-right break-all">{field.value}</span>
    </div>
  );
}
