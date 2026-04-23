import { Loader2, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { createGitWorktree, listGitBranches } from "@/lib/indexer";
import { logger } from "@/lib/logger";
import { CustomSelect } from "@/components/ui/custom-select";

export interface NewWorktreeModalProps {
  isOpen: boolean;
  projectPath: string;
  projectName: string;
  onClose: () => void;
  onSuccess: (worktreePath: string) => void;
}

export function NewWorktreeModal({
  isOpen,
  projectPath,
  projectName,
  onClose,
  onSuccess,
}: NewWorktreeModalProps) {
  const [branchName, setBranchName] = useState("");
  const [baseBranch, setBaseBranch] = useState("");
  const [availableBranches, setAvailableBranches] = useState<string[]>([]);
  const [isLoadingBranches, setIsLoadingBranches] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load branches when modal opens
  useEffect(() => {
    if (isOpen && projectPath) {
      setIsLoadingBranches(true);
      setError(null);
      listGitBranches(projectPath)
        .then((branches) => {
          setAvailableBranches(branches);
          // Default to main or master if available
          const defaultBranch = branches.find((b) => b === "main" || b === "master");
          if (defaultBranch) {
            setBaseBranch(defaultBranch);
          } else if (branches.length > 0) {
            setBaseBranch(branches[0]);
          }
        })
        .catch((err) => {
          logger.error("Failed to load branches:", err);
          setError("Failed to load branches");
        })
        .finally(() => {
          setIsLoadingBranches(false);
        });
    }
  }, [isOpen, projectPath]);

  // Reset form when modal closes
  useEffect(() => {
    if (!isOpen) {
      setBranchName("");
      setBaseBranch("");
      setError(null);
    }
  }, [isOpen]);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();

      if (!branchName.trim()) {
        setError("Branch name is required");
        return;
      }

      if (!baseBranch) {
        setError("Base branch is required");
        return;
      }

      setIsCreating(true);
      setError(null);

      try {
        const result = await createGitWorktree(projectPath, branchName.trim(), baseBranch);
        onSuccess(result.path);
        onClose();
      } catch (err) {
        logger.error("Failed to create worktree:", err);
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setIsCreating(false);
      }
    },
    [projectPath, branchName, baseBranch, onSuccess, onClose]
  );

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: backdrop dismiss */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: backdrop dismiss */}
      <div className="absolute inset-0 bg-black/60" onClick={onClose} />

      {/* Modal */}
      <div className="relative bg-card border border-border rounded-lg shadow-xl w-full max-w-md mx-4">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border">
          <h2 className="text-lg font-semibold text-foreground/90">New Worktree</h2>
          <button
            type="button"
            onClick={onClose}
            className="p-1 hover:bg-muted rounded transition-colors"
            aria-label="Close"
          >
            <X size={18} className="text-muted-foreground" />
          </button>
        </div>

        {/* Body */}
        <form onSubmit={handleSubmit} className="p-4 space-y-4">
          <div className="text-sm text-muted-foreground mb-4">
            Create a new worktree for{" "}
            <span className="text-foreground/90 font-medium">{projectName}</span>
          </div>

          {/* Branch name */}
          <label className="block">
            <span className="block text-sm font-medium text-foreground/80 mb-1">Branch Name</span>
            <input
              type="text"
              value={branchName}
              onChange={(e) => setBranchName(e.target.value)}
              placeholder="feature/my-new-feature"
              className="w-full px-3 py-2 bg-background border border-border rounded-md text-foreground/90 placeholder-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-ring focus:border-transparent"
              disabled={isCreating}
            />
          </label>

          {/* Base branch */}
          <div className="block">
            <span className="block text-sm font-medium text-foreground/80 mb-1">Base Branch</span>
            {isLoadingBranches ? (
              <div className="flex items-center text-muted-foreground text-sm py-2">
                <Loader2 size={14} className="animate-spin mr-2" />
                Loading branches...
              </div>
            ) : (
              <CustomSelect
                value={baseBranch}
                onChange={setBaseBranch}
                options={availableBranches.map((branch) => ({
                  value: branch,
                  label: branch,
                }))}
              />
            )}
          </div>

          {/* Error */}
          {error && (
            <div className="text-sm text-destructive bg-destructive/10 border border-destructive/30 rounded-md p-3">
              {error}
            </div>
          )}

          {/* Actions */}
          <div className="flex justify-end gap-2 pt-2">
            <button
              type="button"
              onClick={onClose}
              disabled={isCreating}
              className="px-4 py-2 text-sm text-foreground/80 hover:text-foreground hover:bg-muted rounded-md transition-colors disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isCreating || isLoadingBranches || !branchName.trim() || !baseBranch}
              className="px-4 py-2 text-sm bg-primary text-primary-foreground rounded-md hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center"
            >
              {isCreating ? (
                <>
                  <Loader2 size={14} className="animate-spin mr-2" />
                  Creating...
                </>
              ) : (
                "Create Worktree"
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
