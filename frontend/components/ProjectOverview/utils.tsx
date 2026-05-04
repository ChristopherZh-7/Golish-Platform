/* ── Pure helpers for activity-feed rendering ─────────────────────────
 *
 * Stateless formatters and icon factories. Kept in a `.tsx` file because
 * `itemIcon` returns JSX. Nothing in here owns React state — anything
 * stateful belongs in a hook or component.
 */

import { Activity, AlertTriangle, Bot, CheckCircle2, Play, Wrench } from "lucide-react";
import { formatDurationLong } from "@/lib/time";
import { type ItemKind, TOOL_DISPLAY } from "./types";

/** Map a backend tool/step name to its human-readable label, falling back
 *  to the underscored name with spaces. */
export function friendly(raw: string): string {
  return TOOL_DISPLAY[raw] ?? raw.replace(/_/g, " ");
}

export const fmtDur = (ms: number) => formatDurationLong(ms) || "0ms";

export function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function itemIcon(kind: ItemKind) {
  switch (kind) {
    case "tool_start":
      return <Wrench className="w-3 h-3 text-blue-400 animate-pulse" />;
    case "tool_done":
      return <CheckCircle2 className="w-3 h-3 text-green-400/70" />;
    case "tool_error":
      return <AlertTriangle className="w-3 h-3 text-red-400" />;
    case "agent_thinking":
      return <Bot className="w-3 h-3 text-purple-400 animate-pulse" />;
    case "agent_done":
      return <Bot className="w-3 h-3 text-green-400/70" />;
    case "sub_agent_start":
      return <Bot className="w-3 h-3 text-cyan-400 animate-pulse" />;
    case "sub_agent_done":
      return <Bot className="w-3 h-3 text-cyan-300/70" />;
    case "pipeline_start":
      return <Play className="w-3 h-3 text-blue-400" />;
    case "pipeline_done":
      return <CheckCircle2 className="w-3 h-3 text-green-400" />;
    case "pipeline_error":
      return <AlertTriangle className="w-3 h-3 text-red-400" />;
    case "info":
      return <Activity className="w-3 h-3 text-muted-foreground/40" />;
  }
}

export function itemColor(kind: ItemKind): string {
  if (kind.endsWith("_start") || kind === "agent_thinking") return "text-blue-300";
  if (kind.endsWith("_done")) return "text-foreground/50";
  if (kind.endsWith("_error")) return "text-red-300";
  return "text-muted-foreground/40";
}
