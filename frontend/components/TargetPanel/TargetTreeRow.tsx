/**
 * `TargetTreeRow` — renders a single target leaf inside the org tree sidebar.
 *
 * Extracted verbatim from `TargetGroupedView.tsx`'s `renderTarget`. Keeps the
 * row chrome (scope toggle, status badge, port count, delete) plus the inline
 * `TargetDetailView` editor when the row is expanded.
 */

import { Globe, Shield, ShieldOff, Trash2, Wifi } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import type { Target, TargetStatus } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import { TargetDetailView } from "./TargetDetail";
import { TYPE_ICONS } from "./targetTypeIcons";

// Stage-aligned target lifecycle (design 2026-06-14-target-status-stage-aligned):
// new < passive < active < enumerated < vuln_scan < verified. Colour ramps with
// progress so a glance down the list shows how far each target has gone.
const STATUS_CONFIG: Record<TargetStatus, { label: string; color: string; bg: string }> = {
  new: { label: "New", color: "text-gray-400", bg: "bg-gray-500/10" },
  passive: { label: "Passive", color: "text-blue-400", bg: "bg-blue-500/10" },
  active: { label: "Active", color: "text-cyan-400", bg: "bg-cyan-500/10" },
  enumerated: { label: "Enumerated", color: "text-violet-400", bg: "bg-violet-500/10" },
  vuln_scan: { label: "Vuln Scan", color: "text-yellow-400", bg: "bg-yellow-500/10" },
  verified: { label: "Verified", color: "text-green-400", bg: "bg-green-500/10" },
};

interface TargetTreeRowProps {
  target: Target;
  t: (key: string) => string;
  editingTargetId: string | null;
  selectedTargetId: string | null;
  setSelectedTargetId: Dispatch<SetStateAction<string | null>>;
  setSelectedOrgId: Dispatch<SetStateAction<string | null>>;
  setEditingTargetId: Dispatch<SetStateAction<string | null>>;
  onToggleScope: (target: Target) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
}

export function TargetTreeRow({
  target,
  t,
  editingTargetId,
  selectedTargetId,
  setSelectedTargetId,
  setSelectedOrgId,
  setEditingTargetId,
  onToggleScope,
  onDelete,
  onUpdateNotes,
}: TargetTreeRowProps) {
  const cfg = target.status ? STATUS_CONFIG[target.status] || STATUS_CONFIG.new : null;
  const isEditing = editingTargetId === target.id;
  const isSelected = selectedTargetId === target.id;
  return (
    <div
      key={target.id}
      className={cn(
        "border-l-2 border-transparent px-2 py-0.5 hover:bg-muted/20 transition-colors group cursor-pointer rounded-r",
        isEditing && "bg-muted/15",
        isSelected && "border-accent/60 bg-muted/25"
      )}
      onClick={() => {
        setSelectedTargetId(target.id);
        if (target.organization_id) setSelectedOrgId(target.organization_id);
        setEditingTargetId(null);
      }}
    >
      <div className="flex h-6 items-center gap-2">
        <span className="flex w-4 items-center justify-center opacity-90">
          {TYPE_ICONS[target.type] || <Globe className="w-3 h-3" />}
        </span>
        <button
          type="button"
          className={cn(
            "rounded p-0.5 transition-colors",
            target.scope === "in"
              ? "text-green-400/90 hover:text-green-300"
              : "text-red-400/75 hover:text-red-300"
          )}
          onClick={(e) => {
            e.stopPropagation();
            onToggleScope(target);
          }}
          title={target.scope === "in" ? t("targets.inScope") : t("targets.outOfScope")}
        >
          {target.scope === "in" ? (
            <Shield className="w-2.5 h-2.5" />
          ) : (
            <ShieldOff className="w-2.5 h-2.5" />
          )}
        </button>
        <span
          className={cn(
            "flex-1 truncate font-mono text-[11px]",
            target.scope === "out" && !isSelected ? "text-muted-foreground" : "text-foreground"
          )}
        >
          {target.value}
        </span>
        {cfg && target.status !== "new" && (
          <span className={cn("rounded px-1.5 py-0.5 text-[9px] font-medium", cfg.color, cfg.bg)}>
            {cfg.label}
          </span>
        )}
        {target.ports && target.ports.length > 0 && (
          <span
            className="flex min-w-8 items-center justify-end gap-0.5 text-[10px] text-emerald-400/75"
            title={`${target.ports.length} open port(s)`}
          >
            <Wifi className="w-2.5 h-2.5" />
            {target.ports.length}
          </span>
        )}
        <button
          type="button"
          className="rounded p-1 text-muted-foreground opacity-0 transition-all hover:bg-red-500/15 hover:text-red-400 group-hover:opacity-100"
          onClick={(e) => {
            e.stopPropagation();
            onDelete(target.id);
          }}
        >
          <Trash2 className="w-2.5 h-2.5" />
        </button>
      </div>
      {isEditing && <TargetDetailView target={target} t={t} onUpdateNotes={onUpdateNotes} />}
    </div>
  );
}
