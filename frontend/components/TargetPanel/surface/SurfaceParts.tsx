import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";

export function StageButton({
  icon,
  label,
  muted = false,
}: {
  icon: React.ReactNode;
  label: string;
  muted?: boolean;
}) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded border px-1.5 text-[10px] transition-colors",
        muted
          ? "border-border/30 bg-background/20 text-muted-foreground hover:bg-muted/25 hover:text-foreground"
          : "border-accent/25 bg-accent/10 text-accent hover:bg-accent/15"
      )}
    >
      {icon}
      {label}
    </button>
  );
}

export function Section({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded border border-border/25 bg-background/15">
      <div className="flex h-8 items-center justify-between border-b border-border/20 px-2.5">
        <h4 className="text-[11px] font-medium text-foreground">{title}</h4>
        {subtitle && <span className="text-[9px] text-muted-foreground">{subtitle}</span>}
      </div>
      <div className="p-2.5">{children}</div>
    </section>
  );
}

export function Kv({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="rounded border border-border/20 bg-muted/5 p-2">
      <p className="text-[10px] text-muted-foreground">{label}</p>
      <p className={cn("mt-1 truncate text-[11px] text-foreground", mono && "font-mono")}>
        {value}
      </p>
    </div>
  );
}

export function Metric({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: number;
}) {
  return (
    <div className="rounded border border-border/20 bg-muted/5 p-2">
      <div className="flex items-center gap-1.5 text-muted-foreground">
        {icon}
        <span className="text-[10px]">{label}</span>
      </div>
      <p className="mt-0.5 text-base font-semibold tabular-nums text-foreground">{value}</p>
    </div>
  );
}

export function EmptyInline({ label, loading }: { label: string; loading?: boolean }) {
  return (
    <div className="rounded border border-dashed border-border/25 bg-background/10 p-3 text-center text-[11px] text-muted-foreground">
      {loading ? (
        <span className="inline-flex items-center gap-2">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading target surface data
        </span>
      ) : (
        label
      )}
    </div>
  );
}

export function EmptyPanel({
  icon,
  title,
  body,
  loading,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
  loading?: boolean;
}) {
  return (
    <div className="flex min-h-[180px] items-center justify-center rounded border border-dashed border-border/25 bg-background/10 p-6 text-center">
      <div className="max-w-sm">
        <div className="mx-auto mb-2.5 flex h-8 w-8 items-center justify-center rounded bg-muted/15 text-muted-foreground">
          {loading ? <Loader2 className="w-5 h-5 animate-spin" /> : icon}
        </div>
        <h4 className="text-xs font-medium text-foreground">{title}</h4>
        <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">{body}</p>
      </div>
    </div>
  );
}
