import { type CheckCircle2, ShieldAlert, ShieldCheck, ShieldX } from "lucide-react";
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
