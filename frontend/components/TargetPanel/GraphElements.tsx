import { useMemo, useState } from "react";
import {
  Bug, ChevronRight, Crosshair, Globe, Hash,
  Network, Search, Server, Wifi, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getRootDomain } from "@/lib/domain";
import { SEV_BADGE, SEV_SHORT_LABELS } from "@/lib/severity";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";
import { type Target } from "@/lib/pentest/types";

// ── Sidebar target list ──

interface GraphSidebarProps {
  targets: Target[];
  selectedTarget: Target | null;
  onFocusNode: (targetId: string) => void;
}

export function GraphSidebar({ targets, selectedTarget, onFocusNode }: GraphSidebarProps) {
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [searchFilter, setSearchFilter] = useState("");

  const rootTargets = useMemo(() => targets.filter((t) => !t.parent_id), [targets]);

  const childMap = useMemo(() => {
    const map = new Map<string, Target[]>();
    for (const t of targets) {
      if (t.parent_id) {
        const arr = map.get(t.parent_id) || [];
        arr.push(t);
        map.set(t.parent_id, arr);
      }
    }
    return map;
  }, [targets]);

  const domainGroups = useMemo(() => {
    const map = new Map<string, Target[]>();
    for (const t of rootTargets) {
      const domain = getRootDomain(t.value);
      const arr = map.get(domain) || [];
      arr.push(t);
      map.set(domain, arr);
    }
    return [...map.entries()].sort((a, b) => b[1].length - a[1].length);
  }, [rootTargets]);

  const filteredGroups = useMemo(() => {
    if (!searchFilter) return domainGroups;
    const q = searchFilter.toLowerCase();
    return domainGroups
      .map(([domain, items]) => {
        const filtered = items.filter((t) =>
          t.value.toLowerCase().includes(q) ||
          t.name.toLowerCase().includes(q) ||
          childMap.get(t.id)?.some((c) => c.value.toLowerCase().includes(q))
        );
        return [domain, filtered] as [string, Target[]];
      })
      .filter(([, items]) => items.length > 0);
  }, [domainGroups, searchFilter, childMap]);

  return (
    <div className="w-52 border-r border-border/30 flex flex-col flex-shrink-0 bg-card/50">
      <div className="px-2 py-2 border-b border-border/20">
        <div className="relative">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-muted-foreground/40" />
          <input
            className="w-full pl-7 pr-2 py-1 text-[11px] bg-background/50 border border-border/30 rounded focus:border-accent/50 outline-none"
            placeholder="Filter targets..."
            value={searchFilter}
            onChange={(e) => setSearchFilter(e.target.value)}
          />
        </div>
      </div>
      <div className="flex-1 overflow-y-auto px-1 py-1">
        {filteredGroups.map(([domain, items]) => {
          const isMulti = items.length > 1;
          const isExpanded = expandedGroups.has(domain) || !!searchFilter;

          return (
            <div key={domain}>
              {isMulti && (
                <button
                  type="button"
                  className="flex items-center gap-1 px-2 py-1.5 w-full rounded-md hover:bg-muted/30 text-left text-[11px]"
                  onClick={() => setExpandedGroups((prev) => {
                    const next = new Set(prev);
                    if (next.has(domain)) next.delete(domain); else next.add(domain);
                    return next;
                  })}
                >
                  <ChevronRight className={cn("w-3 h-3 text-muted-foreground/40 transition-transform", isExpanded && "rotate-90")} />
                  <Globe className="w-3 h-3 text-blue-400/70 flex-shrink-0" />
                  <span className="truncate flex-1 font-medium text-foreground/80">{domain}</span>
                  <span className="text-[9px] text-muted-foreground/40">{items.length}</span>
                </button>
              )}

              {(isExpanded || !isMulti) && items.map((target) => {
                const isSelected = selectedTarget?.id === target.id;
                const children = childMap.get(target.id) || [];
                return (
                  <div key={target.id} className={isMulti ? "pl-3" : ""}>
                    <button
                      type="button"
                      className={cn(
                        "flex items-center gap-1.5 px-2 py-1 w-full rounded-md text-left text-[10px] transition-colors",
                        isSelected ? "bg-accent/15 text-accent" : "hover:bg-muted/20 text-muted-foreground hover:text-foreground",
                        target.scope === "out" && "opacity-50",
                      )}
                      onClick={() => onFocusNode(target.id)}
                    >
                      <span className={cn(
                        "w-1.5 h-1.5 rounded-full flex-shrink-0",
                        target.scope === "in" ? "bg-emerald-400" : "bg-zinc-500",
                      )} />
                      <span className="truncate flex-1 font-mono">{target.value}</span>
                      {target.ports?.length > 0 && (
                        <span className="text-[9px] text-emerald-400/50">{target.ports.length}p</span>
                      )}
                    </button>
                    {children.length > 0 && (isExpanded || !isMulti) && (
                      <div className="pl-4">
                        {children.slice(0, 8).map((child) => (
                          <button
                            key={child.id}
                            type="button"
                            className={cn(
                              "flex items-center gap-1 px-1.5 py-0.5 w-full rounded text-left text-[9px] transition-colors",
                              selectedTarget?.id === child.id ? "text-accent" : "text-muted-foreground/50 hover:text-muted-foreground",
                            )}
                            onClick={() => onFocusNode(child.id)}
                          >
                            <span className="truncate flex-1 font-mono">{child.value}</span>
                          </button>
                        ))}
                        {children.length > 8 && (
                          <span className="px-1.5 text-[8px] text-muted-foreground/30">+{children.length - 8} more</span>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
      <div className="px-2 py-1.5 border-t border-border/20 text-[10px] text-muted-foreground/40 text-center">
        {targets.length} targets
      </div>
    </div>
  );
}

// ── Selected target detail aside panel ──

interface GraphNodeDetailProps {
  target: Target;
  targets: Target[];
  findings: { id: string; title: string; severity: string; status: string }[];
  onClose: () => void;
  onNavigate: (child: Target) => void;
}

export function GraphNodeDetail({ target, targets, findings, onClose, onNavigate }: GraphNodeDetailProps) {
  const children = useMemo(() => targets.filter((t) => t.parent_id === target.id), [targets, target.id]);

  return (
    <aside className={cn(
      "absolute top-3 right-3 z-10 flex max-h-[min(28rem,calc(100%-1.5rem))] w-[min(19rem,calc(100%-1.5rem))] flex-col overflow-hidden rounded-xl border border-border/60",
      "bg-popover/95 shadow-xl shadow-black/10 backdrop-blur-md",
      "ring-1 ring-black/[0.04] dark:ring-white/[0.06]",
    )}>
      <div className="flex items-start gap-2.5 border-b border-border/50 bg-muted/25 px-3 py-2.5">
        <div className={cn(
          "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg",
          target.scope === "out" ? "bg-muted text-muted-foreground" : "bg-accent/15 text-accent",
        )}>
          {target.type === "domain" ? <Globe className="h-4 w-4" /> :
           target.type === "ip" ? <Hash className="h-4 w-4" /> :
           target.type === "cidr" ? <Network className="h-4 w-4" /> :
           target.type === "wildcard" ? <Crosshair className="h-4 w-4" /> :
           <Server className="h-4 w-4" />}
        </div>
        <div className="min-w-0 flex-1 pt-0.5">
          <h3 className="truncate text-sm font-semibold leading-snug tracking-tight text-foreground font-mono">
            {target.value}
          </h3>
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            <span className={cn(
              "rounded-md border px-1.5 py-0.5 text-[10px] font-medium",
              target.scope === "in"
                ? "border-emerald-500/20 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                : "border-red-500/20 bg-red-500/10 text-red-600 dark:text-red-400",
            )}>
              {target.scope === "in" ? "In Scope" : "Out of Scope"}
            </span>
            <span className="rounded-md border border-border/50 bg-muted/50 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
              {target.type}
            </span>
          </div>
        </div>
        <button
          type="button"
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted/80 hover:text-foreground"
          onClick={onClose}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 space-y-3">
        {(target.real_ip || target.os_info || target.webserver || target.cdn_waf) && (
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-xs">
            {target.real_ip && (
              <><dt className="text-muted-foreground">IP</dt><dd className="font-mono text-foreground tabular-nums">{target.real_ip}</dd></>
            )}
            {target.os_info && (
              <><dt className="text-muted-foreground">OS</dt><dd className="text-foreground">{target.os_info}</dd></>
            )}
            {target.webserver && (
              <><dt className="text-muted-foreground">Server</dt><dd className="text-foreground">{target.webserver}</dd></>
            )}
            {target.cdn_waf && (
              <><dt className="text-muted-foreground">CDN/WAF</dt><dd className="text-foreground">{target.cdn_waf}</dd></>
            )}
          </dl>
        )}

        {target.ports?.length > 0 && (
          <div>
            <div className="mb-2 flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-foreground flex items-center gap-1">
                <Wifi className="w-3 h-3 text-emerald-400" />
                Ports
              </span>
              <span className="rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                {target.ports.length}
              </span>
            </div>
            <ul className="space-y-1">
              {target.ports.map((p) => {
                const dot =
                  p.state === "open" ? "bg-emerald-400 ring-1 ring-emerald-500/40" :
                  p.state === "filtered" ? "bg-amber-400 ring-1 ring-amber-500/40" :
                  "bg-rose-400 ring-1 ring-rose-500/40";
                return (
                  <li key={`${p.port}-${p.protocol}`} className="flex items-center gap-2 rounded-lg border border-border/40 bg-muted/15 px-2 py-1.5 text-xs">
                    <span className={cn("h-2 w-2 shrink-0 rounded-full", dot)} />
                    <span className="w-[4.25rem] shrink-0 font-mono tabular-nums text-foreground">{p.port}/{p.protocol}</span>
                    <span className="min-w-0 truncate text-muted-foreground">{p.service || "\u2014"}</span>
                    {p.http_status != null && (
                      <span className={cn("text-[10px] font-mono", p.http_status < 400 ? "text-green-400" : "text-red-400")}>
                        [{p.http_status}]
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
          </div>
        )}

        {children.length > 0 && (
          <div className="border-t border-border/40 pt-3">
            <div className="mb-2 flex items-center justify-between text-xs font-medium text-foreground">
              <span className="flex items-center gap-1">
                <Network className="w-3 h-3 text-accent/60" />
                Subdomains
              </span>
              <span className="rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                {children.length}
              </span>
            </div>
            <ul className="space-y-1">
              {children.map((child) => (
                <li
                  key={child.id}
                  className="flex items-center gap-2 rounded-lg border border-border/40 bg-muted/10 px-2 py-1.5 text-xs cursor-pointer hover:bg-muted/20 transition-colors"
                  onClick={() => onNavigate(child)}
                >
                  <Globe className="w-3 h-3 text-blue-400 flex-shrink-0" />
                  <span className="font-mono text-foreground/70 flex-1 truncate">{child.value}</span>
                  {child.ports?.length > 0 && (
                    <span className="flex items-center gap-0.5 text-[10px] text-emerald-400/60">
                      <Wifi className="w-2.5 h-2.5" />{child.ports.length}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}

        {findings.length > 0 && (
          <div className="border-t border-border/40 pt-3">
            <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-foreground">
              <Bug className="h-3.5 w-3.5 text-muted-foreground" />
              Findings
              <span className="ml-auto rounded-full border border-border/50 bg-muted/40 px-2 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground">
                {findings.length}
              </span>
            </div>
            <ul className="space-y-1">
              {findings.map((f) => (
                <li key={f.id} className="flex items-start gap-2 rounded-lg border border-border/40 bg-muted/10 px-2 py-1.5">
                  <span className={cn(
                    "mt-0.5 shrink-0 rounded border px-1 py-px text-[9px] font-semibold uppercase leading-none",
                    SEV_BADGE[f.severity] || SEV_BADGE.info,
                  )}>
                    {SEV_SHORT_LABELS[f.severity] || SEV_SHORT_LABELS.info}
                  </span>
                  <span className="min-w-0 flex-1 text-[11px] leading-snug text-foreground">{f.title}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="border-t border-border/40 pt-3">
          <p className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/80">Notes</p>
          <QuickNotes entityType="target" entityId={target.id} compact />
        </div>
      </div>
    </aside>
  );
}
