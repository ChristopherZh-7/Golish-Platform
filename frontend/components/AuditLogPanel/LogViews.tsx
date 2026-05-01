import { useCallback, useEffect, useState } from "react";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { stripAllAnsi } from "@/lib/ansi";
import { SEV_BADGE } from "@/lib/severity";
import { formatLogDate } from "@/lib/time";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import {
  auditLogApi,
  type AgentLogEntry, type TerminalLogEntry, type SearchLogEntry,
  type PassiveScanEntry, type WikiChangeEntry,
} from "@/lib/audit-log";

export function AgentLogsView() {
  const [entries, setEntries] = useState<AgentLogEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const pp = getProjectPath() ?? "";
      const list = await auditLogApi.agentLogsList(pp, 200);
      setEntries(list ?? []);
    } catch { setEntries([]); }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const toggle = (id: string) => setExpanded(p => {
    const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n;
  });

  if (loading) return <div className="flex items-center justify-center py-12"><Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" /></div>;
  if (!entries.length) return <div className="text-center text-[11px] text-muted-foreground/30 py-12">No agent logs</div>;

  return (
    <div className="space-y-0.5">
      {entries.map(e => (
        <div key={e.id}>
          <div
            className="flex items-start gap-2 py-1.5 px-2 rounded cursor-pointer hover:bg-muted/10 transition-colors"
            onClick={() => toggle(e.id)}
          >
            {expanded.has(e.id) ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30 mt-0.5 flex-shrink-0" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20 mt-0.5 flex-shrink-0" />}
            <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap w-28 flex-shrink-0">{formatLogDate(e.createdAt)}</span>
            <span className="text-[8px] text-violet-400 bg-violet-500/10 px-1.5 py-0.5 rounded flex-shrink-0">{e.initiator}</span>
            <span className="text-[8px] text-muted-foreground/40 flex-shrink-0">&rarr;</span>
            <span className="text-[8px] text-blue-400 bg-blue-500/10 px-1.5 py-0.5 rounded flex-shrink-0">{e.executor}</span>
            <span className="text-[10px] text-foreground/70 flex-1 truncate">{e.task}</span>
            {e.durationMs != null && <span className="text-[8px] text-muted-foreground/30 flex-shrink-0">{e.durationMs}ms</span>}
          </div>
          {expanded.has(e.id) && (
            <div className="ml-8 mb-2 px-3 py-2 rounded-lg bg-[var(--bg-hover)]/20 border border-border/10 space-y-1">
              <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">Session:</span> {e.sessionId.slice(0, 12)}...</div>
              {e.taskId && <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">Task:</span> {e.taskId.slice(0, 12)}...</div>}
              {e.result && <pre className="text-[9px] text-foreground/50 font-mono whitespace-pre-wrap break-all max-h-48 overflow-y-auto mt-1">{e.result}</pre>}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

export function TerminalLogsView() {
  const [entries, setEntries] = useState<TerminalLogEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const pp = getProjectPath() ?? "";
      const list = await auditLogApi.terminalLogsList(pp, 200);
      setEntries(list ?? []);
    } catch { setEntries([]); }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const toggle = (id: string) => setExpanded(p => {
    const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n;
  });

  if (loading) return <div className="flex items-center justify-center py-12"><Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" /></div>;
  if (!entries.length) return <div className="text-center text-[11px] text-muted-foreground/30 py-12">No terminal logs</div>;

  return (
    <div className="space-y-0.5">
      {entries.map(e => {
        const clean = stripAllAnsi(e.content);
        return (
          <div key={e.id}>
            <div
              className="flex items-start gap-2 py-1.5 px-2 rounded cursor-pointer hover:bg-muted/10 transition-colors"
              onClick={() => toggle(e.id)}
            >
              {expanded.has(e.id) ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30 mt-0.5 flex-shrink-0" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20 mt-0.5 flex-shrink-0" />}
              <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap w-28 flex-shrink-0">{formatLogDate(e.createdAt)}</span>
              <span className={cn("text-[8px] px-1.5 py-0.5 rounded flex-shrink-0", e.stream === "stdout" ? "text-green-400 bg-green-500/10" : "text-red-400 bg-red-500/10")}>{e.stream}</span>
              <span className="text-[10px] text-foreground/70 flex-1 truncate font-mono">{clean.slice(0, 120)}</span>
            </div>
            {expanded.has(e.id) && (
              <div className="ml-8 mb-2 px-3 py-2 rounded-lg bg-[var(--bg-hover)]/20 border border-border/10 space-y-1">
                <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">Session:</span> {e.sessionId.slice(0, 12)}...</div>
                <pre className="text-[9px] text-foreground/50 font-mono whitespace-pre-wrap break-all max-h-64 overflow-y-auto">{clean}</pre>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function SearchLogsView() {
  const [entries, setEntries] = useState<SearchLogEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const pp = getProjectPath() ?? "";
      const list = await auditLogApi.searchLogsList(pp, 200);
      setEntries(list ?? []);
    } catch { setEntries([]); }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const toggle = (id: string) => setExpanded(p => {
    const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n;
  });

  if (loading) return <div className="flex items-center justify-center py-12"><Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" /></div>;
  if (!entries.length) return <div className="text-center text-[11px] text-muted-foreground/30 py-12">No search logs</div>;

  return (
    <div className="space-y-0.5">
      {entries.map(e => (
        <div key={e.id}>
          <div
            className="flex items-start gap-2 py-1.5 px-2 rounded cursor-pointer hover:bg-muted/10 transition-colors"
            onClick={() => toggle(e.id)}
          >
            {expanded.has(e.id) ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30 mt-0.5 flex-shrink-0" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20 mt-0.5 flex-shrink-0" />}
            <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap w-28 flex-shrink-0">{formatLogDate(e.createdAt)}</span>
            <span className="text-[8px] text-cyan-400 bg-cyan-500/10 px-1.5 py-0.5 rounded flex-shrink-0">{e.engine}</span>
            {e.initiator && <span className="text-[8px] text-violet-400 bg-violet-500/10 px-1.5 py-0.5 rounded flex-shrink-0">{e.initiator}</span>}
            <span className="text-[10px] text-foreground/70 flex-1 truncate">{e.query}</span>
          </div>
          {expanded.has(e.id) && e.result && (
            <div className="ml-8 mb-2 px-3 py-2 rounded-lg bg-[var(--bg-hover)]/20 border border-border/10">
              <pre className="text-[9px] text-foreground/50 font-mono whitespace-pre-wrap break-all max-h-48 overflow-y-auto">{e.result}</pre>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

export function PassiveScanLogsView() {
  const [entries, setEntries] = useState<PassiveScanEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const pp = getProjectPath() ?? "";
      const list = await auditLogApi.passiveScansGlobal(pp, 200);
      setEntries(list ?? []);
    } catch { setEntries([]); }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const toggle = (id: string) => setExpanded(p => {
    const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n;
  });

  const sevColor = (s: string) => SEV_BADGE[s] || "text-muted-foreground/50 bg-muted/5";

  if (loading) return <div className="flex items-center justify-center py-12"><Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" /></div>;
  if (!entries.length) return <div className="text-center text-[11px] text-muted-foreground/30 py-12">No passive scan logs</div>;

  return (
    <div className="space-y-0.5">
      {entries.map(e => (
        <div key={e.id}>
          <div
            className="flex items-start gap-2 py-1.5 px-2 rounded cursor-pointer hover:bg-muted/10 transition-colors"
            onClick={() => toggle(e.id)}
          >
            {expanded.has(e.id) ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30 mt-0.5 flex-shrink-0" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20 mt-0.5 flex-shrink-0" />}
            <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap w-28 flex-shrink-0">{formatLogDate(e.testedAt)}</span>
            <span className={cn("text-[8px] px-1.5 py-0.5 rounded flex-shrink-0", sevColor(e.severity))}>{e.severity}</span>
            <span className="text-[8px] text-emerald-400 bg-emerald-500/10 px-1.5 py-0.5 rounded flex-shrink-0">{e.testType}</span>
            <span className="text-[10px] text-foreground/70 flex-1 truncate">{e.url || e.result}</span>
            {e.toolUsed && <span className="text-[8px] text-muted-foreground/30 bg-muted/5 px-1.5 py-0.5 rounded flex-shrink-0 font-mono">{e.toolUsed}</span>}
          </div>
          {expanded.has(e.id) && (
            <div className="ml-8 mb-2 px-3 py-2 rounded-lg bg-[var(--bg-hover)]/20 border border-border/10 space-y-1">
              <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">Target:</span> {e.targetId.slice(0, 12)}...</div>
              {e.url && <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">URL:</span> {e.url}</div>}
              <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">Result:</span> {e.result}</div>
              {e.payload && <div className="text-[9px] text-muted-foreground/30"><span className="text-muted-foreground/50">Payload:</span> <code className="font-mono">{e.payload}</code></div>}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

export function WikiChangelogsView() {
  const [entries, setEntries] = useState<WikiChangeEntry[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list = await auditLogApi.wikiChangelogList(200);
      setEntries(list ?? []);
    } catch { setEntries([]); }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  if (loading) return <div className="flex items-center justify-center py-12"><Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" /></div>;
  if (!entries.length) return <div className="text-center text-[11px] text-muted-foreground/30 py-12">No wiki changes</div>;

  return (
    <div className="space-y-0.5">
      {entries.map(e => (
        <div key={e.id} className="flex items-start gap-2 py-1.5 px-2 rounded hover:bg-muted/5 transition-colors">
          <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap w-28 flex-shrink-0">{formatLogDate(e.createdAt)}</span>
          <span className={cn("text-[8px] px-1.5 py-0.5 rounded flex-shrink-0", e.action === "create" ? "text-green-400 bg-green-500/10" : e.action === "delete" ? "text-red-400 bg-red-500/10" : "text-yellow-400 bg-yellow-500/10")}>{e.action}</span>
          {e.category && <span className="text-[8px] text-muted-foreground/40 bg-muted/5 px-1.5 py-0.5 rounded flex-shrink-0">{e.category}</span>}
          <span className="text-[10px] text-foreground/70 flex-1 truncate">{e.title || e.pagePath}</span>
          {e.actor && <span className="text-[8px] text-muted-foreground/30 flex-shrink-0">{e.actor}</span>}
        </div>
      ))}
    </div>
  );
}
