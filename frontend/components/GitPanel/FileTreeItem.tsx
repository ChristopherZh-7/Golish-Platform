import {
  ChevronDown,
  ChevronRight,
  File,
  GitCommitHorizontal,
  Minus,
  Pencil,
  Plus,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import type { GitChange } from "@/lib/git";
import { cn } from "@/lib/utils";

export interface TreeNode {
  name: string;
  path: string;
  isDirectory: boolean;
  children: TreeNode[];
  change?: GitChange;
}

export function buildFileTree(changes: GitChange[]): TreeNode[] {
  const root: TreeNode[] = [];

  for (const change of changes) {
    const parts = change.path.split("/");
    let currentLevel = root;

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      const isFile = i === parts.length - 1;
      const pathSoFar = parts.slice(0, i + 1).join("/");

      let existing = currentLevel.find((n) => n.name === part);

      if (!existing) {
        existing = {
          name: part,
          path: pathSoFar,
          isDirectory: !isFile,
          children: [],
          change: isFile ? change : undefined,
        };
        currentLevel.push(existing);
      }

      if (!isFile) {
        currentLevel = existing.children;
      }
    }
  }

  const compactNodes = (nodes: TreeNode[]): TreeNode[] => {
    return nodes.map((node) => {
      if (!node.isDirectory) return node;
      let compacted = { ...node, children: compactNodes(node.children) };
      while (compacted.children.length === 1 && compacted.children[0].isDirectory) {
        const child = compacted.children[0];
        compacted = {
          ...compacted,
          name: `${compacted.name}/${child.name}`,
          path: child.path,
          children: child.children,
        };
      }
      return compacted;
    });
  };

  const sortNodes = (nodes: TreeNode[]): TreeNode[] => {
    return nodes
      .map((node) => ({
        ...node,
        children: sortNodes(node.children),
      }))
      .sort((a, b) => {
        if (a.isDirectory && !b.isDirectory) return -1;
        if (!a.isDirectory && b.isDirectory) return 1;
        return a.name.localeCompare(b.name);
      });
  };

  return sortNodes(compactNodes(root));
}

export function FileTreeItem({
  node,
  depth,
  onStage,
  onUnstage,
  onDiff,
  actionLabel,
  isStaged,
}: {
  node: TreeNode;
  depth: number;
  onStage?: (path: string) => void;
  onUnstage?: (path: string) => void;
  onDiff?: (change: GitChange) => void;
  actionLabel: string;
  isStaged: boolean;
}) {
  const [expanded, setExpanded] = useState(true);

  const StatusIcon = useMemo(() => {
    if (!node.change) return { icon: File, className: "text-muted-foreground" };
    switch (node.change.kind) {
      case "added":
      case "untracked":
        return { icon: Plus, className: "text-emerald-400" };
      case "deleted":
        return { icon: Minus, className: "text-red-400" };
      case "modified":
      case "renamed":
        return { icon: Pencil, className: "text-amber-400" };
      case "conflict":
        return { icon: File, className: "text-pink-400" };
      default:
        return { icon: File, className: "text-muted-foreground" };
    }
  }, [node.change]);

  if (node.isDirectory) {
    return (
      <div>
        <button
          type="button"
          className="w-full flex items-center gap-1 py-0.5 px-1 rounded hover:bg-muted/40 cursor-pointer select-none text-left"
          style={{ paddingLeft: `${depth * 20 + 8}px` }}
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? (
            <ChevronDown className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
          ) : (
            <ChevronRight className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
          )}
          <span className="text-xs text-foreground truncate">{node.name}</span>
        </button>
        {expanded &&
          node.children.map((child) => (
            <FileTreeItem
              key={child.path}
              node={child}
              depth={depth + 1}
              onStage={onStage}
              onUnstage={onUnstage}
              onDiff={onDiff}
              actionLabel={actionLabel}
              isStaged={isStaged}
            />
          ))}
      </div>
    );
  }

  return (
    <button
      type="button"
      className="w-full group flex items-center gap-1 py-0.5 px-1 rounded hover:bg-muted/40 cursor-pointer text-left"
      style={{ paddingLeft: `${depth * 20 + 8}px` }}
      onClick={() => node.change && onDiff?.(node.change)}
    >
      <StatusIcon.icon className={cn("w-3.5 h-3.5 shrink-0", StatusIcon.className)} />
      <span className="text-xs text-foreground truncate flex-1">{node.name}</span>
      <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
        {isStaged && onUnstage && (
          <Button
            variant="ghost"
            size="icon"
            className="h-5 w-5"
            onClick={(e) => {
              e.stopPropagation();
              onUnstage(node.path);
            }}
          >
            <X className="w-3 h-3" />
          </Button>
        )}
        {!isStaged && onStage && (
          <Button
            variant="ghost"
            size="icon"
            className="h-5 w-5 text-emerald-400"
            onClick={(e) => {
              e.stopPropagation();
              onStage(node.path);
            }}
          >
            <GitCommitHorizontal className="w-3 h-3" />
          </Button>
        )}
      </div>
    </button>
  );
}
