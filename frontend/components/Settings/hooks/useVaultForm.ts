import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { logAudit } from "@/lib/audit";
import { copyToClipboard } from "@/lib/clipboard";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";

export interface VaultEntrySafe {
  id: string;
  name: string;
  type: "password" | "token" | "ssh_key" | "api_key" | "cookie" | "certificate" | "other";
  username: string;
  notes: string;
  project: string;
  tags: string[];
  status: string;
  source_url: string;
  last_validated_at: number | null;
  created_at: number;
  updated_at: number;
}

export const ENTRY_TYPES = [
  "password", "token", "api_key", "ssh_key", "cookie", "certificate", "other",
] as const;

export const TYPE_LABEL_KEYS: Record<string, string> = {
  password: "vault.password",
  token: "vault.token",
  ssh_key: "vault.sshKey",
  api_key: "vault.apiKey",
  cookie: "vault.cookie",
  certificate: "vault.certificate",
  other: "vault.other",
};

export function useVaultForm() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [entries, setEntries] = useState<VaultEntrySafe[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [revealedIds, setRevealedIds] = useState<Set<string>>(new Set());
  const [revealedValues, setRevealedValues] = useState<Record<string, string>>({});
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set("__ungrouped__"));

  const groupedEntries = useMemo(() => {
    const groups = new Map<string, VaultEntrySafe[]>();
    for (const e of entries) {
      let key = e.project || "__ungrouped__";
      if (key === "__ungrouped__" && e.tags?.includes("auto-captured")) {
        const dashIdx = e.name.indexOf(" - ");
        if (dashIdx > 0) key = e.name.slice(0, dashIdx);
      }
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(e);
    }
    for (const [, group] of groups) {
      group.sort((a, b) => b.updated_at - a.updated_at);
    }
    return [...groups.entries()].sort(([a], [b]) => {
      if (a === "__ungrouped__") return 1;
      if (b === "__ungrouped__") return -1;
      return a.localeCompare(b);
    });
  }, [entries]);

  const [addForm, setAddForm] = useState({
    name: "", type: "password" as string, value: "", username: "", notes: "", project: "", tags: "",
  });

  const [validatingIds, setValidatingIds] = useState<Set<string>>(new Set());

  const loadEntries = useCallback(async () => {
    try {
      const data = await invoke<VaultEntrySafe[]>("vault_list", { projectPath: getProjectPath() });
      setEntries(Array.isArray(data) ? data : []);
    } catch (e) {
      console.error("Failed to load vault:", e);
    }
  }, []);

  const handleValidate = useCallback(async (id: string) => {
    setValidatingIds((prev) => new Set(prev).add(id));
    try {
      await invoke<string>("vault_validate", { id, projectPath: getProjectPath() });
      await loadEntries();
    } catch (e) {
      console.error("Failed to validate:", e);
    } finally {
      setValidatingIds((prev) => { const n = new Set(prev); n.delete(id); return n; });
    }
  }, [loadEntries]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    (async () => {
      unlisten = await listen<{ host: string }>("credential-expired", async (event) => {
        const host = event.payload.host;
        for (const entry of entries) {
          const entryHost = entry.project || entry.name.split(" - ")[0];
          if (entryHost.includes(host) || host.includes(entryHost)) {
            if (entry.status !== "expired") {
              await invoke("vault_update_status", { id: entry.id, status: "expired", projectPath: getProjectPath() }).catch(() => {});
            }
          }
        }
        loadEntries();
      });
    })();
    return () => { unlisten?.(); };
  }, [entries, loadEntries]);

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
      logAudit({ action: "vault_entry_added", category: "vault", details: addForm.name.trim() });
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
      logAudit({ action: "vault_entry_deleted", category: "vault", details: name, entityType: "vault", entityId: id });
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

  const handleCopyAll = useCallback(async (entry: VaultEntrySafe) => {
    try {
      const value = await invoke<string>("vault_get_value", { id: entry.id, projectPath: getProjectPath() });
      const lines = [
        `Name: ${entry.name}`,
        `Type: ${entry.type}`,
      ];
      if (entry.username) lines.push(`Username: ${entry.username}`);
      lines.push(`Value: ${value}`);
      if (entry.notes) lines.push(`Notes: ${entry.notes}`);
      if (entry.project) lines.push(`Project: ${entry.project}`);
      if (entry.tags.length > 0) lines.push(`Tags: ${entry.tags.join(", ")}`);
      await copyToClipboard(lines.join("\n"));
    } catch { /* ignore */ }
  }, []);

  const handleCopyRef = useCallback(async (name: string) => {
    const ref = `{{vault:${name}}}`;
    await copyToClipboard(ref);
  }, []);

  const toggleGroup = useCallback((groupKey: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(groupKey)) next.delete(groupKey);
      else next.add(groupKey);
      return next;
    });
  }, []);

  const toggleExpanded = useCallback((id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  return {
    entries,
    showAdd,
    setShowAdd,
    revealedIds,
    revealedValues,
    expandedIds,
    expandedGroups,
    groupedEntries,
    addForm,
    setAddForm,
    validatingIds,
    loadEntries,
    handleValidate,
    handleAdd,
    handleDelete,
    handleRevealToggle,
    handleCopyAll,
    handleCopyRef,
    toggleGroup,
    toggleExpanded,
  };
}
