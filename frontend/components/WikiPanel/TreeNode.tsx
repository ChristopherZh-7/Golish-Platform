import type React from "react";
import { ChevronDown, ChevronRight, FolderOpen, FolderPlus, Plus, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WikiEntry } from "@/lib/wiki";
import { FileIcon } from "./FileIcon";

interface TreeNodeProps {
  entry: WikiEntry;
  depth: number;
  expandedDirs: Set<string>;
  activePath: string | null;
  creating: { type: "file" | "folder"; parentPath: string } | null;
  toggleDir: (path: string) => void;
  openFile: (path: string, fileName?: string) => void;
  startCreate: (type: "file" | "folder", parentPath: string) => void;
  setDeleteTarget: (entry: WikiEntry | null) => void;
  renderCreateInput: (depth: number) => React.ReactNode;
}

export function TreeNode({
  entry, depth, expandedDirs, activePath, creating,
  toggleDir, openFile, startCreate, setDeleteTarget, renderCreateInput,
}: TreeNodeProps) {
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
            {creating && creating.parentPath === entry.path && renderCreateInput(depth + 1)}
            {entry.children?.map((child) => (
              <TreeNode
                key={child.path}
                entry={child}
                depth={depth + 1}
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
}
