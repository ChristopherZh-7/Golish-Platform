import {
  ChevronDown,
  ChevronRight,
  File,
  FolderOpen,
  GitBranch,
  Minus,
  Plus,
  TreePine,
  X,
} from "lucide-react";
import { memo } from "react";
import type { ProjectInfo, RecentDirectory } from "@/lib/indexer";

const StatsBadge = memo(function StatsBadge({
  fileCount,
  insertions,
  deletions,
}: {
  fileCount: number;
  insertions: number;
  deletions: number;
}) {
  if (fileCount === 0 && insertions === 0 && deletions === 0) {
    return null;
  }

  return (
    <div className="flex items-center bg-background px-2 py-1 rounded-full border border-border space-x-2 text-xs text-muted-foreground">
      {fileCount > 0 && (
        <div className="flex items-center">
          <File size={12} className="mr-0.5 text-muted-foreground" />
          <span>{fileCount}</span>
        </div>
      )}
      {insertions > 0 && (
        <div className="flex items-center">
          <Plus size={12} className="mr-0.5 text-[var(--ansi-green)]" />
          <span>{insertions}</span>
        </div>
      )}
      {deletions > 0 && (
        <div className="flex items-center">
          <Minus size={12} className="mr-0.5 text-[var(--ansi-red)]" />
          <span>{deletions}</span>
        </div>
      )}
    </div>
  );
});

const WorktreeBadge = memo(function WorktreeBadge({ count }: { count: number }) {
  return (
    <div className="flex items-center bg-background px-2 py-1 rounded-full border border-border text-xs text-muted-foreground">
      <TreePine size={14} className="mr-1 text-primary" />
      {count}
    </div>
  );
});

export const ProjectRow = memo(function ProjectRow({
  project,
  isExpanded,
  onToggle,
  onOpenDirectory,
  onContextMenu,
  onWorktreeContextMenu,
  onDelete,
}: {
  project: ProjectInfo;
  isExpanded: boolean;
  onToggle: () => void;
  onOpenDirectory: (path: string) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onWorktreeContextMenu: (e: React.MouseEvent, worktreePath: string, branchName: string) => void;
  onDelete: () => void;
}) {
  return (
    <div className="border-b border-border/50 last:border-0">
      <div
        role="button"
        tabIndex={0}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle();
          }
        }}
        onContextMenu={onContextMenu}
        className="w-full flex items-center justify-between p-3 hover:bg-muted transition-colors group text-left cursor-pointer"
      >
        <div className="flex items-center min-w-0 mr-4">
          <div className="mr-2 flex-shrink-0 hover:bg-border rounded p-0.5 transition-colors">
            {isExpanded ? (
              <ChevronDown size={14} className="text-muted-foreground" />
            ) : (
              <ChevronRight size={14} className="text-muted-foreground" />
            )}
          </div>
          <FolderOpen
            size={16}
            className="text-muted-foreground mr-3 flex-shrink-0 group-hover:text-primary transition-colors"
          />
          <div className="min-w-0">
            <div className="text-sm font-medium text-foreground/80 truncate group-hover:text-foreground transition-colors">
              {project.name}
            </div>
          </div>
        </div>

        <div className="flex items-center text-xs text-muted-foreground flex-shrink-0 space-x-3">
          <WorktreeBadge count={project.branches.length} />
          <span>{project.last_activity}</span>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
            className="opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive flex-shrink-0"
            title="Delete project"
          >
            <X size={14} />
          </button>
        </div>
      </div>

      {isExpanded && project.branches.length > 0 && (
        <div className="bg-background border-t border-border/50 max-h-[420px] overflow-y-auto custom-scrollbar">
          {project.branches.map((branch) => (
            <button
              type="button"
              key={branch.name}
              onClick={() => onOpenDirectory(branch.path)}
              onContextMenu={(e) => onWorktreeContextMenu(e, branch.path, branch.name)}
              className="w-full flex items-center p-3 pl-12 hover:bg-card transition-colors text-left border-b border-border/30 last:border-0 group"
            >
              <div className="flex items-center min-w-0 w-[450px] mr-4">
                <div className="min-w-0">
                  <div className="flex items-center text-xs text-muted-foreground">
                    <GitBranch size={12} className="mr-1 text-primary flex-shrink-0" />
                    <span className="text-foreground/80 truncate">{branch.name}</span>
                  </div>
                  <div className="text-xs text-muted-foreground/60 truncate font-mono mt-0.5">
                    {branch.path}
                  </div>
                </div>
              </div>

              <StatsBadge
                fileCount={branch.file_count}
                insertions={branch.insertions}
                deletions={branch.deletions}
              />

              <div className="flex items-center text-xs text-muted-foreground flex-shrink-0 ml-auto space-x-2">
                <span>{branch.last_activity}</span>
                <ChevronRight
                  size={14}
                  className="opacity-0 group-hover:opacity-100 transition-opacity text-primary"
                />
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
});

export const RecentDirectoryRow = memo(function RecentDirectoryRow({
  directory,
  onOpen,
  onRemove,
}: {
  directory: RecentDirectory;
  onOpen: () => void;
  onRemove: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="w-full flex items-center p-3 hover:bg-muted transition-colors group text-left border-b border-border/50 last:border-0"
    >
      <div className="flex items-center min-w-0 w-[500px] mr-4">
        <FolderOpen
          size={16}
          className="text-muted-foreground mr-3 flex-shrink-0 group-hover:text-primary transition-colors"
        />
        <div className="min-w-0">
          <div className="text-sm font-medium text-foreground/80 truncate group-hover:text-foreground transition-colors">
            {directory.name}
          </div>
          {directory.branch && (
            <div className="flex items-center text-xs text-muted-foreground opacity-60">
              <GitBranch size={12} className="mr-1 text-primary" />
              <span className="text-foreground/80">{directory.branch}</span>
            </div>
          )}
        </div>
      </div>

      <StatsBadge
        fileCount={directory.file_count}
        insertions={directory.insertions}
        deletions={directory.deletions}
      />

      <div className="flex items-center text-xs text-muted-foreground flex-shrink-0 ml-auto space-x-2">
        <span>{directory.last_accessed}</span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          className="opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive flex-shrink-0"
          title="Remove from recent"
        >
          <X size={14} />
        </button>
      </div>
    </button>
  );
});
