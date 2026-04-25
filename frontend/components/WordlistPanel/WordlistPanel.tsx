import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { logAudit } from "@/lib/audit";
import {
  BookText,
  Copy,
  Eye,
  FileUp,
  Merge,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { copyToClipboard } from "@/lib/clipboard";
import { cn } from "@/lib/utils";
import { CustomSelect } from "@/components/ui/custom-select";

interface WordlistMeta {
  id: string;
  name: string;
  category: string;
  description: string;
  line_count: number;
  file_size: number;
  filename: string;
  tags: string[];
  created_at: number;
}

const CATEGORIES = [
  "passwords", "directories", "subdomains", "usernames",
  "extensions", "parameters", "fuzz", "custom", "merged",
];

const CAT_COLORS: Record<string, string> = {
  passwords: "text-red-400 bg-red-500/10",
  directories: "text-blue-400 bg-blue-500/10",
  subdomains: "text-cyan-400 bg-cyan-500/10",
  usernames: "text-green-400 bg-green-500/10",
  extensions: "text-purple-400 bg-purple-500/10",
  parameters: "text-orange-400 bg-orange-500/10",
  fuzz: "text-yellow-400 bg-yellow-500/10",
  custom: "text-slate-400 bg-slate-500/10",
  merged: "text-pink-400 bg-pink-500/10",
};

import { formatBytes as formatSize } from "@/lib/format";

export function WordlistPanel() {
  const [wordlists, setWordlists] = useState<WordlistMeta[]>([]);
  const [loading, setLoading] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [filterCat, setFilterCat] = useState("all");
  const [previewId, setPreviewId] = useState<string | null>(null);
  const [previewLines, setPreviewLines] = useState<string[]>([]);
  const [mergeIds, setMergeIds] = useState<Set<string>>(new Set());
  const [merging, setMerging] = useState(false);
  const [mergeName, setMergeName] = useState("");

  const [importForm, setImportForm] = useState({
    name: "",
    category: "custom",
    description: "",
    tags: "",
  });

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<WordlistMeta[]>("wordlist_list");
      setWordlists(Array.isArray(list) ? list : []);
    } catch {
      setWordlists([]);
    }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  const handleImportFile = useCallback(async () => {
    if (!importForm.name.trim()) return;

    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".txt,.lst,.dict,.wordlist,.csv";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = async () => {
        const b64 = (reader.result as string).split(",")[1];
        try {
          await invoke("wordlist_import", {
            name: importForm.name.trim(),
            category: importForm.category,
            description: importForm.description,
            contentBase64: b64,
            originalFilename: file.name,
            tags: importForm.tags ? importForm.tags.split(",").map((s) => s.trim()).filter(Boolean) : null,
          });
          setImportForm({ name: "", category: "custom", description: "", tags: "" });
          setShowImport(false);
          load();
          logAudit({ action: "wordlist_imported", category: "tools", details: importForm.name.trim() });
        } catch (e) {
          console.error("Import failed:", e);
        }
      };
      reader.readAsDataURL(file);
    };
    input.click();
  }, [importForm, load]);

  const handleDelete = useCallback(async (id: string) => {
    try {
      await invoke("wordlist_delete", { id });
      load();
    } catch { /* ignore */ }
  }, [load]);

  const handleDedup = useCallback(async (id: string) => {
    try {
      const updated = await invoke<WordlistMeta>("wordlist_deduplicate", { id });
      setWordlists((prev) => prev.map((w) => (w.id === id ? updated : w)));
    } catch (e) {
      console.error("Dedup failed:", e);
    }
  }, []);

  const handlePreview = useCallback(async (id: string) => {
    if (previewId === id) {
      setPreviewId(null);
      return;
    }
    try {
      const lines = await invoke<string[]>("wordlist_preview", { id, lines: 30 });
      setPreviewLines(lines);
      setPreviewId(id);
    } catch { /* ignore */ }
  }, [previewId]);

  const handleCopyPath = useCallback(async (id: string) => {
    try {
      const path = await invoke<string>("wordlist_path", { id });
      await copyToClipboard(path);
    } catch { /* ignore */ }
  }, []);

  const handleMerge = useCallback(async () => {
    if (mergeIds.size < 2 || !mergeName.trim()) return;
    setMerging(true);
    try {
      await invoke("wordlist_merge", {
        ids: Array.from(mergeIds),
        newName: mergeName.trim(),
        deduplicate: true,
      });
      setMergeIds(new Set());
      setMergeName("");
      load();
    } catch (e) {
      console.error("Merge failed:", e);
    }
    setMerging(false);
  }, [mergeIds, mergeName, load]);

  const toggleMergeSelect = useCallback((id: string) => {
    setMergeIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const safeWordlists = wordlists ?? [];
  const filtered = filterCat === "all" ? safeWordlists : safeWordlists.filter((w) => w.category === filterCat);

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/20">
        <BookText className="w-3.5 h-3.5 text-accent/70" />
        <span className="text-[11px] font-medium flex-1">Wordlists</span>
        <span className="text-[9px] text-muted-foreground/60">{safeWordlists.length} lists</span>
        {mergeIds.size >= 2 && (
          <div className="flex items-center gap-1">
            <input
              value={mergeName}
              onChange={(e) => setMergeName(e.target.value)}
              placeholder="Merged name..."
              className="text-[10px] w-24 px-1.5 py-0.5 bg-background border border-border/30 rounded outline-none"
            />
            <button
              onClick={handleMerge}
              disabled={merging || !mergeName.trim()}
              className="text-[9px] text-accent hover:text-accent/80 font-medium disabled:opacity-30"
            >
              <Merge className="w-3 h-3" />
            </button>
            <button onClick={() => setMergeIds(new Set())} className="text-[9px] text-muted-foreground/30">
              <X className="w-3 h-3" />
            </button>
          </div>
        )}
        <button onClick={() => setShowImport(true)} className="p-1 text-muted-foreground/30 hover:text-accent transition-colors">
          <Plus className="w-3 h-3" />
        </button>
        <button onClick={load} className="p-1 text-muted-foreground/30 hover:text-foreground transition-colors">
          <RefreshCw className={cn("w-3 h-3", loading && "animate-spin")} />
        </button>
      </div>

      {/* Category filter */}
      <div className="flex items-center gap-1 px-3 py-1.5 border-b border-border/10 overflow-x-auto">
        <button
          onClick={() => setFilterCat("all")}
          className={cn(
            "text-[9px] px-2 py-0.5 rounded-full transition-colors whitespace-nowrap",
            filterCat === "all" ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground"
          )}
        >
          All
        </button>
        {CATEGORIES.map((c) => (
          <button
            key={c}
            onClick={() => setFilterCat(c)}
            className={cn(
              "text-[9px] px-2 py-0.5 rounded-full transition-colors whitespace-nowrap capitalize",
              filterCat === c ? "bg-accent/15 text-accent" : "text-muted-foreground/30 hover:text-foreground"
            )}
          >
            {c}
          </button>
        ))}
      </div>

      {/* Import form */}
      {showImport && (
        <div className="px-3 py-2 border-b border-border/20 space-y-1.5 bg-muted/5">
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-medium">Import Wordlist</span>
            <button onClick={() => setShowImport(false)} className="text-muted-foreground/30 hover:text-foreground">
              <X className="w-3 h-3" />
            </button>
          </div>
          <input
            value={importForm.name}
            onChange={(e) => setImportForm((f) => ({ ...f, name: e.target.value }))}
            placeholder="Name..."
            className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
          />
          <div className="flex gap-1.5">
            <CustomSelect
              value={importForm.category}
              onChange={(v) => setImportForm((f) => ({ ...f, category: v }))}
              options={CATEGORIES.map((c) => ({ value: c, label: c }))}
              size="sm"
              className="flex-1"
            />
            <input
              value={importForm.tags}
              onChange={(e) => setImportForm((f) => ({ ...f, tags: e.target.value }))}
              placeholder="Tags (comma-sep)..."
              className="text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none flex-1"
            />
          </div>
          <input
            value={importForm.description}
            onChange={(e) => setImportForm((f) => ({ ...f, description: e.target.value }))}
            placeholder="Description..."
            className="w-full text-[10px] px-2 py-1 bg-background border border-border/30 rounded outline-none"
          />
          <button
            onClick={handleImportFile}
            disabled={!importForm.name.trim()}
            className="flex items-center gap-1.5 text-[10px] text-accent hover:text-accent/80 font-medium disabled:opacity-30"
          >
            <FileUp className="w-3 h-3" />
            Select File & Import
          </button>
        </div>
      )}

      {/* Wordlist items */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1">
        {filtered.length === 0 ? (
          <div className="text-center text-[11px] text-muted-foreground/30 py-12">
            {safeWordlists.length === 0 ? "No wordlists imported yet" : "No wordlists in this category"}
          </div>
        ) : (
          filtered.map((wl) => (
            <div key={wl.id} className="group">
              <div className="flex items-center gap-2 py-1.5 px-2 rounded hover:bg-muted/5 transition-colors">
                <input
                  type="checkbox"
                  checked={mergeIds.has(wl.id)}
                  onChange={() => toggleMergeSelect(wl.id)}
                  className="w-3 h-3 rounded opacity-0 group-hover:opacity-100 transition-opacity accent-accent"
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] font-medium truncate">{wl.name}</span>
                    <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full capitalize", CAT_COLORS[wl.category] || "text-slate-400 bg-slate-500/10")}>
                      {wl.category}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-[9px] text-muted-foreground/40">{wl.line_count.toLocaleString()} lines</span>
                    <span className="text-[9px] text-muted-foreground/25">•</span>
                    <span className="text-[9px] text-muted-foreground/40">{formatSize(wl.file_size)}</span>
                    {wl.tags.length > 0 && (
                      <>
                        <span className="text-[9px] text-muted-foreground/25">•</span>
                        <span className="text-[8px] text-muted-foreground/30">{wl.tags.join(", ")}</span>
                      </>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button onClick={() => handlePreview(wl.id)} className="p-1 text-muted-foreground/30 hover:text-foreground transition-colors" title="Preview">
                    <Eye className="w-3 h-3" />
                  </button>
                  <button onClick={() => handleCopyPath(wl.id)} className="p-1 text-muted-foreground/30 hover:text-foreground transition-colors" title="Copy path">
                    <Copy className="w-3 h-3" />
                  </button>
                  <button onClick={() => handleDedup(wl.id)} className="p-1 text-muted-foreground/30 hover:text-accent transition-colors" title="Deduplicate">
                    <RefreshCw className="w-3 h-3" />
                  </button>
                  <button onClick={() => handleDelete(wl.id)} className="p-1 text-muted-foreground/30 hover:text-red-400 transition-colors" title="Delete">
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              </div>

              {previewId === wl.id && (
                <div className="mx-2 mb-1 rounded border border-border/20 bg-background/80 max-h-48 overflow-y-auto">
                  <pre className="text-[9px] text-muted-foreground/60 p-2 font-mono">
                    {previewLines.join("\n")}
                  </pre>
                </div>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
