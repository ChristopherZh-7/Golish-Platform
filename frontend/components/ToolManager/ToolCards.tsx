import { Check, Download, Loader2, Trash2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { ToolWithMeta } from "./OutputParserEditor";

export function CircleProgress({ size = 28, pct }: { size?: number; pct: number }) {
  const r = (size - 4) / 2;
  const circ = 2 * Math.PI * r;
  const offset = circ * (1 - Math.min(Math.max(pct, 0), 1));
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="flex-shrink-0">
      <circle cx={size / 2} cy={size / 2} r={r} fill="none"
        stroke="currentColor" strokeWidth="2.5" className="text-muted-foreground/10" />
      <circle cx={size / 2} cy={size / 2} r={r} fill="none"
        stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"
        strokeDasharray={`${circ}`} strokeDashoffset={`${offset}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
        className="text-accent transition-[stroke-dashoffset] duration-200" />
    </svg>
  );
}

export function runtimeBadge(runtime: string) {
  const m: Record<string, string> = {
    java: "bg-orange-500/15 text-orange-400",
    python: "bg-blue-500/15 text-blue-400",
    node: "bg-green-500/15 text-green-400",
    native: "bg-zinc-500/15 text-zinc-400",
  };
  return m[runtime] || "bg-muted/50 text-muted-foreground";
}

export function installMethodBadge(method: string) {
  const m: Record<string, string> = {
    github: "bg-purple-500/12 text-purple-400",
    homebrew: "bg-amber-500/12 text-amber-400",
  };
  return m[method] || "bg-muted/30 text-muted-foreground/50";
}

export function getInstallMethodLabel(tool: ToolWithMeta, t: (key: string) => string) {
  const method = tool.install?.method;
  if (!method || method === "manual") return t("toolManager.manual");
  if (method === "github") return "GitHub";
  if (method === "homebrew") return "Homebrew";
  if (method === "gem") return "RubyGem";
  return method;
}

export function TagBadges({ tool, compact }: { tool: ToolWithMeta; compact?: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-1.5 flex-wrap">
      <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", runtimeBadge(tool.runtime))}>
        {tool.runtime}{tool.runtimeVersion ? ` ${tool.runtimeVersion}` : ""}
      </span>
      {tool.install?.method && tool.install.method !== "manual" && (
        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full", installMethodBadge(tool.install.method))}>
          {getInstallMethodLabel(tool, t)}
        </span>
      )}
      {(tool.tier === "essential" || tool.tier === "recommended") && (
        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium",
          tool.tier === "essential" ? "bg-red-500/10 text-red-400" : "bg-amber-500/10 text-amber-400")}>
          {tool.tier === "essential" ? t("toolManager.tierEssential") : t("toolManager.tierRecommended")}
        </span>
      )}
      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/30 text-muted-foreground/50">
        {tool.categoryName}
      </span>
      {!compact && tool.subcategoryName && (
        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/30 text-muted-foreground/50">
          {tool.subcategoryName}
        </span>
      )}
    </div>
  );
}

export interface ActionButtonProps {
  busy: string | null;
  installProgress: Record<string, string>;
  dlProgress: { downloaded: number; total: number } | null;
  onCancel: () => void;
  onUninstall: (tool: ToolWithMeta) => void;
  onInstall: (tool: ToolWithMeta) => void;
}

export function ActionButton({ tool, ...ctx }: ActionButtonProps & { tool: ToolWithMeta }) {
  const { t } = useTranslation();
  const isBusy = ctx.busy === tool.id;
  const progress = ctx.installProgress[tool.id];
  const hasDlProgress = isBusy && ctx.dlProgress && ctx.dlProgress.total > 0;
  const dlPct = hasDlProgress ? ctx.dlProgress!.downloaded / ctx.dlProgress!.total : 0;
  return (
    <div className="flex items-center gap-1 flex-shrink-0" onClick={(e) => e.stopPropagation()}>
      {isBusy ? (
        <div className="flex items-center gap-1.5">
          {hasDlProgress ? (
            <CircleProgress pct={dlPct} size={22} />
          ) : (
            <Loader2 className="w-3.5 h-3.5 animate-spin text-accent/60" />
          )}
          {progress && <span className="text-[10px] text-accent/50 whitespace-nowrap max-w-[80px] truncate">{progress}</span>}
          <button type="button" onClick={ctx.onCancel}
            className="p-0.5 rounded text-muted-foreground/40 hover:text-destructive transition-colors" title={t("common.cancel")}>
            <X className="w-3 h-3" />
          </button>
        </div>
      ) : tool.installed ? (
        <button type="button" onClick={() => ctx.onUninstall(tool)}
          className="p-1 rounded text-muted-foreground/30 opacity-0 group-hover:opacity-100 hover:text-destructive transition-all" title={t("common.uninstall")}>
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      ) : (
        <button type="button" onClick={() => ctx.onInstall(tool)}
          className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
          title={t("toolManager.installVia", { method: getInstallMethodLabel(tool, t) })}>
          <Download className="w-3 h-3" /> {t("common.install")}
        </button>
      )}
    </div>
  );
}

interface ToolCardProps {
  tool: ToolWithMeta;
  onOpen: (tool: ToolWithMeta) => void;
  onContextMenu: (e: React.MouseEvent, tool: ToolWithMeta) => void;
  actionCtx: ActionButtonProps;
}

export function GridCard({ tool, onOpen, onContextMenu, actionCtx }: ToolCardProps) {
  const { t } = useTranslation();
  return (
    <div
      onClick={() => onOpen(tool)}
      onContextMenu={(e) => onContextMenu(e, tool)}
      className={cn(
        "group rounded-xl border transition-colors cursor-pointer p-4",
        tool.installed
          ? "border-border/15 bg-[var(--bg-hover)]/20 hover:bg-[var(--bg-hover)]/40"
          : "border-border/10 bg-[var(--bg-hover)]/8 hover:bg-[var(--bg-hover)]/20"
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {tool.icon && <span className="text-[14px] flex-shrink-0">{tool.icon}</span>}
            <span className={cn("text-[13px] font-medium truncate", tool.installed ? "text-foreground" : "text-foreground/60")}>{tool.name}</span>
            {tool.installed && (
              <span className="text-[9px] px-1.5 py-px rounded-full bg-green-500/15 text-green-400 flex-shrink-0 flex items-center gap-0.5">
                <Check className="w-2.5 h-2.5" /> {t("common.installed")}
              </span>
            )}
          </div>
          <p className={cn("text-[11px] mt-1 line-clamp-2",
            tool.installed ? "text-muted-foreground/60" : "text-muted-foreground/50"
          )}>{tool.description}</p>
        </div>
        <ActionButton tool={tool} {...actionCtx} />
      </div>
      <div className="mt-3"><TagBadges tool={tool} /></div>
    </div>
  );
}

export function ListRow({ tool, onOpen, onContextMenu, actionCtx }: ToolCardProps) {
  const { t } = useTranslation();
  return (
    <div
      onClick={() => onOpen(tool)}
      onContextMenu={(e) => onContextMenu(e, tool)}
      className={cn(
        "group rounded-xl border transition-colors cursor-pointer px-4 py-2.5 flex items-center",
        tool.installed
          ? "border-border/15 bg-[var(--bg-hover)]/20 hover:bg-[var(--bg-hover)]/40"
          : "border-border/10 bg-[var(--bg-hover)]/8 hover:bg-[var(--bg-hover)]/20"
      )}
    >
      <div className="flex items-center gap-2 w-[200px] flex-shrink-0">
        {tool.icon && <span className="text-[14px] flex-shrink-0">{tool.icon}</span>}
        <span className={cn("text-[13px] font-medium truncate", tool.installed ? "text-foreground" : "text-foreground/60")}>{tool.name}</span>
        {tool.installed && (
          <span className="text-[9px] px-1.5 py-px rounded-full bg-green-500/15 text-green-400 flex-shrink-0 flex items-center gap-0.5">
            <Check className="w-2.5 h-2.5" /> {t("common.installed")}
          </span>
        )}
      </div>
      <p className="flex-1 min-w-0 text-[11px] text-muted-foreground/60 truncate px-4">{tool.description}</p>
      <div className="flex-shrink-0 mr-3">
        <TagBadges tool={tool} compact />
      </div>
      <div className="w-[70px] flex justify-end flex-shrink-0">
        <ActionButton tool={tool} {...actionCtx} />
      </div>
    </div>
  );
}
