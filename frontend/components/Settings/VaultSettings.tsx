import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown, ChevronRight, Copy, Eye, EyeOff, Globe, KeyRound, Link2, Loader2, Plus, ShieldCheck, ShieldX, Trash2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useVaultForm, ENTRY_TYPES, TYPE_LABEL_KEYS } from "./hooks/useVaultForm";

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

function InlineNotes({ entryId, initial, onSaved }: { entryId: string; initial: string; onSaved: () => void }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => { setValue(initial); }, [initial]);
  useEffect(() => { if (editing) inputRef.current?.focus(); }, [editing]);

  const save = useCallback(async () => {
    setEditing(false);
    if (value === initial) return;
    try {
      await invoke("vault_update", { id: entryId, notes: value, projectPath: getProjectPath() });
      onSaved();
    } catch (e) {
      console.error("Failed to update notes:", e);
      setValue(initial);
    }
  }, [entryId, value, initial, onSaved]);

  if (editing) {
    return (
      <input
        ref={inputRef}
        className="text-[10px] bg-background border border-accent/40 rounded px-2 py-1 outline-none text-foreground/70 w-full"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => { if (e.key === "Enter") save(); if (e.key === "Escape") { setValue(initial); setEditing(false); } }}
        placeholder="Add a note..."
      />
    );
  }

  return (
    <span
      className={cn(
        "cursor-pointer rounded px-1 py-0.5 -mx-1 hover:bg-[var(--bg-hover)]/40 transition-colors",
        value ? "text-foreground/60 break-all" : "text-muted-foreground/25 italic"
      )}
      onClick={() => setEditing(true)}
    >
      {value || "Click to add note..."}
    </span>
  );
}

export function VaultSettings() {
  const { t } = useTranslation();
  const {
    entries, showAdd, setShowAdd, revealedIds, revealedValues,
    expandedIds, expandedGroups, groupedEntries, addForm, setAddForm,
    validatingIds, loadEntries, handleValidate, handleAdd, handleDelete,
    handleRevealToggle, handleCopyAll, handleCopyRef, toggleGroup, toggleExpanded,
  } = useVaultForm();

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
        <div className="space-y-2">
          {groupedEntries.map(([groupKey, groupItems]) => {
            const isGroupExpanded = expandedGroups.has(groupKey);
            const isUngrouped = groupKey === "__ungrouped__";
            const groupLabel = isUngrouped ? t("vault.manual", "Manual") : groupKey;
            return (
              <div key={groupKey}>
                <div
                  className="flex items-center gap-1.5 px-2 py-1.5 cursor-pointer hover:bg-[var(--bg-hover)]/40 transition-colors rounded-md"
                  onClick={() => toggleGroup(groupKey)}
                >
                  <ChevronRight className={cn("w-3 h-3 text-muted-foreground/40 transition-transform flex-shrink-0", isGroupExpanded && "rotate-90")} />
                  {isUngrouped ? (
                    <KeyRound className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
                  ) : (
                    <Globe className="w-3 h-3 text-blue-400 flex-shrink-0" />
                  )}
                  <span className="text-[11px] font-medium text-foreground/70 flex-1 truncate">{groupLabel}</span>
                  <span className="text-[9px] text-muted-foreground/30 tabular-nums">{groupItems.length}</span>
                </div>
                {isGroupExpanded && (
                  <div className="space-y-1 mt-1 ml-2">
                    {groupItems.map((entry) => {
                      const isExpanded = expandedIds.has(entry.id);
                      const updatedDate = new Date(entry.updated_at * 1000);
                      const timeStr = updatedDate.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
                      return (
                        <div
                          key={entry.id}
                          className={cn(
                            "rounded-lg border transition-colors group",
                            isExpanded ? "border-border/30 bg-[var(--bg-hover)]/15" : "border-border/15 hover:border-border/30 bg-[var(--bg-hover)]/10"
                          )}
                        >
                          <div
                            className="flex items-center gap-2 px-3 py-2 cursor-pointer"
                            onClick={() => {
                              toggleExpanded(entry.id);
                              if (!revealedIds.has(entry.id)) handleRevealToggle(entry.id);
                            }}
                          >
                            {isExpanded ? (
                              <ChevronDown className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
                            ) : (
                              <ChevronRight className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
                            )}
                            <KeyRound className="w-3 h-3 text-accent/60 flex-shrink-0" />
                            <span className="text-[11px] font-medium text-foreground/80 flex-1 truncate">{entry.name}</span>
                            <span className="text-[8px] text-muted-foreground/30 flex-shrink-0 tabular-nums">{timeStr}</span>
                            <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/20 text-muted-foreground/50 font-medium">
                              {t(TYPE_LABEL_KEYS[entry.type] || entry.type)}
                            </span>
                            {entry.status === "valid" && (
                              <span className="flex items-center gap-0.5 text-[8px] px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 font-medium">
                                <ShieldCheck className="w-2.5 h-2.5" />
                                {t("vault.valid", "Valid")}
                              </span>
                            )}
                            {entry.status === "expired" && (
                              <span className="flex items-center gap-0.5 text-[8px] px-1.5 py-0.5 rounded-full bg-red-500/15 text-red-400 font-medium">
                                <ShieldX className="w-2.5 h-2.5" />
                                {t("vault.expired", "Expired")}
                              </span>
                            )}
                            <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity" onClick={(e) => e.stopPropagation()}>
                              {entry.source_url && (
                                <button
                                  className={cn("p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-accent", validatingIds.has(entry.id) && "pointer-events-none")}
                                  title={t("vault.validate", "Validate credential")}
                                  onClick={() => handleValidate(entry.id)}
                                >
                                  {validatingIds.has(entry.id) ? <Loader2 className="w-3 h-3 animate-spin" /> : <ShieldCheck className="w-3 h-3" />}
                                </button>
                              )}
                              <button className="p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-foreground" onClick={() => handleCopyAll(entry)}>
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
                          {!isExpanded && (entry.username || entry.notes) && (
                            <div className="flex items-center gap-3 px-3 pb-2 ml-[22px] text-[10px] text-muted-foreground/40">
                              {entry.username && <span>@{entry.username}</span>}
                              {entry.notes && <span className="truncate max-w-[260px]">{entry.notes}</span>}
                            </div>
                          )}
                          {isExpanded && (
                            <div className="px-3 pb-3 ml-[22px] space-y-2 border-t border-border/10 pt-2">
                              <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[10px]">
                                <span className="text-muted-foreground/50 font-medium">{t("vault.name", "Name")}</span>
                                <span className="text-foreground/70 font-mono truncate">{entry.name}</span>
                                <span className="text-muted-foreground/50 font-medium">{t("vault.type", "Type")}</span>
                                <span className="text-foreground/70">{t(TYPE_LABEL_KEYS[entry.type] || entry.type)}</span>
                                {entry.username && (
                                  <>
                                    <span className="text-muted-foreground/50 font-medium">{t("vault.username", "Username")}</span>
                                    <span className="text-foreground/70 font-mono">{entry.username}</span>
                                  </>
                                )}
                                <span className="text-muted-foreground/50 font-medium">{t("vault.value", "Value")}</span>
                                <div className="flex items-center gap-1.5 min-w-0">
                                  {revealedIds.has(entry.id) && revealedValues[entry.id] ? (
                                    <pre className="text-[10px] font-mono bg-background/50 border border-border/15 rounded px-2 py-1 whitespace-pre-wrap break-all text-foreground/60 max-h-[80px] overflow-y-auto flex-1 min-w-0">
                                      {revealedValues[entry.id]}
                                    </pre>
                                  ) : (
                                    <span className="text-muted-foreground/30 font-mono">••••••••</span>
                                  )}
                                  <button
                                    className="p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-foreground flex-shrink-0"
                                    onClick={() => handleRevealToggle(entry.id)}
                                  >
                                    {revealedIds.has(entry.id) ? <EyeOff className="w-3 h-3" /> : <Eye className="w-3 h-3" />}
                                  </button>
                                </div>
                                <span className="text-muted-foreground/50 font-medium">{t("vault.notes", "Notes")}</span>
                                <InlineNotes entryId={entry.id} initial={entry.notes} onSaved={loadEntries} />
                                {entry.tags.length > 0 && (
                                  <>
                                    <span className="text-muted-foreground/50 font-medium">Tags</span>
                                    <div className="flex flex-wrap gap-1">
                                      {entry.tags.map((tag) => (
                                        <span key={tag} className="text-[9px] px-1.5 py-0.5 rounded-full bg-accent/10 text-accent/70">{tag}</span>
                                      ))}
                                    </div>
                                  </>
                                )}
                                {entry.status !== "unknown" && (
                                  <>
                                    <span className="text-muted-foreground/50 font-medium">{t("vault.status", "Status")}</span>
                                    <span className={cn(
                                      "text-[9px] font-medium",
                                      entry.status === "valid" ? "text-emerald-400" : entry.status === "expired" ? "text-red-400" : "text-muted-foreground/40"
                                    )}>
                                      {entry.status === "valid" ? t("vault.valid", "Valid") : entry.status === "expired" ? t("vault.expired", "Expired") : t("vault.unknown", "Unknown")}
                                      {entry.last_validated_at && (
                                        <span className="text-muted-foreground/30 ml-2 font-normal">
                                          ({t("vault.lastChecked", "checked")} {new Date(entry.last_validated_at * 1000).toLocaleString()})
                                        </span>
                                      )}
                                    </span>
                                  </>
                                )}
                                <span className="text-muted-foreground/50 font-medium">{t("vault.captured", "Captured")}</span>
                                <span className="text-foreground/50 text-[9px] tabular-nums">
                                  {new Date(entry.created_at * 1000).toLocaleString()}
                                  {entry.updated_at !== entry.created_at && (
                                    <span className="text-muted-foreground/30 ml-2">
                                      ({t("vault.updated", "updated")} {new Date(entry.updated_at * 1000).toLocaleString()})
                                    </span>
                                  )}
                                </span>
                              </div>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      </div>
    </div>
  );
}
