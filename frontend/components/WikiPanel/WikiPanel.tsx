import { useCallback, useEffect, useRef, useState } from "react";
import {
  BookOpen, FolderPlus,
  Loader2, Plus, RefreshCw, Search, X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import { wikiApi, type WikiEntry, type WikiSearchResult, type WikiStats } from "@/lib/wiki";
import { extToLang, isMarkdown } from "./utils";
import { FileIcon } from "./FileIcon";
import { WikiEditor } from "./WikiEditor";
import { WikiDashboard } from "./WikiDashboard";
import { TreeNode } from "./TreeNode";
import { CreateInput } from "./CreateInput";

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

  const cancelCreate = useCallback(() => {
    setCreating(null);
    setNewName("");
  }, []);

  const renderCreateInput = useCallback((depth: number) => (
    <CreateInput
      depth={depth}
      creatingType={creating?.type ?? null}
      newName={newName}
      setNewName={setNewName}
      newNameRef={newNameRef}
      confirmCreate={confirmCreate}
      cancelCreate={cancelCreate}
    />
  ), [creating?.type, newName, confirmCreate, cancelCreate]);

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
                {creating && creating.parentPath === "" && renderCreateInput(0)}
                {tree.map((entry) => (
                  <TreeNode
                    key={entry.path}
                    entry={entry}
                    depth={0}
                    expandedDirs={expandedDirs}
                    activePath={activePath}
                    creating={creating}
                    toggleDir={toggleDir}
                    openFile={openFile}
                    startCreate={startCreate}
                    setDeleteTarget={setDeleteTarget}
                    renderCreateInput={renderCreateInput}
                  />
                ))}
              </div>
            )}
          </div>
        </div>

        {/* Right: editor / preview */}
        <div className="flex-1 min-w-0 min-h-0 flex flex-col">
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
                <div className="flex-1 min-h-0">
                  <MarkdownEditor
                    editorKey={activePath || ""}
                    value={content}
                    onChange={handleContentChange}
                  />
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
