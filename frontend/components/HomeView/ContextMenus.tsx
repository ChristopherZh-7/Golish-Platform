import { useCallback, useRef } from "react";
import { Trash2, TreePine } from "lucide-react";
import { useDismissMenu } from "@/hooks/useDismissMenu";

export interface ContextMenuState {
  x: number;
  y: number;
  projectPath: string;
  projectName: string;
}

export interface WorktreeContextMenuState {
  x: number;
  y: number;
  projectPath: string;
  worktreePath: string;
  branchName: string;
}

export function ProjectContextMenu({
  x,
  y,
  onNewWorktree,
  onClose,
}: {
  x: number;
  y: number;
  onNewWorktree: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  useDismissMenu(menuRef, onClose);

  const handleNewWorktreeClick = useCallback(() => {
    onNewWorktree();
    onClose();
  }, [onNewWorktree, onClose]);

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-popover border border-border rounded-md shadow-xl py-1 min-w-[160px]"
      style={{ left: x, top: y }}
    >
      <button
        type="button"
        onClick={handleNewWorktreeClick}
        className="w-full flex items-center px-3 py-2 text-sm text-foreground/80 hover:bg-muted hover:text-foreground transition-colors text-left"
      >
        <TreePine size={14} className="mr-2 text-primary" />
        New Worktree
      </button>
    </div>
  );
}

export function WorktreeContextMenu({
  x,
  y,
  onDelete,
  onClose,
}: {
  x: number;
  y: number;
  onDelete: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  useDismissMenu(menuRef, onClose);

  const handleDeleteClick = useCallback(() => {
    onDelete();
    onClose();
  }, [onDelete, onClose]);

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-popover border border-border rounded-md shadow-xl py-1 min-w-[160px]"
      style={{ left: x, top: y }}
    >
      <button
        type="button"
        onClick={handleDeleteClick}
        className="w-full flex items-center px-3 py-2 text-sm text-destructive hover:bg-muted hover:text-destructive/80 transition-colors text-left"
      >
        <Trash2 size={14} className="mr-2" />
        Delete Worktree
      </button>
    </div>
  );
}
