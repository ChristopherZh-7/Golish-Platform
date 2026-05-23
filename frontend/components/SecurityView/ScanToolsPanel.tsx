import { Crosshair, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { PipelineLauncher } from "@/components/TargetPanel/PipelineLauncher";
import { getProjectPath } from "@/lib/projects";
import { securityApi } from "@/lib/security";
import { NucleiSection } from "./NucleiSection";
import { ScanTimeline } from "./ScanTimeline";
import { StyledSelect } from "./shared";

interface TargetOption {
  id: string;
  value: string;
  type: string;
}

export function ScanToolsPanel({
  initialTarget,
}: {
  initialTarget?: { id: string; value: string };
}) {
  const [targets, setTargets] = useState<TargetOption[]>([]);
  const [selectedTarget, setSelectedTarget] = useState<TargetOption | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setSelectedTarget(null);
    (async () => {
      try {
        const data = (await securityApi.targetList(getProjectPath())) as unknown as {
          targets: TargetOption[];
        };
        if (cancelled) return;
        const scannable = (data?.targets ?? []).filter(
          (t) => t.type === "url" || t.type === "domain" || t.type === "ip"
        );
        setTargets(scannable);
        const initial = initialTarget ? scannable.find((t) => t.id === initialTarget.id) : null;
        if (initial) {
          setSelectedTarget(initial);
        } else if (scannable.length > 0) {
          setSelectedTarget(scannable[0]);
        }
      } catch {
        if (!cancelled) setTargets([]);
      }
      if (!cancelled) setLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [initialTarget?.id, initialTarget]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/20" />
      </div>
    );
  }

  if (targets.length === 0) {
    return (
      <div className="h-full flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
        <Crosshair className="w-12 h-12" />
        <p className="text-[12px] font-medium">No scannable targets</p>
        <p className="text-[10px] text-muted-foreground/15 max-w-[280px] text-center">
          Add URL or domain targets in the Targets panel first. Scan Tools supports WhatWeb
          fingerprinting, targeted Nuclei scanning, and feroxbuster directory brute-forcing.
        </p>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      {/* Target selector */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Crosshair className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[10px] text-muted-foreground/50 flex-shrink-0">Target:</span>
        <StyledSelect
          value={selectedTarget?.id ?? ""}
          onChange={(v) => {
            const t = targets.find((t) => t.id === v);
            if (t) setSelectedTarget(t);
          }}
          options={targets.map((t) => ({ value: t.id, label: `[${t.type}] ${t.value}` }))}
          className="flex-1"
        />
        <span className="text-[9px] text-muted-foreground/30">
          {targets.length} target{targets.length !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Pipeline launcher + Nuclei + scan history */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        {selectedTarget && (
          <>
            <PipelineLauncher
              key={selectedTarget.id}
              targetId={selectedTarget.id}
              targetValue={selectedTarget.value}
            />
            <NucleiSection targetId={selectedTarget.id} targetUrl={selectedTarget.value} />
            <ScanTimeline targetId={selectedTarget.id} targetValue={selectedTarget.value} />
          </>
        )}
      </div>
    </div>
  );
}

// ── Sensitive File Scanner Panel ──
