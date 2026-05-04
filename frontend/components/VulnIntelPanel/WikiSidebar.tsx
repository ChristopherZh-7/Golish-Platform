import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  FileText,
  Link2,
  Loader2,
  Plus,
  Search,
  X,
} from "lucide-react";
import type { WikiPageInfo, WikiTreeNode } from "@/lib/wiki";
import { cn } from "@/lib/utils";
import { CATEGORY_ICONS, CATEGORY_ORDER, filterTree, STATUS_COLORS } from "./useWikiTab";
import type { VulnLink } from "./types";

interface WikiSidebarProps {
  link: VulnLink;
  selectedPath: string | null;
  setSelectedPath: (path: string | null) => void;
  fullTree: WikiTreeNode[];
  loadingTree: boolean;
  expandedDirs: Set<string>;
  toggleDir: (dir: string) => void;
  linkedSet: Set<string>;
  linkedByCategory: Record<string, WikiPageInfo[]>;
  suggestedPages: WikiPageInfo[];
  searchQuery: string;
  setSearchQuery: (q: string) => void;
  searchResults: { path: string; title: string; snippet: string }[];
  searching: boolean;
  browseAll: boolean;
  setBrowseAll: (v: boolean) => void;
  adding: boolean;
  setAdding: (v: boolean) => void;
  newPath: string;
  setNewPath: (v: string) => void;
  creating: boolean;
  setCreating: (v: boolean) => void;
  createPath: string;
  setCreatePath: (v: string) => void;
  handleLinkWiki: (path: string) => void;
  handleUnlinkWiki: (path: string) => void;
  handleCreatePage: () => void;
  navigateToWikiPage: (path: string) => void;
}

export function WikiSidebar({
  link,
  selectedPath,
  setSelectedPath,
  fullTree,
  loadingTree,
  expandedDirs,
  toggleDir,
  linkedSet,
  linkedByCategory,
  suggestedPages,
  searchQuery,
  setSearchQuery,
  searchResults,
  searching,
  browseAll,
  setBrowseAll,
  adding,
  setAdding,
  newPath,
  setNewPath,
  creating,
  setCreating,
  createPath,
  setCreatePath,
  handleLinkWiki,
  handleUnlinkWiki,
  handleCreatePage,
  navigateToWikiPage,
}: WikiSidebarProps) {
  const displayTree =
    searchQuery && !searchResults.length ? filterTree(fullTree, searchQuery) : browseAll ? fullTree : [];

  const renderTreeNode = (node: WikiTreeNode, depth = 0): React.ReactNode => {
    if (node.is_dir) {
      const isExpanded = expandedDirs.has(node.path) || !!searchQuery;
      const icon = CATEGORY_ICONS[node.name] || "";
      return (
        <div key={node.path}>
          <button
            type="button"
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
          type="button"
          onClick={() => setSelectedPath(node.path)}
          className={cn(
            "flex items-center gap-1.5 flex-1 px-1.5 py-1 rounded text-left transition-colors",
            isSelected
              ? "bg-accent/15 text-accent"
              : "text-muted-foreground/40 hover:bg-muted/10 hover:text-muted-foreground/60"
          )}
          style={{ paddingLeft: `${depth * 12 + 6}px` }}
        >
          <FileText
            className={cn(
              "w-3 h-3 flex-shrink-0",
              isSelected ? "text-accent" : "text-muted-foreground/25"
            )}
          />
          <span className="text-[10px] truncate flex-1">{node.name.replace(/\.md$/, "")}</span>
        </button>
        {!isLinked && (
          <button
            type="button"
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

  return (
    <div className="w-[220px] flex-shrink-0 border-r border-border/10 flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-2 py-1.5 border-b border-border/5">
        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">
          Wiki ({link.wikiPaths.length})
        </span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => {
              setCreating(!creating);
              setAdding(false);
            }}
            className="p-0.5 text-muted-foreground/30 hover:text-emerald-400 transition-colors"
            title="Create new wiki page"
          >
            <Plus className="w-3 h-3" />
          </button>
          <button
            type="button"
            onClick={() => {
              setAdding(!adding);
              setCreating(false);
            }}
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
          {searching && (
            <Loader2 className="absolute right-1.5 top-1/2 -translate-y-1/2 w-2.5 h-2.5 animate-spin text-muted-foreground/30" />
          )}
        </div>
      </div>

      {/* Create form */}
      {creating && (
        <div className="px-2 py-1.5 border-b border-border/5 space-y-1">
          <input
            value={createPath}
            onChange={(e) => setCreatePath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleCreatePage();
            }}
            placeholder="products/myapp/CVE-XXXX.md"
            className="w-full h-5 px-1.5 text-[9px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
          />
          <div className="flex gap-1">
            <button
              type="button"
              onClick={handleCreatePage}
              disabled={!createPath.trim()}
              className="text-[8px] text-emerald-400 disabled:opacity-30"
            >
              Create
            </button>
            <button
              type="button"
              onClick={() => {
                setCreating(false);
                setCreatePath("");
              }}
              className="text-[8px] text-muted-foreground/30"
            >
              Cancel
            </button>
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
          />
          <div className="flex gap-1">
            <button
              type="button"
              onClick={() => {
                handleLinkWiki(newPath.trim());
                setAdding(false);
                setNewPath("");
              }}
              disabled={!newPath.trim()}
              className="text-[8px] text-accent disabled:opacity-30"
            >
              Link
            </button>
            <button
              type="button"
              onClick={() => {
                setAdding(false);
                setNewPath("");
              }}
              className="text-[8px] text-muted-foreground/30"
            >
              Cancel
            </button>
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
              type="button"
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
            {CATEGORY_ORDER.filter((cat) => linkedByCategory[cat]?.length).map((cat) => (
              <div key={cat}>
                <div className="flex items-center gap-1 px-2 py-1 mt-1">
                  <span className="text-[10px]">{CATEGORY_ICONS[cat] || "📄"}</span>
                  <span className="text-[8px] text-muted-foreground/40 uppercase tracking-wider flex-1">
                    {cat}
                  </span>
                  <span className="text-[7px] text-muted-foreground/20">
                    {linkedByCategory[cat].length}
                  </span>
                </div>
                {linkedByCategory[cat].map((info) => {
                  const isSelected = selectedPath === info.path;
                  return (
                    <div key={info.path} className="group/file flex items-center">
                      <button
                        type="button"
                        onClick={() => setSelectedPath(info.path)}
                        className={cn(
                          "flex items-center gap-1.5 flex-1 px-2 py-1 rounded text-left transition-colors min-w-0",
                          isSelected
                            ? "bg-accent/15 text-accent"
                            : "text-foreground/70 hover:bg-muted/10"
                        )}
                      >
                        <FileText
                          className={cn(
                            "w-3 h-3 flex-shrink-0",
                            isSelected ? "text-accent" : "text-blue-400/60"
                          )}
                        />
                        <span className="text-[10px] truncate flex-1">
                          {info.title || info.path.split("/").pop()?.replace(/\.md$/, "")}
                        </span>
                        {info.status && (
                          <span
                            className={cn(
                              "text-[6px] px-1 py-0.5 rounded flex-shrink-0",
                              STATUS_COLORS[info.status] || "text-muted-foreground/30 bg-muted/10"
                            )}
                          >
                            {info.status}
                          </span>
                        )}
                      </button>
                      <button
                        type="button"
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
                  <span className="text-[7px] text-muted-foreground/25 uppercase tracking-wider">
                    Suggested
                  </span>
                </div>
                {suggestedPages.map((info) => (
                  <div key={info.path} className="group/sugg flex items-center">
                    <button
                      type="button"
                      onClick={() => setSelectedPath(info.path)}
                      className="flex items-center gap-1.5 flex-1 px-2 py-1 rounded text-left text-muted-foreground/35 hover:text-foreground/60 hover:bg-muted/10 transition-colors min-w-0"
                    >
                      <FileText className="w-3 h-3 flex-shrink-0 text-muted-foreground/20" />
                      <span className="text-[9px] truncate flex-1">{info.title}</span>
                      <span className="text-[6px] text-muted-foreground/15">{info.category}</span>
                    </button>
                    <button
                      type="button"
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

            {/* Browse all tree */}
            {browseAll && displayTree.length > 0 && (
              <>
                <div className="px-2 py-0.5 mt-2 border-t border-border/5">
                  <span className="text-[7px] text-muted-foreground/25 uppercase tracking-wider">
                    All Pages
                  </span>
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
          type="button"
          onClick={() => setBrowseAll(!browseAll)}
          className="text-[8px] text-muted-foreground/30 hover:text-muted-foreground/60 transition-colors"
        >
          {browseAll ? "Hide tree" : "Browse all..."}
        </button>
      </div>
    </div>
  );
}
