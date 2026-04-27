import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  BookOpen, ChevronDown, ChevronRight, FileText,
  Link2, Loader2, Plus, Search, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { VulnLink } from "./types";
import { Markdown } from "@/components/Markdown";
import { wikiApi, type WikiPageInfo, type WikiBacklinkInfo, type WikiTreeNode } from "@/lib/wiki";

export function WikiTab({ link, cveId, onUpdateLink }: { link: VulnLink; cveId: string; onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void }) {
  const [fullTree, setFullTree] = useState<WikiTreeNode[]>([]);
  const [loadingTree, setLoadingTree] = useState(true);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [articleContents, setArticleContents] = useState<Record<string, string>>({});
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [editContent, setEditContent] = useState("");
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());

  const [linkedPageInfos, setLinkedPageInfos] = useState<WikiPageInfo[]>([]);
  const [suggestedPages, setSuggestedPages] = useState<WikiPageInfo[]>([]);
  const [backlinks, setBacklinks] = useState<WikiBacklinkInfo[]>([]);
  const [adding, setAdding] = useState(false);
  const [newPath, setNewPath] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<{ path: string; title: string; snippet: string }[]>([]);
  const [searching, setSearching] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createPath, setCreatePath] = useState("");

  const linkedSet = useMemo(() => new Set(link.wikiPaths), [link.wikiPaths]);

  const reloadTree = useCallback(() => {
    setLoadingTree(true);
    wikiApi.list().then(d => d as unknown as WikiTreeNode[])
      .then((tree) => {
        setFullTree(Array.isArray(tree) ? tree : []);
        const dirs = new Set<string>();
        for (const p of link.wikiPaths) {
          const parts = p.split("/");
          for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join("/"));
        }
        setExpandedDirs(dirs);
        if (link.wikiPaths.length > 0 && !selectedPath) setSelectedPath(link.wikiPaths[0]);
      })
      .catch(console.error)
      .finally(() => setLoadingTree(false));
  }, [link.wikiPaths, selectedPath]);

  useEffect(() => { reloadTree(); }, []);

  useEffect(() => {
    if (!selectedPath || articleContents[selectedPath] !== undefined) return;
    wikiApi.read(selectedPath)
      .then((content) => setArticleContents((prev) => ({ ...prev, [selectedPath]: content })))
      .catch(() => setArticleContents((prev) => ({ ...prev, [selectedPath]: "" })));
  }, [selectedPath]);

  // Ensure DB index is up-to-date before fetching metadata
  const reindexDone = useRef(false);
  const [indexReady, setIndexReady] = useState(false);
  useEffect(() => {
    if (reindexDone.current) return;
    reindexDone.current = true;
    wikiApi.reindex()
      .catch(() => {})
      .finally(() => setIndexReady(true));
  }, []);

  // Fetch metadata for linked pages (category, status, etc.)
  useEffect(() => {
    if (!indexReady) return;
    if (link.wikiPaths.length === 0) { setLinkedPageInfos([]); return; }
    invoke<WikiPageInfo[]>("wiki_pages_for_paths", { paths: link.wikiPaths })
      .then(setLinkedPageInfos)
      .catch(() => setLinkedPageInfos([]));
  }, [link.wikiPaths, indexReady]);

  // Fetch suggested pages for this CVE
  useEffect(() => {
    invoke<WikiPageInfo[]>("wiki_suggest_for_cve", { cveId, limit: 8 })
      .then(setSuggestedPages)
      .catch(() => setSuggestedPages([]));
  }, [cveId, link.wikiPaths]);

  // Fetch backlinks when selected page changes
  useEffect(() => {
    if (!selectedPath) { setBacklinks([]); return; }
    invoke<WikiBacklinkInfo[]>("wiki_backlinks", { path: selectedPath })
      .then(setBacklinks)
      .catch(() => setBacklinks([]));
  }, [selectedPath]);

  // Group linked pages by category
  const linkedByCategory = useMemo(() => {
    const groups: Record<string, WikiPageInfo[]> = {};
    for (const info of linkedPageInfos) {
      const cat = info.category || "uncategorized";
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(info);
    }
    // Include paths that aren't in DB yet as uncategorized
    const infoPathSet = new Set(linkedPageInfos.map((p) => p.path));
    for (const wp of link.wikiPaths) {
      if (!infoPathSet.has(wp)) {
        if (!groups["uncategorized"]) groups["uncategorized"] = [];
        groups["uncategorized"].push({
          path: wp,
          title: wp.split("/").pop()?.replace(/\.md$/, "") || wp,
          category: "uncategorized",
          tags: [],
          status: "draft",
          word_count: 0,
          updated_at: "",
        });
      }
    }
    return groups;
  }, [linkedPageInfos, link.wikiPaths]);

  const categoryOrder = ["products", "techniques", "pocs", "experience", "analysis", "uncategorized"];
  const categoryIcons: Record<string, string> = {
    products: "📦", techniques: "⚔️", pocs: "🔧", experience: "📝", analysis: "🔬", uncategorized: "📄",
  };
  const statusColors: Record<string, string> = {
    draft: "text-yellow-400 bg-yellow-500/10",
    partial: "text-orange-400 bg-orange-500/10",
    complete: "text-green-400 bg-green-500/10",
    "needs-poc": "text-blue-400 bg-blue-500/10",
    verified: "text-emerald-400 bg-emerald-500/10",
  };

  const toggleDir = useCallback((dir: string) => {
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(dir)) next.delete(dir);
      else next.add(dir);
      return next;
    });
  }, []);

  const handleLinkWiki = useCallback((path: string) => {
    onUpdateLink((l) => ({
      ...l,
      wikiPaths: l.wikiPaths.includes(path) ? l.wikiPaths : [...l.wikiPaths, path],
    }));
    invoke("vuln_link_add_wiki", { cveId, wikiPath: path }).catch(console.error);
  }, [onUpdateLink, cveId]);

  const handleUnlinkWiki = useCallback((path: string) => {
    onUpdateLink((l) => ({ ...l, wikiPaths: l.wikiPaths.filter((p) => p !== path) }));
    invoke("vuln_link_remove_wiki", { cveId, wikiPath: path }).catch(console.error);
    if (selectedPath === path) setSelectedPath(null);
  }, [onUpdateLink, cveId, selectedPath]);

  const handleStartEdit = useCallback((path: string) => {
    setEditingPath(path);
    setEditContent(articleContents[path] || "");
  }, [articleContents]);

  const handleSaveEdit = useCallback(async () => {
    if (!editingPath) return;
    try {
      await wikiApi.write(editingPath, editContent);
      setArticleContents((prev) => ({ ...prev, [editingPath]: editContent }));
      setEditingPath(null);
    } catch (err) {
      console.error("Failed to save wiki article:", err);
    }
  }, [editingPath, editContent]);

  const handleDeletePage = useCallback(async (path: string) => {
    if (!confirm(`Delete wiki page "${path}"? This cannot be undone.`)) return;
    try {
      await wikiApi.delete(path);
      setArticleContents((prev) => {
        const next = { ...prev };
        delete next[path];
        return next;
      });
      if (selectedPath === path) setSelectedPath(null);
      if (linkedSet.has(path)) handleUnlinkWiki(path);
      reloadTree();
    } catch (err) {
      console.error("Failed to delete wiki page:", err);
    }
  }, [selectedPath, linkedSet, handleUnlinkWiki, reloadTree]);

  const handleCreatePage = useCallback(async () => {
    const p = createPath.trim();
    if (!p) return;
    const path = p.endsWith(".md") ? p : `${p}.md`;
    const template = `---\ntitle: ${path.split("/").pop()?.replace(/\.md$/, "") || "New Page"}\ncategory: ${path.split("/")[0] || "uncategorized"}\ntags: []\ncves: [${cveId}]\nstatus: draft\n---\n\n# ${path.split("/").pop()?.replace(/\.md$/, "") || "New Page"}\n\nContent here.\n`;
    try {
      await wikiApi.write(path, template);
      handleLinkWiki(path);
      setCreating(false);
      setCreatePath("");
      setArticleContents((prev) => ({ ...prev, [path]: template }));
      reloadTree();
      setSelectedPath(path);
    } catch (err) {
      console.error("Failed to create wiki page:", err);
    }
  }, [createPath, cveId, handleLinkWiki, reloadTree]);

  // Full-text search via DB
  const handleSearch = useCallback(async (query: string) => {
    if (!query.trim()) { setSearchResults([]); return; }
    setSearching(true);
    try {
      const results = await invoke<{ path: string; title: string; category: string; tags: string[]; status: string | null }[]>(
        "wiki_search_db", { query: query.trim(), limit: 20 }
      );
      setSearchResults(results.map((r) => ({
        path: r.path,
        title: r.title || r.path,
        snippet: `[${r.category}] ${r.tags.join(", ")}${r.status ? ` • ${r.status}` : ""}`,
      })));
    } catch {
      setSearchResults([]);
    }
    setSearching(false);
  }, []);

  useEffect(() => {
    const t = setTimeout(() => handleSearch(searchQuery), 300);
    return () => clearTimeout(t);
  }, [searchQuery, handleSearch]);

  // Navigate to a wiki page (used by cross-reference links)
  const navigateToWikiPage = useCallback((path: string) => {
    const expandPath = (p: string) => {
      const parts = p.split("/");
      const dirs = new Set<string>();
      for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join("/"));
      return dirs;
    };
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      for (const d of expandPath(path)) next.add(d);
      return next;
    });
    setArticleContents((prev) => {
      if (prev[path] !== undefined) return prev;
      return { ...prev };
    });
    setSelectedPath(path);
    setSearchQuery("");
    setSearchResults([]);
  }, []);

  const stripFrontmatter = (content: string) => {
    const match = content.match(/^---\n[\s\S]*?\n---\n?/);
    return match ? content.slice(match[0].length) : content;
  };

  const extractFrontmatterField = (content: string, field: string): string | null => {
    const fm = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) return null;
    const m = fm[1].match(new RegExp(`^${field}:\\s*(.+)$`, "m"));
    return m?.[1]?.trim() || null;
  };

  const extractTitle = (content: string) => {
    const t = extractFrontmatterField(content, "title");
    if (t) return t.replace(/^["']|["']$/g, "");
    const h1 = content.match(/^#\s+(.+)$/m);
    return h1?.[1] || null;
  };

  const extractStatus = (content: string) => extractFrontmatterField(content, "status");

  const extractTags = (content: string): string[] => {
    const fm = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) return [];
    const m = fm[1].match(/tags:\s*\[([^\]]*)\]/);
    if (m) return m[1].split(",").map((t) => t.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
    return [];
  };

  const extractCves = (content: string): string[] => {
    const fm = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fm) return [];
    const m = fm[1].match(/cves:\s*\[([^\]]*)\]/);
    if (m) return m[1].split(",").map((c) => c.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
    return [];
  };

  // Intercept wiki-link clicks in rendered markdown
  const handleContentClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    const anchor = target.closest("a");
    if (!anchor) return;
    const href = anchor.getAttribute("href") || "";
    if (href.match(/^(https?:|mailto:|#)/)) return;
    if (href.endsWith(".md") || !href.includes("://")) {
      e.preventDefault();
      const resolved = selectedPath
        ? new URL(href, `file:///${selectedPath}`).pathname.replace(/^\//, "")
        : href;
      navigateToWikiPage(resolved);
    }
  }, [selectedPath, navigateToWikiPage]);

  const proseClasses = "text-[11px] leading-relaxed text-foreground/80 prose prose-invert prose-sm max-w-none prose-headings:text-foreground/90 prose-headings:text-[12px] prose-headings:font-semibold prose-p:text-[11px] prose-p:leading-relaxed prose-code:text-[10px] prose-code:bg-muted/20 prose-code:px-1 prose-code:rounded prose-pre:bg-muted/10 prose-pre:border prose-pre:border-border/10 prose-pre:text-[10px] prose-li:text-[11px] prose-a:text-accent";

  // Filter tree nodes by search
  const filterTree = useCallback((nodes: WikiTreeNode[], q: string): WikiTreeNode[] => {
    if (!q) return nodes;
    const lower = q.toLowerCase();
    return nodes.reduce<WikiTreeNode[]>((acc, node) => {
      if (node.is_dir) {
        const filtered = filterTree(node.children || [], q);
        if (filtered.length > 0) acc.push({ ...node, children: filtered });
      } else if (node.name.toLowerCase().includes(lower) || node.path.toLowerCase().includes(lower)) {
        acc.push(node);
      }
      return acc;
    }, []);
  }, []);

  const [browseAll, setBrowseAll] = useState(false);

  const displayTree = searchQuery && !searchResults.length ? filterTree(fullTree, searchQuery) : browseAll ? fullTree : [];

  const renderTreeNode = (node: WikiTreeNode, depth = 0) => {
    if (node.is_dir) {
      const isExpanded = expandedDirs.has(node.path) || !!searchQuery;
      const icon = categoryIcons[node.name] || "";
      return (
        <div key={node.path}>
          <button
            onClick={() => toggleDir(node.path)}
            className="flex items-center gap-1 w-full px-1.5 py-1 rounded text-left hover:bg-muted/10 transition-colors text-muted-foreground/50"
            style={{ paddingLeft: `${depth * 12 + 6}px` }}
          >
            {isExpanded ? (
              <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/40 flex-shrink-0" />
            ) : (
              <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/40 flex-shrink-0" />
            )}
            {icon ? (
              <span className="text-[10px] flex-shrink-0">{icon}</span>
            ) : (
              <BookOpen className="w-3 h-3 text-muted-foreground/30 flex-shrink-0" />
            )}
            <span className="text-[10px] truncate">{node.name}</span>
          </button>
          {isExpanded && node.children?.map((child) => renderTreeNode(child, depth + 1))}
        </div>
      );
    }

    const isLinked = linkedSet.has(node.path);
    const isSelected = selectedPath === node.path;
    return (
      <div key={node.path} className="group/file flex items-center">
        <button
          onClick={() => setSelectedPath(node.path)}
          className={cn(
            "flex items-center gap-1.5 flex-1 px-1.5 py-1 rounded text-left transition-colors",
            isSelected
              ? "bg-accent/15 text-accent"
              : "text-muted-foreground/40 hover:bg-muted/10 hover:text-muted-foreground/60"
          )}
          style={{ paddingLeft: `${depth * 12 + 6}px` }}
        >
          <FileText className={cn("w-3 h-3 flex-shrink-0", isSelected ? "text-accent" : "text-muted-foreground/25")} />
          <span className="text-[10px] truncate flex-1">{node.name.replace(/\.md$/, "")}</span>
        </button>
        {!isLinked && (
          <button
            onClick={() => handleLinkWiki(node.path)}
            className="p-0.5 text-accent/0 group-hover/file:text-accent/50 hover:!text-accent transition-colors flex-shrink-0"
            title="Link to this CVE"
          >
            <Link2 className="w-2.5 h-2.5" />
          </button>
        )}
      </div>
    );
  };

  const selectedContent = selectedPath ? articleContents[selectedPath] || "" : "";
  const selectedTitle = selectedContent ? extractTitle(selectedContent) || selectedPath?.split("/").pop()?.replace(/\.md$/, "") : null;
  const selectedStatus = selectedContent ? extractStatus(selectedContent) : null;
  const selectedBody = selectedContent ? stripFrontmatter(selectedContent) : "";
  const selectedTags = selectedContent ? extractTags(selectedContent) : [];
  const selectedCves = selectedContent ? extractCves(selectedContent) : [];
  const isEditing = editingPath === selectedPath;

  return (
    <div className="flex h-full min-h-0">
      {/* Left: categorized page list */}
      <div className="w-[220px] flex-shrink-0 border-r border-border/10 flex flex-col">
        {/* Header with actions */}
        <div className="flex items-center justify-between px-2 py-1.5 border-b border-border/5">
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">
            Wiki ({link.wikiPaths.length})
          </span>
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => { setCreating(!creating); setAdding(false); }}
              className="p-0.5 text-muted-foreground/30 hover:text-emerald-400 transition-colors"
              title="Create new wiki page"
            >
              <Plus className="w-3 h-3" />
            </button>
            <button
              onClick={() => { setAdding(!adding); setCreating(false); }}
              className="p-0.5 text-muted-foreground/30 hover:text-accent transition-colors"
              title="Link existing wiki article"
            >
              <Link2 className="w-3 h-3" />
            </button>
          </div>
        </div>

        {/* Search */}
        <div className="px-2 py-1.5 border-b border-border/5">
          <div className="relative">
            <Search className="absolute left-1.5 top-1/2 -translate-y-1/2 w-2.5 h-2.5 text-muted-foreground/25" />
            <input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search wiki..."
              className="w-full h-5 pl-5 pr-1.5 text-[9px] bg-[var(--bg-hover)]/30 rounded border border-border/10 text-foreground placeholder:text-muted-foreground/25 outline-none focus:border-accent/30"
            />
            {searching && <Loader2 className="absolute right-1.5 top-1/2 -translate-y-1/2 w-2.5 h-2.5 animate-spin text-muted-foreground/30" />}
          </div>
        </div>

        {/* Create form */}
        {creating && (
          <div className="px-2 py-1.5 border-b border-border/5 space-y-1">
            <input
              value={createPath}
              onChange={(e) => setCreatePath(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleCreatePage(); }}
              placeholder="products/myapp/CVE-XXXX.md"
              className="w-full h-5 px-1.5 text-[9px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
              autoFocus
            />
            <div className="flex gap-1">
              <button onClick={handleCreatePage} disabled={!createPath.trim()}
                className="text-[8px] text-emerald-400 disabled:opacity-30">Create</button>
              <button onClick={() => { setCreating(false); setCreatePath(""); }}
                className="text-[8px] text-muted-foreground/30">Cancel</button>
            </div>
          </div>
        )}

        {/* Manual link input */}
        {adding && (
          <div className="px-2 py-1.5 border-b border-border/5 space-y-1">
            <input
              value={newPath}
              onChange={(e) => setNewPath(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && newPath.trim()) {
                  handleLinkWiki(newPath.trim());
                  setAdding(false);
                  setNewPath("");
                }
              }}
              placeholder="Path to link..."
              className="w-full h-5 px-1.5 text-[9px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
              autoFocus
            />
            <div className="flex gap-1">
              <button onClick={() => { handleLinkWiki(newPath.trim()); setAdding(false); setNewPath(""); }}
                disabled={!newPath.trim()} className="text-[8px] text-accent disabled:opacity-30">Link</button>
              <button onClick={() => { setAdding(false); setNewPath(""); }}
                className="text-[8px] text-muted-foreground/30">Cancel</button>
            </div>
          </div>
        )}

        {/* DB search results */}
        {searchQuery && searchResults.length > 0 && (
          <div className="border-b border-border/5 max-h-32 overflow-y-auto">
            <div className="px-2 py-0.5">
              <span className="text-[7px] text-muted-foreground/25 uppercase">Search Results</span>
            </div>
            {searchResults.map((r) => (
              <button
                key={r.path}
                onClick={() => navigateToWikiPage(r.path)}
                className="flex flex-col w-full px-2 py-1 hover:bg-muted/10 transition-colors text-left"
              >
                <span className="text-[9px] text-foreground/70 truncate">{r.title}</span>
                <span className="text-[7px] text-muted-foreground/30 truncate">{r.snippet}</span>
              </button>
            ))}
          </div>
        )}

        {/* Main page list */}
        <div className="flex-1 overflow-y-auto py-1">
          {loadingTree ? (
            <div className="flex items-center justify-center py-6">
              <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/20" />
            </div>
          ) : link.wikiPaths.length === 0 && !browseAll && !searchQuery ? (
            <div className="text-[9px] text-muted-foreground/20 text-center py-6 px-3">
              <BookOpen className="w-6 h-6 mx-auto mb-2 text-muted-foreground/10" />
              No linked wiki pages
              <div className="mt-1 text-[8px]">Run AI Research to auto-generate</div>
            </div>
          ) : (
            <>
              {/* Linked pages grouped by category */}
              {categoryOrder.filter((cat) => linkedByCategory[cat]?.length).map((cat) => (
                <div key={cat}>
                  <div className="flex items-center gap-1 px-2 py-1 mt-1">
                    <span className="text-[10px]">{categoryIcons[cat] || "📄"}</span>
                    <span className="text-[8px] text-muted-foreground/40 uppercase tracking-wider flex-1">{cat}</span>
                    <span className="text-[7px] text-muted-foreground/20">{linkedByCategory[cat].length}</span>
                  </div>
                  {linkedByCategory[cat].map((info) => {
                    const isSelected = selectedPath === info.path;
                    return (
                      <div key={info.path} className="group/file flex items-center">
                        <button
                          onClick={() => setSelectedPath(info.path)}
                          className={cn(
                            "flex items-center gap-1.5 flex-1 px-2 py-1 rounded text-left transition-colors min-w-0",
                            isSelected ? "bg-accent/15 text-accent" : "text-foreground/70 hover:bg-muted/10"
                          )}
                        >
                          <FileText className={cn("w-3 h-3 flex-shrink-0", isSelected ? "text-accent" : "text-blue-400/60")} />
                          <span className="text-[10px] truncate flex-1">{info.title || info.path.split("/").pop()?.replace(/\.md$/, "")}</span>
                          {info.status && (
                            <span className={cn("text-[6px] px-1 py-0.5 rounded flex-shrink-0", statusColors[info.status] || "text-muted-foreground/30 bg-muted/10")}>
                              {info.status}
                            </span>
                          )}
                        </button>
                        <button
                          onClick={() => handleUnlinkWiki(info.path)}
                          className="p-0.5 text-destructive/0 group-hover/file:text-destructive/40 hover:!text-destructive transition-colors flex-shrink-0"
                          title="Unlink"
                        >
                          <X className="w-2.5 h-2.5" />
                        </button>
                      </div>
                    );
                  })}
                </div>
              ))}

              {/* Suggested pages */}
              {suggestedPages.length > 0 && (
                <>
                  <div className="px-2 py-1 mt-2 border-t border-border/5">
                    <span className="text-[7px] text-muted-foreground/25 uppercase tracking-wider">Suggested</span>
                  </div>
                  {suggestedPages.map((info) => (
                    <div key={info.path} className="group/sugg flex items-center">
                      <button
                        onClick={() => setSelectedPath(info.path)}
                        className="flex items-center gap-1.5 flex-1 px-2 py-1 rounded text-left text-muted-foreground/35 hover:text-foreground/60 hover:bg-muted/10 transition-colors min-w-0"
                      >
                        <FileText className="w-3 h-3 flex-shrink-0 text-muted-foreground/20" />
                        <span className="text-[9px] truncate flex-1">{info.title}</span>
                        <span className="text-[6px] text-muted-foreground/15">{info.category}</span>
                      </button>
                      <button
                        onClick={() => handleLinkWiki(info.path)}
                        className="p-0.5 text-accent/0 group-hover/sugg:text-accent/50 hover:!text-accent transition-colors flex-shrink-0"
                        title="Link to this CVE"
                      >
                        <Link2 className="w-2.5 h-2.5" />
                      </button>
                    </div>
                  ))}
                </>
              )}

              {/* Browse all tree (fallback) */}
              {browseAll && displayTree.length > 0 && (
                <>
                  <div className="px-2 py-0.5 mt-2 border-t border-border/5">
                    <span className="text-[7px] text-muted-foreground/25 uppercase tracking-wider">All Pages</span>
                  </div>
                  {displayTree.map((node) => renderTreeNode(node))}
                </>
              )}
            </>
          )}
        </div>

        {/* Bottom toggle */}
        <div className="px-2 py-1.5 border-t border-border/5 flex items-center gap-1">
          <button
            onClick={() => setBrowseAll(!browseAll)}
            className="text-[8px] text-muted-foreground/30 hover:text-muted-foreground/60 transition-colors"
          >
            {browseAll ? "Hide tree" : "Browse all..."}
          </button>
        </div>
      </div>

      {/* Right: article content */}
      <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
        {selectedPath ? (
          <>
            {/* Article header */}
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border/5">
              <FileText className="w-3.5 h-3.5 text-blue-400/60 flex-shrink-0" />
              <span className="text-[11px] text-foreground/80 font-medium truncate flex-1">
                {selectedTitle || selectedPath}
              </span>
              {selectedStatus && (
                <span className={cn("text-[8px] px-1.5 py-0.5 rounded", statusColors[selectedStatus] || "text-muted-foreground/40 bg-muted/10")}>
                  {selectedStatus}
                </span>
              )}
              {linkedSet.has(selectedPath) ? (
                <button onClick={() => handleUnlinkWiki(selectedPath)}
                  className="text-[8px] px-1.5 py-0.5 rounded text-destructive/50 hover:text-destructive hover:bg-destructive/10 transition-colors">
                  Unlink
                </button>
              ) : (
                <button onClick={() => handleLinkWiki(selectedPath)}
                  className="text-[8px] px-1.5 py-0.5 rounded text-accent/50 hover:text-accent hover:bg-accent/10 transition-colors">
                  Link
                </button>
              )}
              {isEditing ? (
                <>
                  <button onClick={handleSaveEdit}
                    className="text-[9px] px-2 py-0.5 rounded bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25 transition-colors">
                    Save
                  </button>
                  <button onClick={() => setEditingPath(null)}
                    className="text-[9px] px-2 py-0.5 rounded text-muted-foreground/40 hover:text-muted-foreground/60 transition-colors">
                    Cancel
                  </button>
                </>
              ) : (
                <>
                  <button onClick={() => handleStartEdit(selectedPath)}
                    className="text-[9px] px-2 py-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors">
                    Edit
                  </button>
                  <button onClick={() => handleDeletePage(selectedPath)}
                    className="text-[9px] px-2 py-0.5 rounded text-destructive/30 hover:text-destructive transition-colors">
                    Delete
                  </button>
                </>
              )}
            </div>

            {/* Metadata row: path + tags (CVE badges hidden to reduce clutter) */}
            <div className="px-3 py-1.5 border-b border-border/3 space-y-1">
              <span className="text-[8px] font-mono text-muted-foreground/25">{selectedPath}</span>
              {selectedTags.length > 0 && (
                <div className="flex items-center gap-1 flex-wrap">
                  {selectedTags.map((tag) => (
                    <span key={tag} className="text-[7px] px-1 py-0.5 rounded bg-accent/10 text-accent/60">{tag}</span>
                  ))}
                </div>
              )}
            </div>

            {/* Article body */}
            {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
            <div className="flex-1 overflow-y-auto px-3 py-3" onClick={handleContentClick}>
              {articleContents[selectedPath] === undefined ? (
                <div className="flex items-center justify-center py-8">
                  <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/20" />
                </div>
              ) : isEditing ? (
                <textarea
                  value={editContent}
                  onChange={(e) => setEditContent(e.target.value)}
                  className="w-full h-full min-h-[200px] p-2 rounded bg-[var(--bg-hover)]/30 border border-border/15 text-[10px] font-mono text-foreground/80 resize-y outline-none focus:border-accent/40"
                  spellCheck={false}
                />
              ) : selectedBody ? (
                <div className={proseClasses}>
                  <Markdown content={selectedBody} />
                </div>
              ) : (
                <div className="text-[10px] text-muted-foreground/20 py-8 text-center">Empty article</div>
              )}

              {/* Backlinks section */}
              {backlinks.length > 0 && !isEditing && (
                <div className="mt-4 pt-3 border-t border-border/10">
                  <div className="text-[8px] text-muted-foreground/30 uppercase tracking-wider mb-1.5">
                    Referenced by ({backlinks.length})
                  </div>
                  <div className="space-y-0.5">
                    {backlinks.map((bl) => (
                      <button
                        key={bl.source_path}
                        onClick={() => navigateToWikiPage(bl.source_path)}
                        className="flex items-center gap-1.5 w-full px-1.5 py-1 rounded text-left hover:bg-muted/10 transition-colors"
                      >
                        <Link2 className="w-2.5 h-2.5 text-accent/40 flex-shrink-0" />
                        <span className="text-[9px] text-accent/60 truncate">{bl.source_path.replace(/\.md$/, "")}</span>
                        {bl.context && <span className="text-[8px] text-muted-foreground/20 truncate ml-auto">{bl.context}</span>}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </>
        ) : (
          <div className="flex-1 flex flex-col items-center justify-center gap-2 text-muted-foreground/15">
            <BookOpen className="w-10 h-10" />
            <p className="text-[10px]">Select a wiki article from the tree</p>
            {link.wikiPaths.length === 0 && (
              <p className="text-[9px] text-muted-foreground/10">
                Run AI Research to generate wiki articles for this vulnerability
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

