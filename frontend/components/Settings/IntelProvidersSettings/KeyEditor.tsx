/**
 * KeyEditor — wraps vault CRUD for one provider's API key.
 *
 * Convention:
 * - vault.name == provider_id (e.g. "0.zone")
 * - vault.entry_type == "api_key"
 * - vault.tags == ["intel-provider", provider_id]
 *
 * On load: lists vault entries, finds the matching row.
 * On save: if existing row → update; else → add.
 * On delete: removes the row.
 */

import { Eye, EyeOff, Save, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { vault } from "@/lib/api";
import type { VaultEntrySafe } from "@/lib/api/vault";
import { notify } from "@/lib/notify";
import { cn } from "@/lib/utils";

export function KeyEditor({ providerId, onSaved }: { providerId: string; onSaved: () => void }) {
  const [entry, setEntry] = useState<VaultEntrySafe | null>(null);
  const [value, setValue] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [busy, setBusy] = useState(false);

  // Load existing key on mount.
  useEffect(() => {
    let cancelled = false;
    vault
      .listVaultEntries(null)
      .then(async (entries) => {
        if (cancelled) return;
        const match = entries.find(
          (e: VaultEntrySafe) => e.name === providerId && e.type === "api_key"
        );
        if (match) {
          setEntry(match);
          // Fetch the cleartext value so users can confirm what's saved.
          try {
            const v = await vault.getVaultValue(match.id, null);
            if (!cancelled) setValue(v ?? "");
          } catch (e) {
            console.warn("Failed to load existing key value:", e);
          }
        }
      })
      .catch((e) => console.error("Failed to list vault entries:", e));
    return () => {
      cancelled = true;
    };
  }, [providerId]);

  const save = useCallback(async () => {
    if (!value.trim()) {
      notify.error("API key 不能为空");
      return;
    }
    setBusy(true);
    try {
      if (entry) {
        await vault.updateVaultEntry({
          id: entry.id,
          value,
          username: entry.username,
          notes: entry.notes,
          projectPath: null,
        });
      } else {
        const created = await vault.addVaultEntry({
          name: providerId,
          entryType: "api_key",
          value,
          username: "",
          notes: `Intel provider API key for ${providerId}`,
          project: "",
          tags: ["intel-provider", providerId],
          projectPath: null,
        });
        setEntry(created);
      }
      notify.success("API key 已保存");
      onSaved();
    } catch (e) {
      console.error("Failed to save key:", e);
      notify.error(`保存失败：${e}`);
    } finally {
      setBusy(false);
    }
  }, [entry, providerId, value, onSaved]);

  const remove = useCallback(async () => {
    if (!entry) return;
    if (!confirm(`确认删除 ${providerId} 的 API key？`)) return;
    setBusy(true);
    try {
      await vault.deleteVaultEntry(entry.id, null);
      setEntry(null);
      setValue("");
      notify.success("API key 已删除");
      onSaved();
    } catch (e) {
      console.error("Failed to delete key:", e);
      notify.error(`删除失败：${e}`);
    } finally {
      setBusy(false);
    }
  }, [entry, providerId, onSaved]);

  return (
    <div className="space-y-2">
      <label className="text-xs font-medium text-foreground/80">API Key</label>
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <input
            type={revealed ? "text" : "password"}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={`请输入 ${providerId} API key (zone_key_id / fofa_key / ...)`}
            disabled={busy}
            className={cn(
              "w-full px-3 py-1.5 pr-9 text-xs rounded-md border bg-background",
              "border-border/40 focus:border-accent/60 outline-none transition-colors",
              "font-mono",
              "disabled:opacity-50"
            )}
          />
          <button
            type="button"
            onClick={() => setRevealed(!revealed)}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          >
            {revealed ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
          </button>
        </div>
        <button
          type="button"
          onClick={save}
          disabled={busy || !value.trim()}
          className={cn(
            "px-3 py-1.5 text-xs rounded-md border transition-colors inline-flex items-center gap-1",
            "border-emerald-500/40 bg-emerald-500/10 text-emerald-400 hover:bg-emerald-500/20",
            "disabled:opacity-40 disabled:cursor-not-allowed"
          )}
        >
          <Save className="w-3 h-3" /> 保存
        </button>
        {entry && (
          <button
            type="button"
            onClick={remove}
            disabled={busy}
            className={cn(
              "px-2 py-1.5 text-xs rounded-md border transition-colors",
              "border-red-500/40 bg-red-500/10 text-red-400 hover:bg-red-500/20",
              "disabled:opacity-40"
            )}
            title="删除当前 key"
          >
            <Trash2 className="w-3 h-3" />
          </button>
        )}
      </div>
      {entry && (
        <div className="text-[10px] text-muted-foreground">
          已保存 · vault entry {entry.id.slice(0, 8)}… · 上次更新{" "}
          {new Date(entry.updated_at * 1000).toLocaleString()}
        </div>
      )}
    </div>
  );
}
