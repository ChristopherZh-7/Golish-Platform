import { useCallback, useEffect, useRef, useState } from "react";
import {
  Bold, BookOpen, ChevronDown, ChevronRight, Code, Eye, FileCode2, FileText, FolderOpen, FolderPlus,
  Heading1, Heading2, Heading3, Italic, Link2, List, ListOrdered, Loader2, Pencil, Plus, Quote,
  Search, Shield, Strikethrough, Table, Trash2, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Markdown } from "@/components/Markdown/Markdown";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

interface WikiEntry {
  path: string;
  name: string;
  is_dir: boolean;
  children?: WikiEntry[];
  size?: number;
  modified?: number;
}

interface WikiSearchResult {
  path: string;
  name: string;
  line: number;
  content: string;
}

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
  return name.endsWith(".md") || name.endsWith(".txt");
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

export function WikiPanel() {
  const { t } = useTranslation();
  const [tree, setTree] = useState<WikiEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [activeFileName, setActiveFileName] = useState<string>("");
  const [content, setContent] = useState("");
  const [originalContent, setOriginalContent] = useState("");
  const [dirty, setDirty] = useState(false);
  const [mode, setMode] = useState<"edit" | "preview">("edit");
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const [search, setSearch] = useState("");
  const [searchResults, setSearchResults] = useState<WikiSearchResult[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<WikiEntry | null>(null);

  // Inline creation
  const [creating, setCreating] = useState<{ type: "file" | "folder"; parentPath: string } | null>(null);
  const [newName, setNewName] = useState("");
  const newNameRef = useRef<HTMLInputElement>(null);

  // CVE creation dialog
  const [showCveDialog, setShowCveDialog] = useState(false);
  const [cveId, setCveId] = useState("");
  const [cveTitle, setCveTitle] = useState("");
  const [cvePocLang, setCvePocLang] = useState("py");
  const [cveCreating, setCveCreating] = useState(false);

  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadTree = useCallback(async () => {
    setLoading(true);
    try {
      const data: WikiEntry[] = await invoke("wiki_list");
      setTree(data);
    } catch (e) {
      setError(t("wiki.loadFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => { loadTree(); }, [loadTree]);

  const openFile = useCallback(async (path: string, fileName?: string) => {
    if (dirty && activePath) {
      try { await invoke("wiki_write", { path: activePath, content }); } catch { /* ignore */ }
    }
    try {
      const data: string = await invoke("wiki_read", { path });
      setActivePath(path);
      setActiveFileName(fileName || path.split("/").pop() || path);
      setContent(data);
      setOriginalContent(data);
      setDirty(false);
      setMode(isMarkdown(path) ? "edit" : "edit");
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
          await invoke("wiki_write", { path: activePath, content: value });
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
      const results: WikiSearchResult[] = await invoke("wiki_search", { query: q.trim() });
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
        await invoke("wiki_create_dir", { path });
      } else {
        const filePath = name.includes(".") ? path : `${path}.md`;
        await invoke("wiki_write", { path: filePath, content: name.endsWith(".md") || !name.includes(".") ? `# ${name.replace(/\.\w+$/, "")}\n\n` : "" });
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

  const handleCreateCve = useCallback(async () => {
    if (!cveId.trim() || !cveTitle.trim()) return;
    setCveCreating(true);
    try {
      const readmePath: string = await invoke("wiki_create_cve", {
        cveId: cveId.trim(),
        title: cveTitle.trim(),
        pocLang: cvePocLang || null,
      });
      await loadTree();
      setExpandedDirs((prev) => new Set([...prev, cveId.trim()]));
      openFile(readmePath, "README.md");
      setShowCveDialog(false);
      setCveId("");
      setCveTitle("");
    } catch (e) { setError(String(e)); }
    finally { setCveCreating(false); }
  }, [cveId, cveTitle, cvePocLang, loadTree, openFile]);

  const handleDelete = useCallback(async () => {
    if (!deleteTarget) return;
    try {
      await invoke("wiki_delete", { path: deleteTarget.path });
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
          <button type="button" onClick={() => setShowCveDialog(true)} title={t("wiki.newCve")}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors">
            <Shield className="w-3.5 h-3.5" /> {t("wiki.newCve")}
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
                <div className="flex items-center gap-1">
                  {isMd && (
                    <div className="flex items-center rounded-md border border-border/15 overflow-hidden">
                      <button type="button" onClick={() => setMode("edit")}
                        className={cn("flex items-center gap-1 px-2.5 py-1 text-[11px] transition-colors",
                          mode === "edit" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                        <Pencil className="w-3 h-3" /> {t("wiki.edit")}
                      </button>
                      <button type="button" onClick={() => setMode("preview")}
                        className={cn("flex items-center gap-1 px-2.5 py-1 text-[11px] transition-colors",
                          mode === "preview" ? "bg-accent/15 text-accent" : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]")}>
                        <Eye className="w-3 h-3" /> {t("wiki.preview")}
                      </button>
                    </div>
                  )}
                </div>
              </div>

              <div className="flex-1 min-h-0 overflow-hidden flex flex-col">
                {isMd && mode === "preview" ? (
                  <div className="flex-1 overflow-y-auto px-6 py-4">
                    <div className="prose prose-invert prose-sm max-w-none">
                      <Markdown content={content} />
                    </div>
                  </div>
                ) : (
                  <>
                    {isMd && mode === "edit" && (
                      <MarkdownToolbar textareaRef={editorRef} onChange={handleContentChange} content={content} />
                    )}
                    <WikiEditor
                      ref={editorRef}
                      value={content}
                      onChange={handleContentChange}
                      language={codeLang}
                    />
                  </>
                )}
              </div>
            </>
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center gap-3">
              <BookOpen className="w-10 h-10 text-muted-foreground/10" />
              <p className="text-[13px] text-muted-foreground/30">{t("wiki.selectFile")}</p>
            </div>
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

      {/* CVE creation dialog */}
      {showCveDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setShowCveDialog(false)}>
          <div className="bg-[var(--bg-hover)] rounded-xl border border-border/20 p-6 shadow-xl max-w-md w-full" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center gap-2 mb-4">
              <Shield className="w-5 h-5 text-accent" />
              <h2 className="text-[15px] font-semibold text-foreground">{t("wiki.newCve")}</h2>
            </div>
            <div className="space-y-3">
              <div>
                <label className="text-[11px] text-muted-foreground/60 mb-1 block">{t("wiki.cveId")}</label>
                <input value={cveId} onChange={(e) => setCveId(e.target.value)}
                  placeholder="CVE-2024-XXXX"
                  className="w-full h-8 px-3 text-[12px] font-mono bg-background rounded-lg border border-border/20 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
                />
              </div>
              <div>
                <label className="text-[11px] text-muted-foreground/60 mb-1 block">{t("wiki.cveTitle")}</label>
                <input value={cveTitle} onChange={(e) => setCveTitle(e.target.value)}
                  placeholder={t("wiki.cveTitlePlaceholder")}
                  className="w-full h-8 px-3 text-[12px] bg-background rounded-lg border border-border/20 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
                />
              </div>
              <div>
                <label className="text-[11px] text-muted-foreground/60 mb-1 block">{t("wiki.pocLang")}</label>
                <Select value={cvePocLang} onValueChange={setCvePocLang}>
                  <SelectTrigger className="h-8 text-[12px] border-border/20 bg-background">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="py" className="text-[12px]">Python (.py)</SelectItem>
                    <SelectItem value="go" className="text-[12px]">Go (.go)</SelectItem>
                    <SelectItem value="sh" className="text-[12px]">Bash (.sh)</SelectItem>
                    <SelectItem value="js" className="text-[12px]">JavaScript (.js)</SelectItem>
                    <SelectItem value="rs" className="text-[12px]">Rust (.rs)</SelectItem>
                    <SelectItem value="rb" className="text-[12px]">Ruby (.rb)</SelectItem>
                    <SelectItem value="java" className="text-[12px]">Java (.java)</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="flex justify-end gap-2 mt-5">
              <button type="button" onClick={() => setShowCveDialog(false)}
                className="text-[12px] px-3 py-1.5 rounded-lg text-muted-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors">
                {t("common.cancel")}
              </button>
              <button type="button" onClick={handleCreateCve} disabled={cveCreating || !cveId.trim() || !cveTitle.trim()}
                className={cn("flex items-center gap-1.5 text-[12px] px-4 py-1.5 rounded-lg font-medium transition-colors",
                  cveId.trim() && cveTitle.trim()
                    ? "bg-accent text-accent-foreground hover:bg-accent/90"
                    : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
                {cveCreating && <Loader2 className="w-3 h-3 animate-spin" />}
                {t("wiki.createCve")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
