import type { Target } from "@/lib/pentest/types";
import { Kv, Section } from "../SurfaceParts";

export function IdentityTab({
  target,
  onUpdateNotes,
}: {
  target: Target;
  onUpdateNotes: (id: string, notes: string) => void;
}) {
  return (
    <div className="space-y-2.5">
      <Section title="Identity" subtitle="scope, source, and ownership signals">
        <div className="grid grid-cols-2 gap-1.5">
          <Kv label="Target" value={target.value} mono />
          <Kv label="Type" value={target.type} />
          <Kv label="Source" value={target.source || "manual"} />
          <Kv label="Scope" value={target.scope} />
          <Kv label="Resolved IP" value={target.real_ip || "—"} mono />
          <Kv label="CDN / WAF" value={target.cdn_waf || "—"} />
          <Kv label="OS" value={target.os_info || "—"} />
          <Kv label="Owner" value={target.owner || "—"} />
        </div>
      </Section>
      <Section title="Notes" subtitle="target-level operator context">
        <textarea
          className="min-h-24 w-full resize-y rounded border border-border/35 bg-background/40 px-2 py-1.5 text-[11px] outline-none focus:border-accent"
          placeholder="Target notes"
          defaultValue={target.notes}
          onBlur={(event) => {
            if (event.target.value !== target.notes) {
              onUpdateNotes(target.id, event.target.value);
            }
          }}
        />
      </Section>
    </div>
  );
}
