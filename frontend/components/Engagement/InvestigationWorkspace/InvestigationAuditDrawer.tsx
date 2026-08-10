import { ChevronDown, ShieldCheck } from "lucide-react";
import { useState } from "react";

export interface InvestigationAuditField {
  label: string;
  value: string | number | null | undefined;
}

export function InvestigationAuditDrawer({
  fields,
  title = "Execution audit",
}: {
  fields: InvestigationAuditField[];
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const visible = fields.filter((field) => field.value !== null && field.value !== undefined);

  return (
    <section className="rounded border border-border/30 bg-background/20">
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-[11px] text-muted-foreground hover:text-foreground"
        onClick={() => setOpen((value) => !value)}
      >
        <ShieldCheck className="h-3.5 w-3.5" />
        <span>{title}</span>
        <ChevronDown className={`ml-auto h-3.5 w-3.5 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>
      {open && (
        <dl className="grid gap-2 border-t border-border/25 p-3 text-[10px] sm:grid-cols-2">
          {visible.length === 0 ? (
            <div className="text-muted-foreground">No scheduler audit fields were recorded.</div>
          ) : (
            visible.map((field) => (
              <div key={field.label} className="min-w-0 rounded bg-muted/15 px-2 py-1.5">
                <dt className="text-muted-foreground">{field.label}</dt>
                <dd className="mt-0.5 break-all font-mono text-foreground/80">{field.value}</dd>
              </div>
            ))
          )}
        </dl>
      )}
    </section>
  );
}
