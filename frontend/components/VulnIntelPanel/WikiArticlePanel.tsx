import { BookOpen, FileText, Link2, Loader2 } from "lucide-react";
import { useCallback } from "react";
import { Markdown } from "@/components/Markdown";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import type { WikiBacklinkInfo } from "@/lib/wiki";
import { cn } from "@/lib/utils";
import { PROSE_CLASSES, STATUS_COLORS } from "./useWikiTab";
import type { VulnLink } from "./types";

interface WikiArticlePanelProps {
  selectedPath: string | null;
  articleContents: Record<string, string>;
  editingPath: string | null;
  editContent: string;
  setEditContent: (v: string) => void;
  setEditingPath: (p: string | null) => void;
  isEditing: boolean;
  selectedTitle: string | null | undefined;
  selectedStatus: string | null;
  selectedBody: string;
  selectedTags: string[];
  linkedSet: Set<string>;
  backlinks: WikiBacklinkInfo[];
  link: VulnLink;
  handleLinkWiki: (path: string) => void;
  handleUnlinkWiki: (path: string) => void;
  handleStartEdit: (path: string) => void;
  handleSaveEdit: () => void;
  handleDeletePage: (path: string) => void;
  navigateToWikiPage: (path: string) => void;
}

export function WikiArticlePanel({
  selectedPath,
  articleContents,
  editingPath,
  editContent,
  setEditContent,
  setEditingPath,
  isEditing,
  selectedTitle,
  selectedStatus,
  selectedBody,
  selectedTags,
  linkedSet,
  backlinks,
  handleLinkWiki,
  handleUnlinkWiki,
  handleStartEdit,
  handleSaveEdit,
  handleDeletePage,
  navigateToWikiPage,
  link,
}: WikiArticlePanelProps) {
  const handleContentClick = useCallback(
    (e: React.MouseEvent) => {
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
    },
    [selectedPath, navigateToWikiPage]
  );

  if (!selectedPath) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-2 text-muted-foreground/15">
        <BookOpen className="w-10 h-10" />
        <p className="text-[10px]">Select a wiki article from the tree</p>
        {link.wikiPaths.length === 0 && (
          <p className="text-[9px] text-muted-foreground/10">
            Run AI Research to generate wiki articles for this vulnerability
          </p>
        )}
      </div>
    );
  }

  return (
    <>
      {/* Article header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/5">
        <FileText className="w-3.5 h-3.5 text-blue-400/60 flex-shrink-0" />
        <span className="text-[11px] text-foreground/80 font-medium truncate flex-1">
          {selectedTitle || selectedPath}
        </span>
        {selectedStatus && (
          <span
            className={cn(
              "text-[8px] px-1.5 py-0.5 rounded",
              STATUS_COLORS[selectedStatus] || "text-muted-foreground/40 bg-muted/10"
            )}
          >
            {selectedStatus}
          </span>
        )}
        {linkedSet.has(selectedPath) ? (
          <button
            type="button"
            onClick={() => handleUnlinkWiki(selectedPath)}
            className="text-[8px] px-1.5 py-0.5 rounded text-destructive/50 hover:text-destructive hover:bg-destructive/10 transition-colors"
          >
            Unlink
          </button>
        ) : (
          <button
            type="button"
            onClick={() => handleLinkWiki(selectedPath)}
            className="text-[8px] px-1.5 py-0.5 rounded text-accent/50 hover:text-accent hover:bg-accent/10 transition-colors"
          >
            Link
          </button>
        )}
        {isEditing ? (
          <>
            <button
              type="button"
              onClick={handleSaveEdit}
              className="text-[9px] px-2 py-0.5 rounded bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25 transition-colors"
            >
              Save
            </button>
            <button
              type="button"
              onClick={() => setEditingPath(null)}
              className="text-[9px] px-2 py-0.5 rounded text-muted-foreground/40 hover:text-muted-foreground/60 transition-colors"
            >
              Cancel
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => handleStartEdit(selectedPath)}
              className="text-[9px] px-2 py-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors"
            >
              Edit
            </button>
            <button
              type="button"
              onClick={() => handleDeletePage(selectedPath)}
              className="text-[9px] px-2 py-0.5 rounded text-destructive/30 hover:text-destructive transition-colors"
            >
              Delete
            </button>
          </>
        )}
      </div>

      {/* Metadata row */}
      <div className="px-3 py-1.5 border-b border-border/3 space-y-1">
        <span className="text-[8px] font-mono text-muted-foreground/25">{selectedPath}</span>
        {selectedTags.length > 0 && (
          <div className="flex items-center gap-1 flex-wrap">
            {selectedTags.map((tag) => (
              <span
                key={tag}
                className="text-[7px] px-1 py-0.5 rounded bg-accent/10 text-accent/60"
              >
                {tag}
              </span>
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
          <MarkdownEditor
            editorKey={editingPath || ""}
            value={editContent}
            onChange={setEditContent}
          />
        ) : selectedBody ? (
          <div className={PROSE_CLASSES}>
            <Markdown content={selectedBody} />
          </div>
        ) : (
          <div className="text-[10px] text-muted-foreground/20 py-8 text-center">Empty article</div>
        )}

        {/* Backlinks */}
        {backlinks.length > 0 && !isEditing && (
          <div className="mt-4 pt-3 border-t border-border/10">
            <div className="text-[8px] text-muted-foreground/30 uppercase tracking-wider mb-1.5">
              Referenced by ({backlinks.length})
            </div>
            <div className="space-y-0.5">
              {backlinks.map((bl) => (
                <button
                  type="button"
                  key={bl.source_path}
                  onClick={() => navigateToWikiPage(bl.source_path)}
                  className="flex items-center gap-1.5 w-full px-1.5 py-1 rounded text-left hover:bg-muted/10 transition-colors"
                >
                  <Link2 className="w-2.5 h-2.5 text-accent/40 flex-shrink-0" />
                  <span className="text-[9px] text-accent/60 truncate">
                    {bl.source_path.replace(/\.md$/, "")}
                  </span>
                  {bl.context && (
                    <span className="text-[8px] text-muted-foreground/20 truncate ml-auto">
                      {bl.context}
                    </span>
                  )}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </>
  );
}
