import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Bot, Loader2, MessageSquare, Trash2, Zap } from "lucide-react";
import { cn } from "@/lib/utils";
import { Markdown } from "@/components/Markdown";
import { respondToToolApproval, setAgentMode } from "@/lib/ai";
import { useStore } from "@/store";

type TurnBlock =
  | { type: "text"; content: string }
  | { type: "tool"; id: string; name: string; status: string };

interface CompletedTurn {
  id: string;
  blocks: TurnBlock[];
  /** @deprecated kept for backward compat with old DB records */
  text?: string;
  /** @deprecated kept for backward compat with old DB records */
  toolCalls?: Array<{ id: string; name: string; status: string }>;
}

const EMPTY_STREAMING: never[] = [];

export function ResearchTab({ sessionId, cveId }: { sessionId: string | null; cveId: string }) {
  const sid = sessionId ?? "";
  const isResponding = useStore((s) => sid ? (s.isAgentResponding[sid] ?? false) : false);
  const streamingBlocks = useStore((s) => {
    if (!sid) return EMPTY_STREAMING;
    return s.streamingBlocks[sid] ?? EMPTY_STREAMING;
  });
  const isThinking = useStore((s) => sid ? (s.isAgentThinking[sid] ?? false) : false);
  const storeApproval = useStore((s) => sid ? (s.pendingToolApproval[sid] ?? null) : null);
  const storeAskHuman = useStore((s) => sid ? (s.pendingAskHuman[sid] ?? null) : null);
  const [dismissedApprovalId, setDismissedApprovalId] = useState<string | null>(null);
  const [dismissedAskHumanId, setDismissedAskHumanId] = useState<string | null>(null);
  const agentMode = useStore((s) => sid ? (s.sessions[sid]?.agentMode ?? "default") : "default");
  const isAutoApprove = agentMode === "auto-approve";
  const pendingApproval = isResponding && !isAutoApprove && storeApproval && storeApproval.id !== dismissedApprovalId ? storeApproval : null;
  const pendingAskHuman = isResponding && storeAskHuman && storeAskHuman.requestId !== dismissedAskHumanId ? storeAskHuman : null;
  const scrollRef = useRef<HTMLDivElement>(null);
  const [askHumanInput, setAskHumanInput] = useState("");

  const [completedTurns, setCompletedTurns] = useState<CompletedTurn[]>([]);
  const [loadedFromDb, setLoadedFromDb] = useState(false);
  const prevBlocksRef = useRef<typeof streamingBlocks>([]);

  // Load previous research conversation from DB on mount
  useEffect(() => {
    invoke<{ turns: Array<Record<string, unknown>>; status: string } | null>("kb_research_load", { cveId })
      .then((log) => {
        if (log?.turns && Array.isArray(log.turns) && log.turns.length > 0) {
          const migrated: CompletedTurn[] = log.turns.map((raw) => {
            if (Array.isArray(raw.blocks)) return raw as unknown as CompletedTurn;
            // Migrate old format: { text, toolCalls } -> { blocks }
            const blocks: TurnBlock[] = [];
            const oldTools = Array.isArray(raw.toolCalls) ? raw.toolCalls as Array<{ id: string; name: string; status: string }> : [];
            for (const tc of oldTools) blocks.push({ type: "tool", ...tc });
            if (typeof raw.text === "string" && raw.text) blocks.push({ type: "text", content: raw.text });
            return { id: (raw.id as string) || crypto.randomUUID(), blocks };
          });
          setCompletedTurns(migrated);
        }
      })
      .catch((e) => console.error("Failed to load research log:", e))
      .finally(() => setLoadedFromDb(true));
  }, [cveId]);

  // Capture completed turns preserving block order and persist to DB
  useEffect(() => {
    const hadBlocks = prevBlocksRef.current.length > 0;
    const nowEmpty = streamingBlocks.length === 0;

    if (hadBlocks && nowEmpty) {
      const blocks: TurnBlock[] = [];
      let textAcc = "";
      for (const b of prevBlocksRef.current) {
        if (b.type === "text") {
          textAcc += b.content;
        } else if (b.type === "tool") {
          if (textAcc) { blocks.push({ type: "text", content: textAcc }); textAcc = ""; }
          blocks.push({ type: "tool", id: b.toolCall.id, name: b.toolCall.name, status: b.toolCall.status });
        }
      }
      if (textAcc) blocks.push({ type: "text", content: textAcc });

      if (blocks.length > 0) {
        const turn: CompletedTurn = { id: crypto.randomUUID(), blocks };
        setCompletedTurns((prev) => [...prev, turn]);
        invoke("kb_research_save_turn", { cveId, sessionId: sid, turn }).catch((e) =>
          console.error("Failed to save research turn:", e)
        );
      }
    }
    prevBlocksRef.current = streamingBlocks;
  }, [streamingBlocks, cveId, sid]);

  // Mark research as completed when agent finishes and we have content
  const prevRespondingRef = useRef(false);
  useEffect(() => {
    if (prevRespondingRef.current && !isResponding && completedTurns.length > 0) {
      invoke("kb_research_set_status", { cveId, status: "completed" }).catch((e) =>
        console.error("Failed to set research status:", e)
      );
    }
    prevRespondingRef.current = isResponding;
  }, [isResponding, completedTurns.length, cveId]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [completedTurns, streamingBlocks, isResponding, pendingApproval, pendingAskHuman]);

  const isEmpty = loadedFromDb && completedTurns.length === 0 && streamingBlocks.length === 0 && !isResponding;
  const [clearing, setClearing] = useState(false);

  const handleClearHistory = useCallback(async () => {
    if (!confirm("Delete all research history for this CVE? This cannot be undone.")) return;
    setClearing(true);
    try {
      await invoke("kb_research_clear", { cveId });
      setCompletedTurns([]);
    } catch (e) {
      console.error("Failed to clear research history:", e);
    }
    setClearing(false);
  }, [cveId]);

  const proseClasses = "text-[11px] leading-relaxed text-foreground/80 prose prose-invert prose-sm max-w-none prose-headings:text-foreground/90 prose-headings:text-[12px] prose-headings:font-semibold prose-p:text-[11px] prose-p:leading-relaxed prose-code:text-[10px] prose-code:bg-muted/20 prose-code:px-1 prose-code:rounded prose-pre:bg-muted/10 prose-pre:border prose-pre:border-border/10 prose-pre:text-[10px] prose-li:text-[11px] prose-a:text-accent";

  return (
    <div ref={scrollRef} className="space-y-3 -mx-4 -my-3 px-4 py-3 overflow-y-auto max-h-full">
      {/* Header with clear button */}
      {completedTurns.length > 0 && !isResponding && (
        <div className="flex items-center justify-end">
          <button
            onClick={handleClearHistory}
            disabled={clearing}
            className="flex items-center gap-1 text-[9px] text-destructive/50 hover:text-destructive transition-colors disabled:opacity-30"
          >
            {clearing ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Trash2 className="w-2.5 h-2.5" />}
            Clear History
          </button>
        </div>
      )}

      {!loadedFromDb && (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
        </div>
      )}
      {isEmpty && (
        <div className="flex flex-col items-center justify-center py-12 text-muted-foreground/30">
          <Bot className="w-8 h-8 mb-2" />
          <span className="text-[11px]">No research started yet</span>
        </div>
      )}

      {/* Completed turns history — blocks rendered in order */}
      {completedTurns.map((turn) => (
        <div key={turn.id} className="space-y-2">
          {turn.blocks.map((block, i) =>
            block.type === "tool" ? (
              <div key={block.id || i} className="flex items-center gap-2 px-3 py-1.5 rounded bg-muted/8 border border-border/5">
                <Zap className="w-3 h-3 text-accent/60 flex-shrink-0" />
                <span className="text-[10px] font-mono text-foreground/60 truncate">{block.name}</span>
                <span className={cn(
                  "text-[9px] ml-auto px-1.5 py-0.5 rounded",
                  block.status === "completed" ? "text-green-400 bg-green-500/10" :
                  block.status === "error" ? "text-red-400 bg-red-500/10" :
                  "text-muted-foreground/40 bg-muted/10"
                )}>
                  {block.status === "completed" ? "done" : block.status === "error" ? "error" : "done"}
                </span>
              </div>
            ) : block.content ? (
              <div key={i} className={proseClasses}>
                <Markdown content={block.content} />
              </div>
            ) : null
          )}
        </div>
      ))}

      {/* Live streaming: rendered in order (interleaved) */}
      {isThinking && (
        <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-muted/10 border border-border/5">
          <Loader2 className="w-3 h-3 animate-spin text-muted-foreground/50" />
          <span className="text-[10px] text-muted-foreground/50">Thinking...</span>
        </div>
      )}
      {streamingBlocks.map((block, i) =>
        block.type === "tool" ? (
          <div key={block.toolCall.id} className="flex items-center gap-2 px-3 py-1.5 rounded bg-muted/8 border border-border/5">
            <Zap className="w-3 h-3 text-accent/60 flex-shrink-0" />
            <span className="text-[10px] font-mono text-foreground/60 truncate">{block.toolCall.name}</span>
            <span className={cn(
              "text-[9px] ml-auto px-1.5 py-0.5 rounded",
              block.toolCall.status === "completed" ? "text-green-400 bg-green-500/10" :
              block.toolCall.status === "error" ? "text-red-400 bg-red-500/10" :
              "text-muted-foreground/40 bg-muted/10"
            )}>
              {block.toolCall.status === "completed" ? "done" : block.toolCall.status === "error" ? "error" : "running..."}
            </span>
          </div>
        ) : block.type === "text" && block.content ? (
          <div key={`text-${i}`} className={proseClasses}>
            <Markdown content={block.content} streaming={isResponding} />
          </div>
        ) : null
      )}

      {/* Approval / Ask Human cards */}
      {pendingApproval && (
        <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 space-y-2">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
            <span className="text-[11px] font-medium text-amber-400">Tool approval needed</span>
          </div>
          <div className="text-[10px] font-mono text-foreground/60">{pendingApproval.name}</div>
          {pendingApproval.args && (
            <pre className="text-[9px] text-muted-foreground/40 bg-muted/10 rounded p-2 overflow-x-auto max-h-24">
              {JSON.stringify(pendingApproval.args, null, 2)}
            </pre>
          )}
          <div className="flex gap-2">
            <button
              onClick={() => { setDismissedApprovalId(pendingApproval.id); respondToToolApproval(sid, { request_id: pendingApproval.id, approved: true, remember: false, always_allow: false }).catch(console.error); }}
              className="px-3 py-1 rounded text-[10px] font-medium bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25 transition-colors"
            >
              Approve
            </button>
            <button
              onClick={() => {
                setDismissedApprovalId(pendingApproval.id);
                respondToToolApproval(sid, { request_id: pendingApproval.id, approved: true, remember: false, always_allow: false }).catch(console.error);
                const ws = useStore.getState().sessions[sid]?.workingDirectory || ".";
                setAgentMode(sid, "auto-approve", ws).catch(console.error);
              }}
              className="px-3 py-1 rounded text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors"
            >
              Run Everything
            </button>
            <button
              onClick={() => { setDismissedApprovalId(pendingApproval.id); respondToToolApproval(sid, { request_id: pendingApproval.id, approved: false, remember: false, always_allow: false }).catch(console.error); }}
              className="px-3 py-1 rounded text-[10px] font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
            >
              Deny
            </button>
          </div>
        </div>
      )}

      {pendingAskHuman && (
        <div className="rounded-lg border border-accent/20 bg-accent/5 p-3 space-y-2">
          <div className="flex items-center gap-2">
            <MessageSquare className="w-3.5 h-3.5 text-accent" />
            <span className="text-[11px] font-medium text-accent">AI needs your input</span>
          </div>
          <div className="text-[11px] text-foreground/70">{pendingAskHuman.question}</div>
          {pendingAskHuman.options.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {pendingAskHuman.options.map((opt) => (
                <button
                  key={opt}
                  onClick={() => { setDismissedAskHumanId(pendingAskHuman.requestId); respondToToolApproval(sid, { request_id: pendingAskHuman.requestId, approved: true, reason: opt, remember: false, always_allow: false }).catch(console.error); }}
                  className="px-2.5 py-1 rounded text-[10px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
                >
                  {opt}
                </button>
              ))}
            </div>
          ) : (
            <div className="flex gap-2">
              <input
                type="text"
                value={askHumanInput}
                onChange={(e) => setAskHumanInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && askHumanInput.trim()) {
                    setDismissedAskHumanId(pendingAskHuman.requestId);
                    respondToToolApproval(sid, { request_id: pendingAskHuman.requestId, approved: true, reason: askHumanInput.trim(), remember: false, always_allow: false }).catch(console.error);
                    setAskHumanInput("");
                  }
                }}
                placeholder="Type your response..."
                className="flex-1 px-2.5 py-1 rounded text-[10px] bg-muted/10 border border-border/10 text-foreground placeholder:text-muted-foreground/30 focus:outline-none focus:ring-1 focus:ring-accent/30"
              />
              <button
                onClick={() => {
                  if (askHumanInput.trim()) {
                    setDismissedAskHumanId(pendingAskHuman.requestId);
                    respondToToolApproval(sid, { request_id: pendingAskHuman.requestId, approved: true, reason: askHumanInput.trim(), remember: false, always_allow: false }).catch(console.error);
                    setAskHumanInput("");
                  }
                }}
                className="px-2.5 py-1 rounded text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors"
              >
                Send
              </button>
            </div>
          )}
        </div>
      )}

      {isResponding && !isThinking && streamingBlocks.length === 0 && (
        <div className="flex items-center gap-2 py-2">
          <Loader2 className="w-3 h-3 animate-spin text-accent/60" />
          <span className="text-[10px] text-muted-foreground/40">Researching...</span>
        </div>
      )}

      {!isResponding && completedTurns.length > 0 && (
        <div className="flex items-center gap-1.5 px-3 py-2 rounded-lg bg-green-500/5 border border-green-500/10">
          <div className="w-1.5 h-1.5 rounded-full bg-green-500" />
          <span className="text-[10px] text-green-400">Research complete</span>
        </div>
      )}
    </div>
  );
}

