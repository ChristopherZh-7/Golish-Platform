import { useCallback, useEffect, useRef, useState } from "react";
import {
  Bold, BookOpen, ChevronDown, ChevronRight, Code, FileCode2, FileText, FolderOpen, FolderPlus,
  Heading1, Heading2, Heading3, Italic, Link2, List, ListOrdered, Loader2, Plus, Quote,
  RefreshCw, Search, Strikethrough, Table, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { Markdown } from "@/components/Markdown/Markdown";
import { wikiApi, type WikiEntry, type WikiSearchResult, type WikiStats } from "@/lib/wiki";

function extToLang(name: string): string | null {
  const ext = name.split(".").pop()?.toLowerCase();
  const map: Record<string, string> = {
    py: "python", sh: "bash", bash: "bash", zsh: "bash", go: "go", rs: "rust",
    rb: "ruby", pl: "perl", js: "javascript", ts: "typescript", jsx: "jsx", tsx: "tsx",
    c: "c", cpp: "cpp", h: "c", hpp: "cpp", java: "java", cs: "csharp",
    swift: "swift", kt: "kotlin", lua: "lua", r: "r", ps1: "powershell",
    bat: "batch", php: "php", html: "html", css: "css", xml: "xml",
    json: "json", yaml: "yaml", yml: "yaml", toml: "toml", ini: "ini",
    sql: "sql", graphql: "graphql", proto: "protobuf", nse: "lua",
    dockerfile: "dockerfile", makefile: "makefile",
  };
  return ext ? (map[ext] || null) : null;
}

function isMarkdown(name: string): boolean {
  return name.endsWith(".md");
}

function FileIcon({ name, className }: { name: string; className?: string }) {
  if (isMarkdown(name)) return <FileText className={cn("text-blue-400/60", className)} />;
  return <FileCode2 className={cn("text-emerald-400/60", className)} />;
}

import React from "react";

const WikiEditor = React.forwardRef<
  HTMLTextAreaElement,
  { value: string; onChange: (v: string) => void; language: string | null }
>(function WikiEditor({ value, onChange, language }, ref) {
  const lines = value.split("\n");
  const lineCount = lines.length;
  const internalRef = useRef<HTMLTextAreaElement>(null);
  const lineNumRef = useRef<HTMLDivElement>(null);
  const resolvedRef = (ref as React.MutableRefObject<HTMLTextAreaElement | null>) || internalRef;

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Tab") {
        e.preventDefault();
        const ta = e.currentTarget;
        const start = ta.selectionStart;
        const end = ta.selectionEnd;
        const newValue = value.substring(0, start) + "  " + value.substring(end);
        onChange(newValue);
        requestAnimationFrame(() => {
          ta.selectionStart = ta.selectionEnd = start + 2;
        });
      }
    },
    [value, onChange],
  );

  const handleScroll = useCallback(() => {
    if (resolvedRef.current && lineNumRef.current) {
      lineNumRef.current.scrollTop = resolvedRef.current.scrollTop;
    }
  }, [resolvedRef]);

  return (
    <div className="flex-1 flex min-h-0 overflow-hidden">
      <div
        ref={lineNumRef}
        className="flex-shrink-0 overflow-hidden select-none pt-4 pb-4 text-right pr-2 pl-2 text-[12px] font-mono leading-[1.7] text-muted-foreground/25 border-r border-border/8"
        style={{ width: "48px" }}
        aria-hidden
      >
        {Array.from({ length: lineCount }, (_, i) => (
          <div key={i}>{i + 1}</div>
        ))}
      </div>
      <textarea
        ref={resolvedRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={handleScroll}
        onKeyDown={handleKeyDown}
        spellCheck={false}
        className={cn(
          "flex-1 px-4 py-4 text-[13px] font-mono leading-[1.7] bg-transparent text-foreground outline-none resize-none overflow-y-auto",
          language && "text-emerald-100/90",
        )}
        style={{ tabSize: 2 }}
      />
    </div>
  );
});

interface ToolbarAction {
  icon: typeof Bold;
  label: string;
  prefix: string;
  suffix: string;
  block?: boolean;
}

const MD_ACTIONS: ToolbarAction[] = [
  { icon: Bold, label: "Bold", prefix: "**", suffix: "**" },
  { icon: Italic, label: "Italic", prefix: "_", suffix: "_" },
  { icon: Strikethrough, label: "Strikethrough", prefix: "~~", suffix: "~~" },
  { icon: Code, label: "Inline code", prefix: "`", suffix: "`" },
  { icon: Heading1, label: "Heading 1", prefix: "# ", suffix: "", block: true },
  { icon: Heading2, label: "Heading 2", prefix: "## ", suffix: "", block: true },
  { icon: Heading3, label: "Heading 3", prefix: "### ", suffix: "", block: true },
  { icon: List, label: "Bullet list", prefix: "- ", suffix: "", block: true },
  { icon: ListOrdered, label: "Numbered list", prefix: "1. ", suffix: "", block: true },
  { icon: Quote, label: "Blockquote", prefix: "> ", suffix: "", block: true },
  { icon: Link2, label: "Link", prefix: "[", suffix: "](url)" },
  { icon: Table, label: "Table", prefix: "\n| Header | Header |\n| --- | --- |\n| Cell | Cell |\n", suffix: "", block: true },
];

function MarkdownToolbar({
  textareaRef,
  onChange,
  content,
}: {
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  onChange: (v: string) => void;
  content: string;
}) {
  const applyAction = useCallback(
    (action: ToolbarAction) => {
      const ta = textareaRef.current;
      if (!ta) return;
      const start = ta.selectionStart;
      const end = ta.selectionEnd;
      const selected = content.substring(start, end) || (action.block ? "" : "text");

      let newContent: string;
      let cursorPos: number;

      if (action.block && !selected) {
        const lineStart = content.lastIndexOf("\n", start - 1) + 1;
        newContent = content.substring(0, lineStart) + action.prefix + content.substring(lineStart);
        cursorPos = lineStart + action.prefix.length;
      } else {
        newContent =
          content.substring(0, start) +
          action.prefix +
          selected +
          action.suffix +
          content.substring(end);
        cursorPos = start + action.prefix.length + selected.length;
      }

      onChange(newContent);
      requestAnimationFrame(() => {
        ta.focus();
        ta.selectionStart = ta.selectionEnd = cursorPos;
      });
    },
    [textareaRef, onChange, content],
  );

  return (
    <div className="flex items-center gap-0.5 px-4 py-1.5 border-b border-border/8 flex-shrink-0 flex-wrap">
      {MD_ACTIONS.map((action, i) => (
        <React.Fragment key={action.label}>
          {(i === 4 || i === 7 || i === 10) && (
            <div className="w-px h-4 bg-border/15 mx-1" />
          )}
          <button
            type="button"
            onClick={() => applyAction(action)}
            title={action.label}
            className="p-1 rounded hover:bg-muted/30 text-muted-foreground/40 hover:text-muted-foreground/80 transition-colors"
          >
            <action.icon className="w-3.5 h-3.5" />
          </button>
        </React.Fragment>
      ))}
    </div>
  );
}

const CATEGORY_META: Record<string, { icon: string; label: string; color: string }> = {
  products: { icon: "📦", label: "Products", color: "text-blue-400" },
  techniques: { icon: "⚔️", label: "Techniques", color: "text-red-400" },
  pocs: { icon: "🔧", label: "PoCs", color: "text-amber-400" },
  experience: { icon: "📝", label: "Experience", color: "text-green-400" },
  analysis: { icon: "🔬", label: "Analysis", color: "text-purple-400" },
  uncategorized: { icon: "📄", label: "Other", color: "text-muted-foreground" },
};

const STATUS_COLORS: Record<string, string> = {
  draft: "bg-yellow-500",
  partial: "bg-orange-500",
  complete: "bg-green-500",
  "needs-poc": "bg-blue-500",
  verified: "bg-emerald-500",
};

function WikiDashboard({ stats, onOpenPage }: { stats: WikiStats | null; onOpenPage: (path: string) => void }) {
  if (!stats) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" />
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto px-8 py-6">
      {/* Stats cards */}
      <div className="grid grid-cols-4 gap-3 mb-6">
        {[
          { label: "Pages", value: stats.total_pages, color: "text-accent" },
          { label: "Words", value: stats.total_words.toLocaleString(), color: "text-blue-400" },
          { label: "Categories", value: Object.keys(stats.by_category).length, color: "text-emerald-400" },
          { label: "Orphans", value: stats.orphan_count, color: stats.orphan_count > 0 ? "text-yellow-400" : "text-green-400" },
        ].map((s) => (
          <div key={s.label} className="rounded-lg border border-border/10 bg-muted/5 px-4 py-3">
            <div className={cn("text-[20px] font-semibold", s.color)}>{s.value}</div>
            <div className="text-[10px] text-muted-foreground/40 mt-0.5">{s.label}</div>
          </div>
        ))}
      </div>

      <div className="grid grid-cols-2 gap-6">
        {/* Category breakdown */}
        <div>
          <h3 className="text-[12px] font-medium text-foreground/70 mb-3">By Category</h3>
          <div className="space-y-2">
            {Object.entries(stats.by_category)
              .sort(([, a], [, b]) => b - a)
              .map(([cat, count]) => {
                const meta = CATEGORY_META[cat] || CATEGORY_META.uncategorized;
                const pct = stats.total_pages > 0 ? (count / stats.total_pages) * 100 : 0;
                return (
                  <div key={cat} className="flex items-center gap-2">
                    <span className="text-[12px] w-5 text-center">{meta.icon}</span>
                    <span className={cn("text-[11px] w-24", meta.color)}>{meta.label}</span>
                    <div className="flex-1 h-1.5 bg-muted/10 rounded-full overflow-hidden">
                      <div className={cn("h-full rounded-full bg-accent/40")} style={{ width: `${pct}%` }} />
                    </div>
                    <span className="text-[10px] text-muted-foreground/40 w-8 text-right">{count}</span>
                  </div>
                );
              })}
          </div>
        </div>

        {/* Status breakdown */}
        <div>
          <h3 className="text-[12px] font-medium text-foreground/70 mb-3">By Status</h3>
          <div className="flex flex-wrap gap-2">
            {Object.entries(stats.by_status)
              .sort(([, a], [, b]) => b - a)
              .map(([status, count]) => (
                <div key={status} className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border border-border/10 bg-muted/5">
                  <span className={cn("w-2 h-2 rounded-full", STATUS_COLORS[status] || "bg-muted-foreground")} />
                  <span className="text-[10px] text-foreground/60 capitalize">{status}</span>
                  <span className="text-[10px] text-muted-foreground/30 ml-1">{count}</span>
                </div>
              ))}
          </div>
        </div>
      </div>

      {/* Recent changes */}
      {stats.recent_changes.length > 0 && (
        <div className="mt-6">
          <h3 className="text-[12px] font-medium text-foreground/70 mb-3">Recent Changes</h3>
          <div className="space-y-1">
            {stats.recent_changes.map((ch) => {
              const meta = CATEGORY_META[ch.category] || CATEGORY_META.uncategorized;
              const actionColors: Record<string, string> = {
                create: "text-green-400 bg-green-500/10",
                update: "text-blue-400 bg-blue-500/10",
                delete: "text-red-400 bg-red-500/10",
                link: "text-accent bg-accent/10",
                unlink: "text-orange-400 bg-orange-500/10",
              };
              return (
                <button
                  key={ch.id}
                  onClick={() => onOpenPage(ch.page_path)}
                  className="flex items-center gap-2 w-full px-3 py-2 rounded-lg hover:bg-muted/10 transition-colors text-left"
                >
                  <span className={cn("text-[7px] px-1.5 py-0.5 rounded uppercase font-medium", actionColors[ch.action] || "text-muted-foreground/40 bg-muted/10")}>
                    {ch.action}
                  </span>
                  <span className="text-[10px]">{meta.icon}</span>
                  <span className="text-[11px] text-foreground/70 truncate flex-1">{ch.title || ch.page_path}</span>
                  <span className="text-[9px] text-muted-foreground/25 flex-shrink-0">
                    {new Date(ch.created_at).toLocaleDateString()} {new Date(ch.created_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </span>
                  <span className="text-[8px] text-muted-foreground/20 flex-shrink-0">{ch.actor}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

export function WikiPanel({ initialPath }: { initialPath?: string | null }) {
  const { t } = useTranslation();
  const [tree, setTree] = useState<WikiEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [activeFileName, setActiveFileName] = useState<string>("");
  const [content, setContent] = useState("");
  const [originalContent, setOriginalContent] = useState("");
  const [dirty, setDirty] = useState(false);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const [search, setSearch] = useState("");
  const [searchResults, setSearchResults] = useState<WikiSearchResult[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<WikiEntry | null>(null);
  const [wikiStats, setWikiStats] = useState<WikiStats | null>(null);

  // Inline creation
  const [creating, setCreating] = useState<{ type: "file" | "folder"; parentPath: string } | null>(null);
  const [newName, setNewName] = useState("");
  const newNameRef = useRef<HTMLInputElement>(null);

  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadTree = useCallback(async () => {
    setLoading(true);
    try {
      const data = await wikiApi.list();
      setTree(Array.isArray(data) ? data : []);
    } catch (e) {
      setError(t("wiki.loadFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => { loadTree(); }, [loadTree]);

  // Auto-reindex on first load to fix uncategorized pages
  const reindexedRef = useRef(false);
  useEffect(() => {
    if (reindexedRef.current) return;
    reindexedRef.current = true;
    wikiApi.reindex()
      .then(() => wikiApi.statsFull().then(setWikiStats).catch(() => {}))
      .catch(() => {});
  }, []);

  useEffect(() => {
    wikiApi.statsFull()
      .then(setWikiStats)
      .catch(() => setWikiStats(null));
  }, [tree]);

  useEffect(() => {
    if (initialPath && !loading) {
      openFile(initialPath);
    }
  }, [initialPath, loading]);

  const openFile = useCallback(async (path: string, fileName?: string) => {
    if (dirty && activePath) {
      try { await wikiApi.write(activePath, content); } catch { /* ignore */ }
    }
    try {
      const data: string = await wikiApi.read(path);
      setActivePath(path);
      setActiveFileName(fileName || path.split("/").pop() || path);
      setContent(data);
      setOriginalContent(data);
      setDirty(false);
      setSearchResults(null);
    } catch (e) {
      setError(t("wiki.loadFailed", { error: String(e) }));
    }
  }, [activePath, content, dirty, t]);

  const handleContentChange = useCallback((value: string) => {
    setContent(value);
    setDirty(value !== originalContent);
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(async () => {
      if (activePath) {
        try {
          await wikiApi.write(activePath, value);
          setOriginalContent(value);
          setDirty(false);
        } catch (e) {
          setError(t("wiki.saveFailed", { error: String(e) }));
        }
      }
    }, 1500);
  }, [activePath, originalContent, t]);

  const handleSearch = useCallback(async (q: string) => {
    setSearch(q);
    if (!q.trim()) { setSearchResults(null); return; }
    setSearching(true);
    try {
      const results = await wikiApi.search(q.trim());
      setSearchResults(results);
    } catch { setSearchResults([]); }
    finally { setSearching(false); }
  }, []);

  const startCreate = useCallback((type: "file" | "folder", parentPath: string) => {
    setCreating({ type, parentPath });
    setNewName("");
    if (parentPath) setExpandedDirs((prev) => new Set([...prev, parentPath]));
    requestAnimationFrame(() => newNameRef.current?.focus());
  }, []);

  const confirmCreate = useCallback(async () => {
    if (!creating || !newName.trim()) { setCreating(null); return; }
    const name = newName.trim();
    const path = creating.parentPath ? `${creating.parentPath}/${name}` : name;
    try {
      if (creating.type === "folder") {
        await wikiApi.createDir(path);
      } else {
        const filePath = name.includes(".") ? path : `${path}.md`;
        await wikiApi.write(filePath, name.endsWith(".md") || !name.includes(".") ? `# ${name.replace(/\.\w+$/, "")}\n\n` : "");
        await loadTree();
        setCreating(null);
        setNewName("");
        openFile(filePath, name.includes(".") ? name : `${name}.md`);
        return;
      }
      await loadTree();
    } catch (e) { setError(String(e)); }
    setCreating(null);
    setNewName("");
  }, [creating, newName, loadTree, openFile]);


  const handleDelete = useCallback(async () => {
    if (!deleteTarget) return;
    try {
      await wikiApi.delete(deleteTarget.path);
      if (activePath === deleteTarget.path || activePath?.startsWith(deleteTarget.path + "/")) {
        setActivePath(null);
        setContent("");
        setOriginalContent("");
        setDirty(false);
      }
      await loadTree();
    } catch (e) { setError(String(e)); }
    setDeleteTarget(null);
  }, [deleteTarget, activePath, loadTree]);

  const toggleDir = useCallback((path: string) => {
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
  }, []);

  const flatFileCount = useCallback((entries: WikiEntry[]): number => {
    if (!Array.isArray(entries)) return 0;
    let count = 0;
    for (const e of entries) {
      if (e.is_dir && e.children) count += flatFileCount(e.children);
      else if (!e.is_dir) count++;
    }
    return count;
  }, []);

  const fileCount = flatFileCount(tree);
  const isMd = activePath ? isMarkdown(activePath) : true;
  const codeLang = activePath ? extToLang(activePath) : null;

  const TreeNode = ({ entry, depth }: { entry: WikiEntry; depth: number }) => {
    const isExpanded = expandedDirs.has(entry.path);
    const isActive = activePath === entry.path;
    const pl = 8 + depth * 16;

    if (entry.is_dir) {
      return (
        <div>
          <div
            className={cn(
              "group flex items-center gap-1.5 py-1 pr-2 rounded-md cursor-pointer transition-colors",
              "text-foreground/70 hover:bg-[var(--bg-hover)]"
            )}
            style={{ paddingLeft: pl }}
            onClick={() => toggleDir(entry.path)}
            onContextMenu={(e) => { e.preventDefault(); setDeleteTarget(entry); }}
          >
            {isExpanded
              ? <ChevronDown className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
              : <ChevronRight className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />}
            <FolderOpen className="w-3.5 h-3.5 text-amber-400/70 flex-shrink-0" />
            <span className="text-[12px] truncate flex-1">{entry.name}</span>
            <button type="button" onClick={(e) => { e.stopPropagation(); startCreate("file", entry.path); }}
              className="p-0.5 rounded opacity-0 group-hover:opacity-40 hover:!opacity-100 hover:text-accent transition-all">
              <Plus className="w-3 h-3" />
            </button>
            <button type="button" onClick={(e) => { e.stopPropagation(); startCreate("folder", entry.path); }}
              className="p-0.5 rounded opacity-0 group-hover:opacity-40 hover:!opacity-100 hover:text-accent transition-all">
              <FolderPlus className="w-3 h-3" />
            </button>
          </div>
          {isExpanded && (
            <div>
              {creating && creating.parentPath === entry.path && <CreateInput depth={depth + 1} />}
              {entry.children?.map((child) => <TreeNode key={child.path} entry={child} depth={depth + 1} />)}
            </div>
          )}
        </div>
      );
    }

    return (
      <div
        className={cn(
          "group flex items-center gap-1.5 py-1 pr-2 rounded-md cursor-pointer transition-colors",
          isActive ? "bg-accent/10 text-accent" : "text-foreground/70 hover:bg-[var(--bg-hover)]"
        )}
        style={{ paddingLeft: pl }}
        onClick={() => openFile(entry.path, entry.name)}
        onContextMenu={(e) => { e.preventDefault(); setDeleteTarget(entry); }}
      >
        <FileIcon name={entry.name} className="w-3.5 h-3.5 flex-shrink-0" />
        <span className="text-[12px] truncate flex-1">{entry.name}</span>
        <button type="button" onClick={(e) => { e.stopPropagation(); setDeleteTarget(entry); }}
          className="p-0.5 rounded opacity-0 group-hover:opacity-40 hover:!opacity-100 hover:text-destructive transition-all">
          <Trash2 className="w-2.5 h-2.5" />
        </button>
      </div>
    );
  };

  const CreateInput = ({ depth }: { depth: number }) => {
    const pl = 8 + depth * 16;
    const icon = creating?.type === "folder"
      ? <FolderOpen className="w-3.5 h-3.5 text-amber-400/70 flex-shrink-0" />
      : <FileText className="w-3.5 h-3.5 text-blue-400/60 flex-shrink-0" />;
    return (
      <div className="flex items-center gap-1.5 py-0.5 pr-2" style={{ paddingLeft: pl }}>
        {icon}
        <input
          ref={newNameRef}
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") confirmCreate();
            if (e.key === "Escape") { setCreating(null); setNewName(""); }
          }}
          onBlur={() => { if (!newName.trim()) { setCreating(null); setNewName(""); } else confirmCreate(); }}
          placeholder={creating?.type === "folder" ? t("wiki.folderName") : t("wiki.fileName")}
          className="flex-1 px-1.5 py-0.5 text-[11px] rounded bg-background border border-accent/40 text-foreground placeholder:text-muted-foreground/30 outline-none"
        />
      </div>
    );
  };

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-border/15 flex-shrink-0">
        <div>
          <h1 className="text-[16px] font-semibold text-foreground">{t("wiki.title")}</h1>
          <p className="text-[11px] text-muted-foreground/50 mt-0.5">
            {t("wiki.fileCount", { count: fileCount })}
          </p>
        </div>
        <div className="flex items-center gap-1.5">
          <button type="button"
            onClick={async () => {
              try {
                const result = await wikiApi.reindex() as unknown as { reindexed: number };
                await loadTree();
                setWikiStats(null);
                wikiApi.statsFull().then(setWikiStats).catch(() => {});
                setError(null);
                if (result.reindexed > 0) {
                  setError(`Reindexed ${result.reindexed} pages`);
                  setTimeout(() => setError(null), 3000);
                }
              } catch (e) { setError(String(e)); }
            }}
            title="Reindex: re-scan all wiki files and fix categories"
            className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors"
          >
            <RefreshCw className="w-4 h-4" />
          </button>
          <button type="button" onClick={() => startCreate("file", "")} title={t("wiki.newFile")}
            className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors">
            <Plus className="w-4 h-4" />
          </button>
          <button type="button" onClick={() => startCreate("folder", "")} title={t("wiki.newFolder")}
            className="p-2 rounded-lg text-muted-foreground/50 hover:text-accent hover:bg-[var(--bg-hover)] transition-colors">
            <FolderPlus className="w-4 h-4" />
          </button>
        </div>
      </div>

      {error && (
        <div className="mx-6 mt-3 text-[11px] text-destructive/80 bg-destructive/5 rounded-md px-3 py-2 flex items-center justify-between">
          <span>{error}</span>
          <button type="button" onClick={() => setError(null)} className="ml-2 text-destructive/50 hover:text-destructive"><X className="w-3 h-3" /></button>
        </div>
      )}

      {/* Content area */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        {/* Left: file tree + search */}
        <div className="w-[260px] flex-shrink-0 flex flex-col border-r border-border/10">
          <div className="px-3 py-2 border-b border-border/8">
            <div className="relative">
              <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-muted-foreground/30" />
              <input
                value={search} onChange={(e) => handleSearch(e.target.value)}
                placeholder={t("wiki.searchPlaceholder")}
                className="w-full h-7 pl-7 pr-2 text-[11px] bg-[var(--bg-hover)]/30 rounded-md border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
              />
              {searching && <Loader2 className="absolute right-2 top-1/2 -translate-y-1/2 w-3 h-3 animate-spin text-muted-foreground/30" />}
            </div>
          </div>

          <div className="flex-1 overflow-y-auto py-1.5 px-1.5">
            {searchResults !== null ? (
              searchResults.length === 0 ? (
                <div className="flex flex-col items-center justify-center h-20 gap-1">
                  <Search className="w-4 h-4 text-muted-foreground/15" />
                  <span className="text-[11px] text-muted-foreground/30">{t("common.noResults")}</span>
                </div>
              ) : (
                <div className="space-y-0.5">
                  {searchResults.map((r, i) => (
                    <div key={`${r.path}-${r.line}-${i}`}
                      className="px-2 py-1.5 rounded-md cursor-pointer hover:bg-[var(--bg-hover)] transition-colors"
                      onClick={() => { openFile(r.path, r.name); setSearch(""); setSearchResults(null); }}>
                      <div className="flex items-center gap-1.5">
                        <FileIcon name={r.name} className="w-3 h-3 flex-shrink-0" />
                        <span className="text-[11px] text-foreground/80 truncate">{r.name}</span>
                        {r.line > 0 && (
                          <span className="text-[9px] text-muted-foreground/40 ml-auto flex-shrink-0">
                            {t("wiki.line", { line: r.line })}
                          </span>
                        )}
                      </div>
                      {r.line > 0 && <p className="text-[10px] text-muted-foreground/40 mt-0.5 truncate pl-[18px]">{r.content}</p>}
                    </div>
                  ))}
                </div>
              )
            ) : loading ? (
              <div className="flex items-center justify-center h-20">
                <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" />
              </div>
            ) : tree.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-32 gap-2">
                <BookOpen className="w-6 h-6 text-muted-foreground/15" />
                <span className="text-[11px] text-muted-foreground/30">{t("wiki.noFiles")}</span>
                <span className="text-[10px] text-muted-foreground/20">{t("wiki.noFilesHint")}</span>
              </div>
            ) : (
              <div>
                {creating && creating.parentPath === "" && <CreateInput depth={0} />}
                {tree.map((entry) => <TreeNode key={entry.path} entry={entry} depth={0} />)}
              </div>
            )}
          </div>
        </div>

        {/* Right: editor / preview */}
        <div className="flex-1 min-w-0 flex flex-col">
          {activePath ? (
            <>
              <div className="flex items-center justify-between px-4 py-2 border-b border-border/8 flex-shrink-0">
                <div className="flex items-center gap-2 min-w-0">
                  <FileIcon name={activeFileName} className="w-3.5 h-3.5 flex-shrink-0" />
                  <span className="text-[12px] font-medium text-foreground/80 truncate">{activePath}</span>
                  {dirty && <span className="w-1.5 h-1.5 rounded-full bg-accent/60 flex-shrink-0" />}
                  {codeLang && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/12 text-emerald-400 flex-shrink-0">
                      {codeLang}
                    </span>
                  )}
                </div>
              </div>

              {isMd ? (
                <div className="flex-1 min-h-0 overflow-hidden flex flex-col">
                  <MarkdownToolbar textareaRef={editorRef} onChange={handleContentChange} content={content} />
                  <div className="flex-1 flex min-h-0 overflow-hidden">
                    <div className="flex-1 min-w-0 flex flex-col overflow-hidden border-r border-border/10">
                      <WikiEditor
                        ref={editorRef}
                        value={content}
                        onChange={handleContentChange}
                        language={null}
                      />
                    </div>
                    <div className="flex-1 min-w-0 overflow-y-auto px-6 py-4">
                      <div className="prose prose-invert prose-sm max-w-none">
                        <Markdown content={content} />
                      </div>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="flex-1 min-h-0 overflow-hidden flex flex-col">
                  <WikiEditor
                    ref={editorRef}
                    value={content}
                    onChange={handleContentChange}
                    language={codeLang}
                  />
                </div>
              )}
            </>
          ) : (
            <WikiDashboard
              stats={wikiStats}
              onOpenPage={(path) => openFile(path)}
            />
          )}
        </div>
      </div>

      {/* Delete confirm */}
      {deleteTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setDeleteTarget(null)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-5 shadow-xl max-w-xs w-full" onClick={(e) => e.stopPropagation()}>
            <p className="text-[13px] text-foreground mb-1">{t("wiki.deleteConfirm", { name: deleteTarget.name })}</p>
            <p className="text-[11px] text-muted-foreground/50 mb-4">{t("wiki.deleteConfirmMsg")}</p>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setDeleteTarget(null)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">
                {t("common.cancel")}
              </button>
              <button type="button" onClick={handleDelete}
                className="text-[12px] px-3 py-1.5 rounded-lg bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">
                {t("common.delete")}
              </button>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}
