import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown, Copy, Eye, EyeOff, KeyRound, Link2, Plus, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";

interface VaultEntrySafe {
  id: string;
  name: string;
  type: "password" | "token" | "ssh_key" | "api_key" | "cookie" | "certificate" | "other";
  username: string;
  notes: string;
  project: string;
  tags: string[];
  created_at: number;
  updated_at: number;
}

const ENTRY_TYPES = [
  "password", "token", "api_key", "ssh_key", "cookie", "certificate", "other",
] as const;

const TYPE_LABEL_KEYS: Record<string, string> = {
  password: "vault.password",
  token: "vault.token",
  ssh_key: "vault.sshKey",
  api_key: "vault.apiKey",
  cookie: "vault.cookie",
  certificate: "vault.certificate",
  other: "vault.other",
};

function VaultMiniDropdown({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        className={cn(
          "w-full flex items-center justify-between gap-1.5 px-2.5 py-1.5 text-xs rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/30 border-border/30 text-foreground",
          "hover:bg-[var(--bg-hover)]/60 hover:border-border/50",
          open && "border-accent/40 bg-[var(--bg-hover)]/50",
        )}
        onClick={() => setOpen(!open)}
      >
        <span className="truncate">{selected?.label ?? value}</span>
        <ChevronDown className={cn("w-3 h-3 text-muted-foreground transition-transform flex-shrink-0", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 w-full min-w-[120px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-xs transition-colors",
                "hover:bg-[var(--bg-hover)]/60",
                opt.value === value && "text-accent bg-accent/5 font-medium",
              )}
              onClick={() => { onChange(opt.value); setOpen(false); }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function VaultSettings() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [entries, setEntries] = useState<VaultEntrySafe[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [revealedIds, setRevealedIds] = useState<Set<string>>(new Set());
  const [revealedValues, setRevealedValues] = useState<Record<string, string>>({});
  const [addForm, setAddForm] = useState({
    name: "", type: "password" as string, value: "", username: "", notes: "", project: "", tags: "",
  });

  const loadEntries = useCallback(async () => {
    try {
      const data = await invoke<VaultEntrySafe[]>("vault_list", { projectPath: getProjectPath() });
      setEntries(Array.isArray(data) ? data : []);
    } catch (e) {
      console.error("Failed to load vault:", e);
    }
  }, []);

  useEffect(() => { loadEntries(); }, [loadEntries, currentProjectPath]);

  const handleAdd = useCallback(async () => {
    if (!addForm.name.trim() || !addForm.value.trim()) return;
    try {
      await invoke("vault_add", {
        name: addForm.name.trim(),
        entryType: addForm.type,
        value: addForm.value,
        username: addForm.username || null,
        notes: addForm.notes || null,
        project: addForm.project || null,
        tags: addForm.tags ? addForm.tags.split(",").map((s) => s.trim()).filter(Boolean) : null,
        projectPath: getProjectPath(),
      });
      setAddForm({ name: "", type: "password", value: "", username: "", notes: "", project: "", tags: "" });
      setShowAdd(false);
      loadEntries();
      invoke("audit_log", { action: "vault_entry_added", category: "vault", details: addForm.name.trim(), projectPath: getProjectPath() }).catch(() => {});
    } catch (e) {
      console.error("Failed to add entry:", e);
    }
  }, [addForm, loadEntries]);

  const handleDelete = useCallback(async (id: string, name: string) => {
    if (!confirm(t("vault.deleteConfirm", { name }))) return;
    try {
      await invoke("vault_delete", { id, projectPath: getProjectPath() });
      setRevealedIds((s) => { const n = new Set(s); n.delete(id); return n; });
      loadEntries();
      invoke("audit_log", { action: "vault_entry_deleted", category: "vault", details: name, entityType: "vault", entityId: id, projectPath: getProjectPath() }).catch(() => {});
    } catch (e) {
      console.error("Failed to delete:", e);
    }
  }, [t, loadEntries]);

  const handleRevealToggle = useCallback(async (id: string) => {
    if (revealedIds.has(id)) {
      setRevealedIds((s) => { const n = new Set(s); n.delete(id); return n; });
      return;
    }
    try {
      const value = await invoke<string>("vault_get_value", { id, projectPath: getProjectPath() });
      setRevealedValues((v) => ({ ...v, [id]: value }));
      setRevealedIds((s) => new Set(s).add(id));
    } catch (e) {
      console.error("Failed to reveal:", e);
    }
  }, [revealedIds]);

  const handleCopyValue = useCallback(async (id: string) => {
    try {
      const value = await invoke<string>("vault_get_value", { id, projectPath: getProjectPath() });
      await navigator.clipboard.writeText(value);
    } catch (e) {
      console.error("Failed to copy:", e);
    }
  }, []);

  const handleCopyRef = useCallback(async (name: string) => {
    const ref = `{{vault:${name}}}`;
    await navigator.clipboard.writeText(ref);
  }, []);

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <span className="text-[11px] text-muted-foreground/40">
          {entries.length} {t("vault.credentials", "credentials")}
        </span>
        <span className="text-[10px] text-muted-foreground/20 ml-1">
          {t("vault.refHint", "Reference: {{vault:name}}")}
        </span>
        <div className="flex-1" />
        <button
          className="flex items-center gap-1.5 px-2.5 py-1 text-[11px] rounded-md font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
          onClick={() => setShowAdd(!showAdd)}
        >
          <Plus className="w-3 h-3" />
          {t("vault.addEntry")}
        </button>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-3">

      {/* Add form */}
      {showAdd && (
        <div className="p-4 rounded-lg border border-border/50 bg-muted/10 space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-[11px] text-muted-foreground block mb-1">{t("vault.name")} *</label>
              <input
                className="w-full text-xs bg-background border border-border/50 rounded px-2.5 py-1.5 outline-none focus:border-accent"
                placeholder={t("vault.namePlaceholder")}
                value={addForm.name}
                onChange={(e) => setAddForm((f) => ({ ...f, name: e.target.value }))}
                autoFocus
              />
            </div>
            <div>
              <label className="text-[11px] text-muted-foreground block mb-1">{t("vault.type")}</label>
              <VaultMiniDropdown
                value={addForm.type}
                onChange={(v) => setAddForm((f) => ({ ...f, type: v }))}
                options={ENTRY_TYPES.map((tt) => ({ value: tt, label: t(TYPE_LABEL_KEYS[tt]) }))}
              />
            </div>
          </div>
          <div>
            <label className="text-[11px] text-muted-foreground block mb-1">{t("vault.value")} *</label>
            <textarea
              className="w-full text-xs bg-background border border-border/50 rounded px-2.5 py-1.5 outline-none focus:border-accent resize-none font-mono"
              placeholder={t("vault.valuePlaceholder")}
              rows={addForm.type === "ssh_key" || addForm.type === "certificate" ? 6 : 2}
              value={addForm.value}
              onChange={(e) => setAddForm((f) => ({ ...f, value: e.target.value }))}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-[11px] text-muted-foreground block mb-1">{t("vault.username")}</label>
              <input
                className="w-full text-xs bg-background border border-border/50 rounded px-2.5 py-1.5 outline-none focus:border-accent"
                placeholder={t("vault.usernamePlaceholder")}
                value={addForm.username}
                onChange={(e) => setAddForm((f) => ({ ...f, username: e.target.value }))}
              />
            </div>
            <div>
              <label className="text-[11px] text-muted-foreground block mb-1">{t("vault.project")}</label>
              <input
                className="w-full text-xs bg-background border border-border/50 rounded px-2.5 py-1.5 outline-none focus:border-accent"
                value={addForm.project}
                onChange={(e) => setAddForm((f) => ({ ...f, project: e.target.value }))}
              />
            </div>
          </div>
          <div>
            <label className="text-[11px] text-muted-foreground block mb-1">{t("vault.notes")}</label>
            <input
              className="w-full text-xs bg-background border border-border/50 rounded px-2.5 py-1.5 outline-none focus:border-accent"
              value={addForm.notes}
              onChange={(e) => setAddForm((f) => ({ ...f, notes: e.target.value }))}
            />
          </div>
          <div className="flex justify-end gap-2 pt-1">
            <button
              className="px-3 py-1.5 text-xs rounded bg-muted/50 hover:bg-muted text-foreground"
              onClick={() => setShowAdd(false)}
            >{t("common.cancel")}</button>
            <button
              className="px-3 py-1.5 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90"
              onClick={handleAdd}
            >{t("vault.addEntry")}</button>
          </div>
        </div>
      )}

      {entries.length === 0 && !showAdd ? (
        <div className="flex flex-col items-center justify-center py-16 text-muted-foreground/20">
          <KeyRound className="w-10 h-10 mb-3" />
          <p className="text-[12px] font-medium">{t("vault.noEntries")}</p>
          <p className="text-[10px] text-muted-foreground/15 mt-1 max-w-xs text-center">
            {t("vault.description")}
          </p>
        </div>
      ) : (
        <div className="space-y-1.5">
          {entries.map((entry) => (
            <div
              key={entry.id}
              className="px-3 py-2.5 rounded-lg border border-border/15 hover:border-border/30 bg-[var(--bg-hover)]/10 transition-colors group"
            >
              <div className="flex items-center gap-2">
                <KeyRound className="w-3 h-3 text-accent/60 flex-shrink-0" />
                <span className="text-[12px] font-medium text-foreground/80 flex-1 truncate">{entry.name}</span>
                <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/20 text-muted-foreground/50 font-medium">
                  {t(TYPE_LABEL_KEYS[entry.type] || entry.type)}
                </span>
                <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button className="p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-foreground" onClick={() => handleRevealToggle(entry.id)}>
                    {revealedIds.has(entry.id) ? <EyeOff className="w-3 h-3" /> : <Eye className="w-3 h-3" />}
                  </button>
                  <button className="p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-foreground" onClick={() => handleCopyValue(entry.id)}>
                    <Copy className="w-3 h-3" />
                  </button>
                  <button className="p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-accent" onClick={() => handleCopyRef(entry.name)}>
                    <Link2 className="w-3 h-3" />
                  </button>
                  <button className="p-1 rounded hover:bg-red-500/15 text-muted-foreground/40 hover:text-red-400" onClick={() => handleDelete(entry.id, entry.name)}>
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              </div>
              {(entry.username || entry.project || entry.notes) && (
                <div className="flex items-center gap-3 mt-1 ml-5 text-[10px] text-muted-foreground/40">
                  {entry.username && <span>@{entry.username}</span>}
                  {entry.project && <span className="text-accent/50">{entry.project}</span>}
                  {entry.notes && <span className="truncate max-w-[200px]">{entry.notes}</span>}
                </div>
              )}
              {revealedIds.has(entry.id) && revealedValues[entry.id] && (
                <pre className="mt-2 ml-5 text-[10px] font-mono bg-background/50 border border-border/15 rounded-md px-2.5 py-1.5 whitespace-pre-wrap break-all text-foreground/60 max-h-[100px] overflow-y-auto">
                  {revealedValues[entry.id]}
                </pre>
              )}
            </div>
          ))}
        </div>
      )}
      </div>
    </div>
  );
}
