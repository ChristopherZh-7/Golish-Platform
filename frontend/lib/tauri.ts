/**
 * Barrel re-export for all Tauri invoke wrappers.
 *
 * Sub-modules are organized by domain:
 *   tauri/pty.ts   — PTY session lifecycle
 *   tauri/shell.ts — Shell integration, input classification, path completion, IME
 *   tauri/git.ts   — Git operations (status, diff, commit, push, worktree)
 *   tauri/files.ts — File/prompt/skill reading, workspace file listing
 */

export {
  type PtySession,
  ptyCreate,
  ptyWrite,
  ptyResize,
  ptyDestroy,
  ptyGetSession,
  ptyGetForegroundProcess,
  setActiveTerminalSession,
} from "./tauri/pty";

export {
  type IntegrationStatus,
  shellIntegrationStatus,
  shellIntegrationInstall,
  shellIntegrationUninstall,
  type ClassifyResult,
  classifyInput,
  type PathEntryType,
  type PathCompletion,
  type PathCompletionResponse,
  listPathCompletions,
  imeGetSource,
  imeSetSource,
} from "./tauri/shell";

export {
  getGitBranch,
  type GitStatusEntry,
  type GitStatusSummary,
  type GitDiffResult,
  gitStatus,
  gitDiff,
  gitDiffStaged,
  gitStage,
  gitUnstage,
  gitCommit,
  gitPush,
  deleteWorktree,
} from "./tauri/git";

export {
  type PromptInfo,
  listPrompts,
  readPrompt,
  type SkillInfo,
  type SkillFileInfo,
  listSkills,
  readSkill,
  readSkillBody,
  listSkillFiles,
  readSkillFile,
  type FileInfo,
  listWorkspaceFiles,
  readFileAsBase64,
  readTextFile,
} from "./tauri/files";
