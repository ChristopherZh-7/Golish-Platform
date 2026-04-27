import { invoke } from "@tauri-apps/api/core";

export async function getGitBranch(path: string): Promise<string | null> {
  return invoke("get_git_branch", { path });
}

export interface GitStatusEntry {
  path: string;
  index_status: string | null;
  worktree_status: string | null;
  rename_from: string | null;
  rename_to: string | null;
}

export interface GitStatusSummary {
  branch: string | null;
  ahead: number;
  behind: number;
  entries: GitStatusEntry[];
  insertions: number;
  deletions: number;
}

export interface GitDiffResult {
  file: string;
  staged: boolean;
  is_binary: boolean;
  diff: string;
}

export async function gitStatus(workingDirectory: string): Promise<GitStatusSummary> {
  return invoke("git_status", { workingDirectory });
}

export async function gitDiff(
  workingDirectory: string,
  file: string,
  staged?: boolean
): Promise<GitDiffResult> {
  return invoke("git_diff", { workingDirectory, file, staged });
}

export async function gitDiffStaged(workingDirectory: string): Promise<string> {
  return invoke("git_diff_staged", { workingDirectory });
}

export async function gitStage(workingDirectory: string, files: string[]): Promise<void> {
  return invoke("git_stage", { workingDirectory, files });
}

export async function gitUnstage(workingDirectory: string, files: string[]): Promise<void> {
  return invoke("git_unstage", { workingDirectory, files });
}

export async function gitCommit(
  workingDirectory: string,
  message: string,
  options?: { signOff?: boolean; amend?: boolean }
): Promise<void> {
  return invoke("git_commit", {
    workingDirectory,
    message,
    sign_off: options?.signOff ?? false,
    amend: options?.amend ?? false,
  });
}

export async function gitPush(
  workingDirectory: string,
  options?: { force?: boolean; setUpstream?: boolean }
): Promise<void> {
  return invoke("git_push", {
    workingDirectory,
    force: options?.force ?? false,
    set_upstream: options?.setUpstream ?? false,
  });
}

export async function deleteWorktree(
  workingDirectory: string,
  worktreePath: string,
  force?: boolean
): Promise<void> {
  return invoke("git_delete_worktree", { workingDirectory, worktreePath, force });
}
