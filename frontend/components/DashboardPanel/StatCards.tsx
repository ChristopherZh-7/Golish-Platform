import { type CheckCircle2, ShieldAlert, ShieldCheck, ShieldX } from "lucide-react";
import type { ProjectMethodology } from "@/lib/dashboard";
import { SEV_HEX as SEV_COLORS } from "@/lib/severity";
import { cn } from "@/lib/utils";

const SEV_ORDER = ["critical", "high", "medium", "low", "info"];

export function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

export function SeverityBar({ data }: { data: Record<string, number> }) {
  const total = Object.values(data).reduce((s, v) => s + v, 0);
  if (total === 0) return null;

  return (
    <div className="space-y-1.5">
      <div className="flex h-2 rounded-full overflow-hidden bg-muted/20">
        {SEV_ORDER.map((sev) => {
          const count = data[sev] || 0;
          if (count === 0) return null;
          return (
            <div
              key={sev}
              className="h-full transition-all duration-700"
              style={{
                width: `${(count / total) * 100}%`,
                backgroundColor: SEV_COLORS[sev],
                opacity: 0.8,
              }}
              title={`${sev}: ${count}`}
            />
          );
        })}
      </div>
      <div className="flex items-center gap-3">
        {SEV_ORDER.map((sev) => {
          const count = data[sev] || 0;
          if (count === 0) return null;
          return (
            <div key={sev} className="flex items-center gap-1 text-[10px]">
              <div
                className="w-1.5 h-1.5 rounded-full"
                style={{ backgroundColor: SEV_COLORS[sev] }}
              />
              <span className="text-muted-foreground/50 capitalize">{sev}</span>
              <span className="font-medium text-foreground/70">{count}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function MiniTimeline({ data }: { data: { date: string; count: number }[] }) {
  if (data.length < 2) return null;
  const max = Math.max(...data.map((d) => d.count), 1);
  const w = 260;
  const h = 48;
  const padY = 2;
  const usableH = h - padY * 2;
  const stepX = w / (data.length - 1);

  const points = data.map((d, i) => ({
    x: i * stepX,
    y: padY + usableH - (d.count / max) * usableH,
  }));

  const line = points.map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`).join(" ");
  const area = `${line} L ${points[points.length - 1].x} ${h} L 0 ${h} Z`;

  return (
    <div className="space-y-1">
      <svg aria-hidden="true" width={w} height={h} className="overflow-visible">
        <defs>
          <linearGradient id="dash-tl-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.15" />
            <stop offset="100%" stopColor="var(--accent)" stopOpacity="0.02" />
          </linearGradient>
        </defs>
        <path d={area} fill="url(#dash-tl-fill)" />
        <path d={line} fill="none" stroke="var(--accent)" strokeWidth="1.5" opacity="0.5" />
        {points.map((p, i) => (
          <circle key={i} cx={p.x} cy={p.y} r={1.5} fill="var(--accent)" opacity="0.7">
            <title>
              {data[i].date}: {data[i].count}
            </title>
          </circle>
        ))}
      </svg>
      <div className="flex justify-between text-[8px] text-muted-foreground/25 px-0.5">
        <span>{data[0].date}</span>
        <span>{data[data.length - 1].date}</span>
      </div>
    </div>
  );
}

export function MethodologyRing({ projects }: { projects: ProjectMethodology[] }) {
  let total = 0;
  let checked = 0;
  for (const p of projects) {
    for (const phase of p.phases) {
      total += phase.items.length;
      checked += phase.items.filter((i) => i.checked).length;
    }
  }
  if (total === 0) return null;
  const pct = Math.round((checked / total) * 100);
  const r = 32;
  const strokeW = 5;
  const circ = 2 * Math.PI * r;
  const offset = circ - (pct / 100) * circ;

  return (
    <div className="flex items-center gap-4">
      <div className="relative">
        <svg aria-hidden="true" width={78} height={78}>
          <circle
            cx={39}
            cy={39}
            r={r}
            fill="none"
            stroke="currentColor"
            strokeWidth={strokeW}
            className="text-muted/20"
          />
          <circle
            cx={39}
            cy={39}
            r={r}
            fill="none"
            strokeWidth={strokeW}
            strokeDasharray={circ}
            strokeDashoffset={offset}
            strokeLinecap="round"
            className={cn(
              "transition-all duration-1000 ease-out",
              pct === 100
                ? "stroke-green-500/80"
                : pct > 50
                  ? "stroke-accent/70"
                  : "stroke-amber-500/60"
            )}
            transform="rotate(-90 39 39)"
          />
        </svg>
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <span className="text-base font-bold leading-none">{pct}%</span>
        </div>
      </div>
      <div className="space-y-1 min-w-0 flex-1">
        {projects.map((p) => {
          const pTotal = p.phases.reduce((a, ph) => a + ph.items.length, 0);
          const pDone = p.phases.reduce((a, ph) => a + ph.items.filter((i) => i.checked).length, 0);
          const pp = pTotal > 0 ? Math.round((pDone / pTotal) * 100) : 0;
          return (
            <div key={p.id} className="space-y-0.5">
              <div className="flex items-center justify-between text-[10px]">
                <span className="text-foreground/70 truncate">{p.project_name}</span>
                <span className="text-muted-foreground/40 tabular-nums">
                  {pDone}/{pTotal}
                </span>
              </div>
              <div className="h-1 rounded-full bg-muted/15 overflow-hidden">
                <div
                  className={cn(
                    "h-full rounded-full transition-all duration-700",
                    pp === 100 ? "bg-green-500/70" : pp > 50 ? "bg-accent/50" : "bg-amber-500/50"
                  )}
                  style={{ width: `${pp}%` }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function MetricCard({
  icon: Icon,
  value,
  label,
  detail,
  accent,
  displayValue,
}: {
  icon: typeof CheckCircle2;
  value: number;
  label: string;
  detail?: string;
  accent: "blue" | "red" | "amber" | "green" | "purple";
  displayValue?: string;
}) {
  const colors: Record<string, string> = {
    blue: "bg-blue-500/8 text-blue-400/70 border-blue-500/10",
    red: "bg-red-500/8 text-red-400/70 border-red-500/10",
    amber: "bg-amber-500/8 text-amber-400/70 border-amber-500/10",
    green: "bg-green-500/8 text-green-400/70 border-green-500/10",
    purple: "bg-purple-500/8 text-purple-400/70 border-purple-500/10",
  };
  const iconColors: Record<string, string> = {
    blue: "bg-blue-500/10 text-blue-400/60",
    red: "bg-red-500/10 text-red-400/60",
    amber: "bg-amber-500/10 text-amber-400/60",
    green: "bg-green-500/10 text-green-400/60",
    purple: "bg-purple-500/10 text-purple-400/60",
  };

  return (
    <div className={cn("rounded-xl border p-3.5 space-y-2 transition-colors", colors[accent])}>
      <div className="flex items-center justify-between">
        <div className={cn("p-1.5 rounded-lg", iconColors[accent])}>
          <Icon className="w-3.5 h-3.5" />
        </div>
        <span className="text-2xl font-bold leading-none text-foreground/85 tabular-nums">
          {displayValue ?? fmtNum(value)}
        </span>
      </div>
      <div>
        <div className="text-[11px] font-medium text-foreground/50">{label}</div>
        {detail && <div className="text-[9px] text-muted-foreground/35 mt-0.5">{detail}</div>}
      </div>
    </div>
  );
}

export function SevIcon({ severity }: { severity: string }) {
  if (severity === "critical" || severity === "high") {
    return <ShieldX className="w-3 h-3 text-red-400/70 flex-shrink-0" />;
  }
  if (severity === "medium") {
    return <ShieldAlert className="w-3 h-3 text-orange-400/70 flex-shrink-0" />;
  }
  return <ShieldCheck className="w-3 h-3 text-blue-400/70 flex-shrink-0" />;
}
