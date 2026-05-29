import {
  Crosshair,
  Filter,
  Focus,
  GitBranch,
  Layers3,
  Network,
  RotateCcw,
  Search,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { TopologyMode, TopologyVisibility } from "./types";

const MODE_OPTIONS: Array<{
  id: TopologyMode;
  label: string;
  hint: string;
  icon: React.ReactNode;
}> = [
  {
    id: "ownership",
    label: "Ownership",
    hint: "org chain",
    icon: <GitBranch className="h-3 w-3" />,
  },
  { id: "surface", label: "Surface", hint: "ports", icon: <Network className="h-3 w-3" /> },
  { id: "evidence", label: "Evidence", hint: "ledger", icon: <Layers3 className="h-3 w-3" /> },
];

const VISIBILITY_OPTIONS: Array<{
  id: keyof TopologyVisibility;
  label: string;
  color: string;
}> = [
  { id: "organization", label: "Orgs", color: "bg-cyan-300" },
  { id: "target", label: "Targets", color: "bg-blue-300" },
  { id: "service", label: "Services", color: "bg-emerald-300" },
  { id: "evidence", label: "Evidence", color: "bg-amber-300" },
];

export function TopologyControls({
  mode,
  visibility,
  query,
  stats,
  selectedLabel,
  onModeChange,
  onVisibilityChange,
  onQueryChange,
  onFitSelected,
  focusActive,
  focusLabel,
  canIsolate,
  onIsolateSelected,
  onClearFocus,
}: {
  mode: TopologyMode;
  visibility: TopologyVisibility;
  query: string;
  stats: { organizations: number; targets: number; services: number; evidence: number };
  selectedLabel: string;
  onModeChange: (mode: TopologyMode) => void;
  onVisibilityChange: (kind: keyof TopologyVisibility) => void;
  onQueryChange: (query: string) => void;
  onFitSelected: () => void;
  focusActive: boolean;
  focusLabel: string | null;
  canIsolate: boolean;
  onIsolateSelected: () => void;
  onClearFocus: () => void;
}) {
  return (
    <aside className="flex h-full w-[232px] shrink-0 flex-col border-r border-border/35 bg-card/35">
      <div className="border-b border-border/25 px-4 py-4">
        <div className="text-[10px] font-bold uppercase text-muted-foreground">Topology</div>
        <div className="mt-1 text-[15px] font-semibold text-foreground">Attack Surface Map</div>
        <div className="mt-1 text-[11px] text-muted-foreground">
          {stats.organizations} orgs · {stats.targets} targets · {stats.evidence} evidence
        </div>
        <label className="relative mt-3 block">
          <Search className="absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground/55" />
          <input
            className="h-8 w-full rounded-md border border-border/35 bg-background/35 pl-7 pr-2 text-[11px] text-foreground outline-none transition-colors placeholder:text-muted-foreground/45 focus:border-cyan-400/45"
            placeholder="Filter graph..."
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
          />
        </label>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
        <SectionLabel icon={<Layers3 className="h-3 w-3" />} label="Graph mode" />
        <div className="mt-2 space-y-1.5">
          {MODE_OPTIONS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={cn(
                "flex h-9 w-full items-center gap-2 rounded-md border px-2 text-left text-[11px] transition-colors",
                mode === option.id
                  ? "border-cyan-400/25 bg-cyan-400/10 text-cyan-200"
                  : "border-border/30 bg-background/20 text-muted-foreground hover:bg-muted/20 hover:text-foreground"
              )}
              onClick={() => onModeChange(option.id)}
            >
              {option.icon}
              <span className="min-w-0 flex-1 font-medium">{option.label}</span>
              <span className="text-[10px] text-muted-foreground/70">{option.hint}</span>
            </button>
          ))}
        </div>

        <SectionLabel
          className="mt-6"
          icon={<Filter className="h-3 w-3" />}
          label="Visible types"
        />
        <div className="mt-2 grid grid-cols-2 gap-2">
          {VISIBILITY_OPTIONS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={cn(
                "flex h-8 items-center gap-1.5 rounded-md border px-2 text-[10px] transition-colors",
                visibility[option.id]
                  ? "border-border/40 bg-muted/25 text-foreground"
                  : "border-border/20 bg-background/15 text-muted-foreground/55"
              )}
              onClick={() => onVisibilityChange(option.id)}
            >
              <span className={cn("h-2 w-2 rounded-full", option.color)} />
              {option.label}
            </button>
          ))}
        </div>

        <SectionLabel className="mt-6" icon={<Crosshair className="h-3 w-3" />} label="Focus" />
        <div className="mt-2 rounded-lg border border-border/35 bg-background/20 p-3">
          <div className="truncate text-[12px] font-medium text-foreground">
            {focusActive ? (focusLabel ?? selectedLabel) : selectedLabel}
          </div>
          <div className="mt-1 text-[10px] text-muted-foreground">
            {focusActive ? "isolated · others hidden" : "selected node"}
          </div>
          {focusActive ? (
            <button
              type="button"
              className="mt-2.5 inline-flex h-7 w-full items-center justify-center gap-1.5 rounded-md border border-cyan-300/40 bg-cyan-300/10 text-[10px] font-semibold text-cyan-200 transition-colors hover:bg-cyan-300/15"
              onClick={onClearFocus}
            >
              <X className="h-3 w-3" />
              Exit focus (Esc)
            </button>
          ) : (
            <button
              type="button"
              disabled={!canIsolate}
              className="mt-2.5 inline-flex h-7 w-full items-center justify-center gap-1.5 rounded-md border border-border/40 bg-background/25 text-[10px] font-semibold text-muted-foreground transition-colors hover:border-cyan-300/40 hover:text-cyan-200 disabled:cursor-not-allowed disabled:opacity-40"
              onClick={onIsolateSelected}
            >
              <Focus className="h-3 w-3" />
              Isolate (F)
            </button>
          )}
        </div>
        <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground/70">
          Double-click a node or press F to isolate its org chain + subtree. Single-click dims
          unrelated nodes.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-2 border-t border-border/25 p-4">
        <button
          type="button"
          className="inline-flex h-8 items-center justify-center gap-1.5 rounded-md border border-cyan-400/20 bg-cyan-400/10 text-[10px] font-semibold text-cyan-200 transition-colors hover:bg-cyan-400/15"
          onClick={onFitSelected}
        >
          <Crosshair className="h-3 w-3" />
          Fit
        </button>
        <button
          type="button"
          className="inline-flex h-8 items-center justify-center gap-1.5 rounded-md border border-border/35 bg-background/20 text-[10px] font-semibold text-muted-foreground transition-colors hover:bg-muted/20 hover:text-foreground"
          onClick={() => onModeChange(mode)}
        >
          <RotateCcw className="h-3 w-3" />
          Layout
        </button>
      </div>
    </aside>
  );
}

function SectionLabel({
  icon,
  label,
  className,
}: {
  icon: React.ReactNode;
  label: string;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-1.5 text-[10px] font-bold uppercase text-muted-foreground",
        className
      )}
    >
      {icon}
      {label}
    </div>
  );
}
