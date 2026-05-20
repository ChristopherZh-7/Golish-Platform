/**
 * ProviderCard — one collapsible card per ASM provider.
 *
 * Shows provider name / quota hint / signup link + a KeyEditor for the
 * API key, and a "Test Connection" button that calls `intel_test_connection`.
 */

import { CheckCircle2, ChevronDown, ChevronRight, ExternalLink, XCircle } from "lucide-react";
import { useCallback, useState } from "react";
import { intel } from "@/lib/api";
import type { ConnectionStatus, ProviderMeta } from "@/lib/api/intel";
import { cn } from "@/lib/utils";
import { KeyEditor } from "./KeyEditor";

export function ProviderCard({ meta, onChanged }: { meta: ProviderMeta; onChanged: () => void }) {
  const [open, setOpen] = useState(false);
  const [testing, setTesting] = useState(false);
  const [lastResult, setLastResult] = useState<ConnectionStatus | null>(null);

  const handleTest = useCallback(async () => {
    setTesting(true);
    try {
      const result = await intel.testConnection(meta.id);
      setLastResult(result);
    } catch (e) {
      setLastResult({ status: "network_error", message: String(e) });
    } finally {
      setTesting(false);
    }
  }, [meta.id]);

  const statusBadge = lastResult ? (
    <span
      className={cn(
        "text-[10px] font-medium px-1.5 py-0.5 rounded-md inline-flex items-center gap-1",
        lastResult.status === "ok"
          ? "bg-emerald-500/15 text-emerald-400"
          : "bg-red-500/15 text-red-400"
      )}
      title={lastResult.message}
    >
      {lastResult.status === "ok" ? (
        <CheckCircle2 className="w-3 h-3" />
      ) : (
        <XCircle className="w-3 h-3" />
      )}
      {lastResult.status === "ok"
        ? "Connected"
        : lastResult.status === "auth_failed"
          ? "Auth failed"
          : lastResult.status === "quota_exhausted"
            ? "Quota out"
            : "Network err"}
    </span>
  ) : null;

  return (
    <div className="rounded-lg border border-border/40 bg-card/50">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="w-full px-4 py-3 flex items-center justify-between hover:bg-[var(--bg-hover)]/40 rounded-t-lg transition-colors"
      >
        <div className="flex items-center gap-3 min-w-0">
          {open ? (
            <ChevronDown className="w-4 h-4 text-muted-foreground flex-shrink-0" />
          ) : (
            <ChevronRight className="w-4 h-4 text-muted-foreground flex-shrink-0" />
          )}
          <div className="min-w-0 text-left">
            <div className="text-sm font-medium text-foreground flex items-center gap-2">
              {meta.display_name}
              {meta.requires_paid && (
                <span className="text-[10px] font-medium px-1.5 py-0.5 rounded-md bg-amber-500/15 text-amber-400">
                  Paid
                </span>
              )}
              {statusBadge}
            </div>
            <div className="text-xs text-muted-foreground truncate">{meta.description}</div>
          </div>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          <span className="text-[10px] text-muted-foreground">{meta.id}</span>
        </div>
      </button>

      {open && (
        <div className="border-t border-border/30 p-4 space-y-4">
          <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
            <div>
              <span className="text-muted-foreground">配额：</span>
              <span className="text-foreground/80">{meta.quota_hint || "—"}</span>
            </div>
            <div className="flex items-center gap-3">
              <a
                href={meta.signup_url}
                target="_blank"
                rel="noreferrer"
                className="text-accent hover:underline inline-flex items-center gap-1"
              >
                <ExternalLink className="w-3 h-3" /> 注册 / API 申请
              </a>
              <a
                href={meta.docs_url}
                target="_blank"
                rel="noreferrer"
                className="text-accent hover:underline inline-flex items-center gap-1"
              >
                <ExternalLink className="w-3 h-3" /> 文档
              </a>
            </div>
            <div className="col-span-2">
              <span className="text-muted-foreground">支持的 query_type：</span>
              <span className="text-foreground/80 font-mono text-[11px]">
                {meta.supported_query_types.join(" · ")}
              </span>
            </div>
          </div>

          <KeyEditor providerId={meta.id} onSaved={onChanged} />

          <div className="flex items-center gap-2 pt-2 border-t border-border/20">
            <button
              type="button"
              onClick={handleTest}
              disabled={testing}
              className={cn(
                "text-xs px-3 py-1.5 rounded-md border transition-colors",
                "border-accent/40 bg-accent/10 text-accent hover:bg-accent/20",
                "disabled:opacity-50 disabled:cursor-not-allowed"
              )}
            >
              {testing ? "Testing..." : "Test connection"}
            </button>
            {lastResult && (
              <span
                className={cn(
                  "text-[11px]",
                  lastResult.status === "ok" ? "text-emerald-400" : "text-red-400"
                )}
              >
                {lastResult.message}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
