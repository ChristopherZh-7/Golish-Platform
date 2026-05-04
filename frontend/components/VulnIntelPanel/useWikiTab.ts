import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, vulnLinks } from "@/lib/api";
import { type WikiBacklinkInfo, type WikiPageInfo, type WikiTreeNode, wikiApi } from "@/lib/wiki";
import type { VulnLink } from "./types";

export const CATEGORY_ORDER = [
  "products",
  "techniques",
  "pocs",
  "experience",
  "analysis",
  "uncategorized",
] as const;

export const CATEGORY_ICONS: Record<string, string> = {
  products: "📦",
  techniques: "⚔️",
  pocs: "🔧",
  experience: "📝",
  analysis: "🔬",
  uncategorized: "📄",
};

export const STATUS_COLORS: Record<string, string> = {
  draft: "text-yellow-400 bg-yellow-500/10",
  partial: "text-orange-400 bg-orange-500/10",
  complete: "text-green-400 bg-green-500/10",
  "needs-poc": "text-blue-400 bg-blue-500/10",
  verified: "text-emerald-400 bg-emerald-500/10",
};

export const PROSE_CLASSES =
  "text-[11px] leading-relaxed text-foreground/80 prose prose-invert prose-sm max-w-none prose-headings:text-foreground/90 prose-headings:text-[12px] prose-headings:font-semibold prose-p:text-[11px] prose-p:leading-relaxed prose-code:text-[10px] prose-code:bg-muted/20 prose-code:px-1 prose-code:rounded prose-pre:bg-muted/10 prose-pre:border prose-pre:border-border/10 prose-pre:text-[10px] prose-li:text-[11px] prose-a:text-accent";

export function stripFrontmatter(content: string) {
  const match = content.match(/^---\n[\s\S]*?\n---\n?/);
  return match ? content.slice(match[0].length) : content;
}

export function extractFrontmatterField(content: string, field: string): string | null {
  const fm = content.match(/^---\n([\s\S]*?)\n---/);
  if (!fm) return null;
  const m = fm[1].match(new RegExp(`^${field}:\\s*(.+)$`, "m"));
  return m?.[1]?.trim() || null;
}

export function extractTitle(content: string) {
  const t = extractFrontmatterField(content, "title");
  if (t) return t.replace(/^["']|["']$/g, "");
  const h1 = content.match(/^#\s+(.+)$/m);
  return h1?.[1] || null;
}

export function extractStatus(content: string) {
  return extractFrontmatterField(content, "status");
}

export function extractTags(content: string): string[] {
  const fm = content.match(/^---\n([\s\S]*?)\n---/);
  if (!fm) return [];
  const m = fm[1].match(/tags:\s*\[([^\]]*)\]/);
  if (m)
    return m[1]
      .split(",")
      .map((t) => t.trim().replace(/^["']|["']$/g, ""))
      .filter(Boolean);
  return [];
}

export function filterTree(nodes: WikiTreeNode[], q: string): WikiTreeNode[] {
  if (!q) return nodes;
  const lower = q.toLowerCase();
  return nodes.reduce<WikiTreeNode[]>((acc, node) => {
    if (node.is_dir) {
      const filtered = filterTree(node.children || [], q);
      if (filtered.length > 0) acc.push({ ...node, children: filtered });
    } else if (
      node.name.toLowerCase().includes(lower) ||
      node.path.toLowerCase().includes(lower)
    ) {
      acc.push(node);
    }
    return acc;
  }, []);
}

export function useWikiTab(link: VulnLink, cveId: string, onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void) {
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
  const [searchResults, setSearchResults] = useState<
    { path: string; title: string; snippet: string }[]
  >([]);
  const [searching, setSearching] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createPath, setCreatePath] = useState("");
  const [browseAll, setBrowseAll] = useState(false);

  const linkedSet = useMemo(() => new Set(link.wikiPaths), [link.wikiPaths]);

  const reloadTree = useCallback(() => {
    setLoadingTree(true);
    wikiApi
      .list()
      .then((d) => d as unknown as WikiTreeNode[])
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

  useEffect(() => {
    reloadTree();
  }, [reloadTree]);

  useEffect(() => {
    if (!selectedPath || articleContents[selectedPath] !== undefined) return;
    wikiApi
      .read(selectedPath)
      .then((content) => setArticleContents((prev) => ({ ...prev, [selectedPath]: content })))
      .catch(() => setArticleContents((prev) => ({ ...prev, [selectedPath]: "" })));
    // selectedPath is guaranteed non-null here; using ternary to satisfy TypeScript
  }, [selectedPath, selectedPath ? articleContents[selectedPath] : undefined]);

  const reindexDone = useRef(false);
  const [indexReady, setIndexReady] = useState(false);
  useEffect(() => {
    if (reindexDone.current) return;
    reindexDone.current = true;
    wikiApi
      .reindex()
      .catch(() => {})
      .finally(() => setIndexReady(true));
  }, []);

  useEffect(() => {
    if (!indexReady) return;
    if (link.wikiPaths.length === 0) {
      setLinkedPageInfos([]);
      return;
    }
    invoke<WikiPageInfo[]>("wiki_pages_for_paths", { paths: link.wikiPaths })
      .then(setLinkedPageInfos)
      .catch(() => setLinkedPageInfos([]));
  }, [link.wikiPaths, indexReady]);

  useEffect(() => {
    invoke<WikiPageInfo[]>("wiki_suggest_for_cve", { cveId, limit: 8 })
      .then(setSuggestedPages)
      .catch(() => setSuggestedPages([]));
  }, [cveId]);

  useEffect(() => {
    if (!selectedPath) {
      setBacklinks([]);
      return;
    }
    invoke<WikiBacklinkInfo[]>("wiki_backlinks", { path: selectedPath })
      .then(setBacklinks)
      .catch(() => setBacklinks([]));
  }, [selectedPath]);

  const linkedByCategory = useMemo(() => {
    const groups: Record<string, WikiPageInfo[]> = {};
    for (const info of linkedPageInfos) {
      const cat = info.category || "uncategorized";
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(info);
    }
    const infoPathSet = new Set(linkedPageInfos.map((p) => p.path));
    for (const wp of link.wikiPaths) {
      if (!infoPathSet.has(wp)) {
        if (!groups.uncategorized) groups.uncategorized = [];
        groups.uncategorized.push({
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

  const toggleDir = useCallback((dir: string) => {
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(dir)) next.delete(dir);
      else next.add(dir);
      return next;
    });
  }, []);

  const handleLinkWiki = useCallback(
    (path: string) => {
      onUpdateLink((l) => ({
        ...l,
        wikiPaths: l.wikiPaths.includes(path) ? l.wikiPaths : [...l.wikiPaths, path],
      }));
      vulnLinks.addWikiLink(cveId, path).catch(console.error);
    },
    [onUpdateLink, cveId]
  );

  const handleUnlinkWiki = useCallback(
    (path: string) => {
      onUpdateLink((l) => ({ ...l, wikiPaths: l.wikiPaths.filter((p) => p !== path) }));
      vulnLinks.removeWikiLink(cveId, path).catch(console.error);
      if (selectedPath === path) setSelectedPath(null);
    },
    [onUpdateLink, cveId, selectedPath]
  );

  const handleStartEdit = useCallback(
    (path: string) => {
      setEditingPath(path);
      setEditContent(articleContents[path] || "");
    },
    [articleContents]
  );

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

  const handleDeletePage = useCallback(
    async (path: string) => {
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
    },
    [selectedPath, linkedSet, handleUnlinkWiki, reloadTree]
  );

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

  const handleSearch = useCallback(async (query: string) => {
    if (!query.trim()) {
      setSearchResults([]);
      return;
    }
    setSearching(true);
    try {
      const results = await invoke<
        { path: string; title: string; category: string; tags: string[]; status: string | null }[]
      >("wiki_search_db", { query: query.trim(), limit: 20 });
      setSearchResults(
        results.map((r) => ({
          path: r.path,
          title: r.title || r.path,
          snippet: `[${r.category}] ${r.tags.join(", ")}${r.status ? ` • ${r.status}` : ""}`,
        }))
      );
    } catch {
      setSearchResults([]);
    }
    setSearching(false);
  }, []);

  useEffect(() => {
    const t = setTimeout(() => handleSearch(searchQuery), 300);
    return () => clearTimeout(t);
  }, [searchQuery, handleSearch]);

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

  const displayTree =
    searchQuery && !searchResults.length
      ? filterTree(fullTree, searchQuery)
      : browseAll
        ? fullTree
        : [];

  const selectedContent = selectedPath ? articleContents[selectedPath] || "" : "";
  const selectedTitle = selectedContent
    ? extractTitle(selectedContent) || selectedPath?.split("/").pop()?.replace(/\.md$/, "")
    : null;
  const selectedStatus = selectedContent ? extractStatus(selectedContent) : null;
  const selectedBody = selectedContent ? stripFrontmatter(selectedContent) : "";
  const selectedTags = selectedContent ? extractTags(selectedContent) : [];
  const isEditing = editingPath === selectedPath;

  return {
    fullTree,
    loadingTree,
    selectedPath,
    setSelectedPath,
    articleContents,
    editingPath,
    setEditingPath,
    editContent,
    setEditContent,
    expandedDirs,
    linkedPageInfos,
    suggestedPages,
    backlinks,
    adding,
    setAdding,
    newPath,
    setNewPath,
    searchQuery,
    setSearchQuery,
    searchResults,
    searching,
    creating,
    setCreating,
    createPath,
    setCreatePath,
    browseAll,
    setBrowseAll,
    linkedSet,
    linkedByCategory,
    toggleDir,
    handleLinkWiki,
    handleUnlinkWiki,
    handleStartEdit,
    handleSaveEdit,
    handleDeletePage,
    handleCreatePage,
    handleSearch,
    navigateToWikiPage,
    displayTree,
    selectedContent,
    selectedTitle,
    selectedStatus,
    selectedBody,
    selectedTags,
    isEditing,
  };
}
