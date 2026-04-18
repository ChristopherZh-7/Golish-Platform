import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { CustomSelect } from "@/components/ui/custom-select";
import { zapSendRequest } from "@/lib/pentest/zap-api";
import {
  AlertTriangle, Crosshair,
  Loader2, Pause, Play, Plus, Settings2, Trash2, X,
} from "lucide-react";

const MARKER = "§";

// ── Built-in payload lists ───────────────────────────────────────────

const BUILTIN_PAYLOADS: Record<string, string[]> = {
  "SQLi (basic)": [
    "'", "\"", "' OR '1'='1", "\" OR \"1\"=\"1", "1' OR '1'='1'--",
    "' UNION SELECT NULL--", "1; DROP TABLE users--", "' AND 1=1--",
    "' AND 1=2--", "admin'--", "1' WAITFOR DELAY '0:0:5'--",
    "1 AND SLEEP(5)", "' OR SLEEP(5)#",
  ],
  "XSS (basic)": [
    "<script>alert(1)</script>", "<img src=x onerror=alert(1)>",
    "<svg onload=alert(1)>", "javascript:alert(1)", "\"><script>alert(1)</script>",
    "'-alert(1)-'", "<body onload=alert(1)>", "<input onfocus=alert(1) autofocus>",
    "{{7*7}}", "${7*7}", "<img/src=x onerror=alert(1)>",
  ],
  "Path Traversal": [
    "../../../etc/passwd", "..\\..\\..\\windows\\win.ini",
    "....//....//....//etc/passwd", "%2e%2e%2f%2e%2e%2f%2e%2e%2fetc%2fpasswd",
    "/etc/passwd", "file:///etc/passwd",
  ],
  "Command Injection": [
    "; id", "| id", "` id `", "$(id)", "; cat /etc/passwd",
    "| cat /etc/passwd", "\n id", "& id", "&& id",
  ],
  "SSTI": [
    "{{7*7}}", "${7*7}", "<%= 7*7 %>", "#{7*7}", "{7*7}",
    "{{config}}", "{{self}}", "${T(java.lang.Runtime).getRuntime().exec('id')}",
  ],
  "Auth Bypass": [
    "admin", "administrator", "root", "test", "guest",
    "null", "undefined", "true", "false", "0", "1", "-1",
  ],
  "Numbers (0-9)": Array.from({ length: 10 }, (_, i) => String(i)),
};

// ── Types ────────────────────────────────────────────────────────────

type AttackMode = "sniper" | "battering_ram" | "pitchfork";

interface IntruderResult {
  index: number;
  payload: string;
  position: number;
  status: number;
  length: number;
  time_ms: number;
  responseHeaders: string;
  responseBody: string;
  error?: string;
}

interface IntruderPanelProps {
  injectedRequest?: string | null;
  onInjectedConsumed?: () => void;
}

// ── Main Component ───────────────────────────────────────────────────

export function IntruderPanel({ injectedRequest, onInjectedConsumed }: IntruderPanelProps) {
  const [rawRequest, setRawRequest] = useState(
    "GET / HTTP/1.1\nHost: example.com\nUser-Agent: Mozilla/5.0\n\n",
  );
  const [attackMode, setAttackMode] = useState<AttackMode>("sniper");
  const [payloadSets, setPayloadSets] = useState<string[][]>([[]]);
  const [payloadInput, setPayloadInput] = useState("");
  const [activePayloadSet, setActivePayloadSet] = useState(0);
  const [selectedBuiltin, setSelectedBuiltin] = useState("");
  const [results, setResults] = useState<IntruderResult[]>([]);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState({ current: 0, total: 0 });
  const [selectedResult, setSelectedResult] = useState<IntruderResult | null>(null);
  const [showConfig, setShowConfig] = useState(false);
  const [concurrency, setConcurrency] = useState(1);
  const [delay, setDelay] = useState(0);
  const abortRef = useRef(false);

  useEffect(() => {
    if (injectedRequest) {
      setRawRequest(injectedRequest);
      onInjectedConsumed?.();
    }
  }, [injectedRequest, onInjectedConsumed]);

  // Find marker positions in the request
  const positions = useMemo(() => {
    const pos: { start: number; end: number; text: string }[] = [];
    let i = 0;
    while (i < rawRequest.length) {
      const start = rawRequest.indexOf(MARKER, i);
      if (start === -1) break;
      const end = rawRequest.indexOf(MARKER, start + 1);
      if (end === -1) break;
      pos.push({ start, end: end + 1, text: rawRequest.slice(start + 1, end) });
      i = end + 1;
    }
    return pos;
  }, [rawRequest]);

  // Generate request variants from base + payloads
  const generateRequests = useCallback((): { request: string; payload: string; posIdx: number }[] => {
    if (positions.length === 0 || payloadSets.every((s) => s.length === 0)) return [];

    const variants: { request: string; payload: string; posIdx: number }[] = [];

    if (attackMode === "sniper") {
      for (let pi = 0; pi < positions.length; pi++) {
        const payloads = payloadSets[pi % payloadSets.length] ?? [];
        for (const payload of payloads) {
          let req = rawRequest;
          for (let j = positions.length - 1; j >= 0; j--) {
            const p = positions[j];
            if (j === pi) {
              req = req.slice(0, p.start) + payload + req.slice(p.end);
            } else {
              req = req.slice(0, p.start) + p.text + req.slice(p.end);
            }
          }
          variants.push({ request: req, payload, posIdx: pi });
        }
      }
    } else if (attackMode === "battering_ram") {
      const payloads = payloadSets[0] ?? [];
      for (const payload of payloads) {
        let req = rawRequest;
        for (let j = positions.length - 1; j >= 0; j--) {
          const p = positions[j];
          req = req.slice(0, p.start) + payload + req.slice(p.end);
        }
        variants.push({ request: req, payload, posIdx: 0 });
      }
    } else {
      const maxLen = Math.max(...payloadSets.map((s) => s.length));
      for (let i = 0; i < maxLen; i++) {
        let req = rawRequest;
        const payloadParts: string[] = [];
        for (let j = positions.length - 1; j >= 0; j--) {
          const p = positions[j];
          const payloads = payloadSets[j % payloadSets.length] ?? [];
          const pl = payloads[i % payloads.length] ?? "";
          req = req.slice(0, p.start) + pl + req.slice(p.end);
          payloadParts.unshift(pl);
        }
        variants.push({ request: req, payload: payloadParts.join(" | "), posIdx: 0 });
      }
    }
    return variants;
  }, [rawRequest, positions, payloadSets, attackMode]);

  const runAttack = useCallback(async () => {
    const variants = generateRequests();
    if (variants.length === 0) return;

    abortRef.current = false;
    setRunning(true);
    setResults([]);
    setProgress({ current: 0, total: variants.length });
    setSelectedResult(null);

    for (let i = 0; i < variants.length; i++) {
      if (abortRef.current) break;

      const { request, payload, posIdx } = variants[i];
      const start = performance.now();

      try {
        const resp = await zapSendRequest(request);
        const elapsed = Math.round(performance.now() - start);
        const bodyLen = (resp.response_body ?? "").length;
        const status = resp.status_code ?? 0;

        setResults((prev) => [
          ...prev,
          {
            index: i,
            payload,
            position: posIdx,
            status,
            length: bodyLen,
            time_ms: elapsed,
            responseHeaders: resp.response_header ?? "",
            responseBody: resp.response_body ?? "",
          },
        ]);
      } catch (e) {
        setResults((prev) => [
          ...prev,
          {
            index: i, payload, position: posIdx, status: 0, length: 0,
            time_ms: Math.round(performance.now() - start),
            responseHeaders: "", responseBody: "",
            error: String(e),
          },
        ]);
      }

      setProgress({ current: i + 1, total: variants.length });

      if (delay > 0) {
        await new Promise((r) => setTimeout(r, delay));
      }
    }

    setRunning(false);
  }, [generateRequests, delay]);

  const stopAttack = useCallback(() => { abortRef.current = true; }, []);

  const addPayload = useCallback(() => {
    if (!payloadInput.trim()) return;
    const lines = payloadInput.split("\n").map((l) => l.trim()).filter(Boolean);
    setPayloadSets((prev) => {
      const next = [...prev];
      next[activePayloadSet] = [...(next[activePayloadSet] ?? []), ...lines];
      return next;
    });
    setPayloadInput("");
  }, [payloadInput, activePayloadSet]);

  const loadBuiltin = useCallback(() => {
    if (!selectedBuiltin || !BUILTIN_PAYLOADS[selectedBuiltin]) return;
    setPayloadSets((prev) => {
      const next = [...prev];
      next[activePayloadSet] = [...(next[activePayloadSet] ?? []), ...BUILTIN_PAYLOADS[selectedBuiltin]];
      return next;
    });
    setSelectedBuiltin("");
  }, [selectedBuiltin, activePayloadSet]);

  const clearPayloadSet = useCallback(() => {
    setPayloadSets((prev) => {
      const next = [...prev];
      next[activePayloadSet] = [];
      return next;
    });
  }, [activePayloadSet]);

  const totalRequests = generateRequests().length;

  return (
    <div className="h-full flex flex-col text-[11px]">
      {/* Header bar */}
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-border/10 flex-shrink-0">
        <Crosshair className="w-3.5 h-3.5 text-accent" />
        <span className="font-medium text-foreground/80">Intruder</span>

        <CustomSelect
          className="ml-2 min-w-[100px]"
          value={attackMode}
          onChange={(v) => setAttackMode(v as AttackMode)}
          options={[
            { value: "sniper", label: "Sniper" },
            { value: "battering_ram", label: "Battering Ram" },
            { value: "pitchfork", label: "Pitchfork" },
          ]}
          size="xs"
        />

        <span className="text-muted-foreground/30 text-[9px] ml-1">
          {positions.length} position{positions.length !== 1 ? "s" : ""} · {totalRequests} request{totalRequests !== 1 ? "s" : ""}
        </span>

        <div className="ml-auto flex items-center gap-1">
          <button
            type="button"
            onClick={() => setShowConfig(!showConfig)}
            className={cn(
              "p-1 rounded transition-colors",
              showConfig ? "text-accent bg-accent/10" : "text-muted-foreground/30 hover:text-muted-foreground/60",
            )}
          >
            <Settings2 className="w-3 h-3" />
          </button>

          {running ? (
            <button
              type="button"
              onClick={stopAttack}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium bg-red-500/15 text-red-400 hover:bg-red-500/25 transition-colors"
            >
              <Pause className="w-3 h-3" /> Stop
            </button>
          ) : (
            <button
              type="button"
              onClick={runAttack}
              disabled={positions.length === 0 || payloadSets.every((s) => s.length === 0)}
              className={cn(
                "flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium transition-colors",
                positions.length === 0 || payloadSets.every((s) => s.length === 0)
                  ? "bg-muted/10 text-muted-foreground/20 cursor-not-allowed"
                  : "bg-accent/15 text-accent hover:bg-accent/25",
              )}
            >
              <Play className="w-3 h-3" /> Attack
            </button>
          )}
        </div>
      </div>

      {/* Config panel */}
      {showConfig && (
        <div className="flex items-center gap-4 px-3 py-1.5 border-b border-border/5 bg-muted/5">
          <label className="flex items-center gap-1.5 text-[9px] text-muted-foreground/50">
            Concurrency
            <input
              type="number" min={1} max={20}
              className="w-10 px-1 py-0.5 bg-background border border-border/30 rounded text-[10px] outline-none"
              value={concurrency} onChange={(e) => setConcurrency(Number(e.target.value) || 1)}
            />
          </label>
          <label className="flex items-center gap-1.5 text-[9px] text-muted-foreground/50">
            Delay (ms)
            <input
              type="number" min={0}
              className="w-14 px-1 py-0.5 bg-background border border-border/30 rounded text-[10px] outline-none"
              value={delay} onChange={(e) => setDelay(Number(e.target.value) || 0)}
            />
          </label>
          <span className="text-[8px] text-muted-foreground/25">
            Mark positions with {MARKER}value{MARKER} in the request editor
          </span>
        </div>
      )}

      {/* Progress bar */}
      {running && (
        <div className="px-3 py-1 border-b border-border/5">
          <div className="flex items-center justify-between text-[9px] text-muted-foreground/40 mb-0.5">
            <span>{progress.current}/{progress.total}</span>
            <span>{((progress.current / Math.max(1, progress.total)) * 100).toFixed(0)}%</span>
          </div>
          <div className="h-1 rounded-full bg-muted/10 overflow-hidden">
            <div
              className="h-full rounded-full bg-accent/60 transition-all"
              style={{ width: `${(progress.current / Math.max(1, progress.total)) * 100}%` }}
            />
          </div>
        </div>
      )}

      {/* Main content: request editor + payloads + results */}
      <div className="flex-1 min-h-0 flex">
        {/* Left: Request editor + Payloads */}
        <div className="w-[380px] flex-shrink-0 border-r border-border/10 flex flex-col">
          {/* Request editor */}
          <div className="flex-1 min-h-0 flex flex-col">
            <div className="px-2 py-1 text-[9px] font-medium text-muted-foreground/40 border-b border-border/5 flex items-center justify-between">
              <span>Request (mark positions with {MARKER}…{MARKER})</span>
              <span className="text-accent/50">{positions.length} pos</span>
            </div>
            <textarea
              className="flex-1 min-h-0 w-full px-2 py-1.5 bg-background text-[10px] font-mono text-foreground resize-none outline-none"
              value={rawRequest}
              onChange={(e) => setRawRequest(e.target.value)}
              spellCheck={false}
              disabled={running}
            />
          </div>

          {/* Payload manager */}
          <div className="h-[220px] flex-shrink-0 border-t border-border/10 flex flex-col">
            <div className="px-2 py-1 text-[9px] font-medium text-muted-foreground/40 border-b border-border/5 flex items-center gap-2">
              <span>Payloads</span>
              {payloadSets.length > 1 && (
                <div className="flex gap-0.5">
                  {payloadSets.map((_, i) => (
                    <button
                      key={i}
                      type="button"
                      className={cn(
                        "px-1.5 py-0.5 rounded text-[8px]",
                        i === activePayloadSet
                          ? "bg-accent/15 text-accent"
                          : "text-muted-foreground/30 hover:text-muted-foreground/60",
                      )}
                      onClick={() => setActivePayloadSet(i)}
                    >
                      Set {i + 1} ({payloadSets[i]?.length ?? 0})
                    </button>
                  ))}
                </div>
              )}
              <div className="ml-auto flex items-center gap-1">
                {attackMode !== "battering_ram" && (
                  <button
                    type="button"
                    className="p-0.5 rounded text-muted-foreground/20 hover:text-accent"
                    onClick={() => {
                      setPayloadSets((prev) => [...prev, []]);
                      setActivePayloadSet(payloadSets.length);
                    }}
                    title="Add payload set"
                  >
                    <Plus className="w-2.5 h-2.5" />
                  </button>
                )}
                <button
                  type="button"
                  className="p-0.5 rounded text-muted-foreground/20 hover:text-red-400"
                  onClick={clearPayloadSet}
                  title="Clear current set"
                >
                  <Trash2 className="w-2.5 h-2.5" />
                </button>
              </div>
            </div>

            {/* Builtin selector */}
            <div className="flex items-center gap-1 px-2 py-1">
              <CustomSelect
                className="flex-1"
                value={selectedBuiltin}
                onChange={setSelectedBuiltin}
                options={[
                  { value: "", label: "Load built-in list…" },
                  ...Object.keys(BUILTIN_PAYLOADS).map((k) => ({
                    value: k, label: `${k} (${BUILTIN_PAYLOADS[k].length})`,
                  })),
                ]}
                size="xs"
              />
              <button
                type="button"
                onClick={loadBuiltin}
                disabled={!selectedBuiltin}
                className="px-1.5 py-0.5 rounded text-[9px] bg-accent/10 text-accent hover:bg-accent/20 disabled:opacity-30"
              >
                Load
              </button>
            </div>

            {/* Manual add */}
            <div className="flex items-center gap-1 px-2">
              <input
                className="flex-1 px-1.5 py-0.5 text-[9px] bg-background border border-border/30 rounded outline-none"
                placeholder="Add payloads (one per line)"
                value={payloadInput}
                onChange={(e) => setPayloadInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); addPayload(); } }}
              />
              <button
                type="button"
                onClick={addPayload}
                className="px-1.5 py-0.5 rounded text-[9px] bg-accent/10 text-accent hover:bg-accent/20"
              >
                Add
              </button>
            </div>

            {/* Payload list preview */}
            <div className="flex-1 min-h-0 overflow-y-auto px-2 py-1">
              {(payloadSets[activePayloadSet] ?? []).length === 0 ? (
                <div className="text-[9px] text-muted-foreground/20 text-center py-3">
                  No payloads. Load a built-in list or add manually.
                </div>
              ) : (
                <div className="space-y-0">
                  {(payloadSets[activePayloadSet] ?? []).slice(0, 50).map((p, i) => (
                    <div key={`${i}-${p}`} className="flex items-center gap-1 text-[9px] py-0.5 group">
                      <span className="w-5 text-right text-muted-foreground/20 font-mono">{i + 1}</span>
                      <span className="font-mono text-foreground/60 truncate flex-1">{p}</span>
                      <button
                        type="button"
                        className="opacity-0 group-hover:opacity-100 p-0.5 text-muted-foreground/20 hover:text-red-400"
                        onClick={() => {
                          setPayloadSets((prev) => {
                            const next = [...prev];
                            next[activePayloadSet] = (next[activePayloadSet] ?? []).filter((_, idx) => idx !== i);
                            return next;
                          });
                        }}
                      >
                        <X className="w-2 h-2" />
                      </button>
                    </div>
                  ))}
                  {(payloadSets[activePayloadSet] ?? []).length > 50 && (
                    <div className="text-[8px] text-muted-foreground/20 text-center py-1">
                      … and {(payloadSets[activePayloadSet] ?? []).length - 50} more
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Right: Results table + detail */}
        <div className="flex-1 min-w-0 flex flex-col">
          {results.length === 0 && !running ? (
            <div className="flex-1 flex flex-col items-center justify-center gap-2 text-muted-foreground/15">
              <Crosshair className="w-10 h-10" />
              <span className="text-[11px] font-medium">No results yet</span>
              <span className="text-[9px] max-w-[200px] text-center">
                Mark positions with {MARKER}…{MARKER}, add payloads, then click Attack
              </span>
            </div>
          ) : (
            <div className="flex-1 min-h-0 flex flex-col">
              {/* Results table */}
              <div className={cn("overflow-y-auto", selectedResult ? "h-1/2" : "flex-1")}>
                <table className="w-full">
                  <thead className="sticky top-0 bg-card z-10">
                    <tr className="text-left text-[9px] text-muted-foreground/40 border-b border-border/10">
                      <th className="px-2 py-1 w-8">#</th>
                      <th className="px-2 py-1">Payload</th>
                      <th className="px-2 py-1 w-14">Status</th>
                      <th className="px-2 py-1 w-16">Length</th>
                      <th className="px-2 py-1 w-14">Time</th>
                    </tr>
                  </thead>
                  <tbody>
                    {results.map((r) => (
                      <tr
                        key={r.index}
                        className={cn(
                          "border-b border-border/5 cursor-pointer hover:bg-muted/10 transition-colors text-[10px]",
                          selectedResult?.index === r.index && "bg-accent/5",
                        )}
                        onClick={() => setSelectedResult(r)}
                      >
                        <td className="px-2 py-1 text-muted-foreground/25 font-mono">{r.index + 1}</td>
                        <td className="px-2 py-1 font-mono truncate max-w-[200px]">
                          {r.error ? (
                            <span className="text-red-400 flex items-center gap-1">
                              <AlertTriangle className="w-2.5 h-2.5" /> {r.error.slice(0, 40)}
                            </span>
                          ) : (
                            <span className="text-foreground/70">{r.payload}</span>
                          )}
                        </td>
                        <td className={cn("px-2 py-1 font-mono", statusColor(r.status))}>{r.status || "—"}</td>
                        <td className="px-2 py-1 font-mono text-muted-foreground/40">{r.length}</td>
                        <td className="px-2 py-1 font-mono text-muted-foreground/30">{r.time_ms}ms</td>
                      </tr>
                    ))}
                    {running && (
                      <tr>
                        <td colSpan={5} className="px-2 py-2 text-center">
                          <Loader2 className="w-3 h-3 animate-spin text-accent inline mr-1" />
                          <span className="text-[9px] text-muted-foreground/30">Running…</span>
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>

              {/* Response detail */}
              {selectedResult && (
                <div className="h-1/2 border-t border-border/10 flex flex-col">
                  <div className="flex items-center justify-between px-2 py-1 border-b border-border/5 flex-shrink-0">
                    <span className="text-[9px] text-muted-foreground/40">
                      #{selectedResult.index + 1} — {selectedResult.payload} — {selectedResult.status}
                    </span>
                    <button
                      type="button"
                      onClick={() => setSelectedResult(null)}
                      className="p-0.5 text-muted-foreground/20 hover:text-foreground"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                  <div className="flex-1 min-h-0 overflow-auto">
                    <pre className="px-2 py-1 text-[9px] font-mono text-foreground/50 whitespace-pre-wrap break-all">
                      {selectedResult.responseHeaders}
                      {selectedResult.responseHeaders && "\n\n"}
                      {selectedResult.responseBody.slice(0, 10000)}
                      {selectedResult.responseBody.length > 10000 && "\n\n… (truncated)"}
                    </pre>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function statusColor(code: number): string {
  if (code >= 200 && code < 300) return "text-green-400";
  if (code >= 300 && code < 400) return "text-blue-400";
  if (code >= 400 && code < 500) return "text-yellow-400";
  if (code >= 500) return "text-red-400";
  return "text-muted-foreground/30";
}
