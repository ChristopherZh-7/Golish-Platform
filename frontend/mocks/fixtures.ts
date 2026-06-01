/**
 * Read-only mock data fixtures served by the mock IPC handler (workflows,
 * sessions, approval patterns, prompts, skills, per-project settings) plus
 * the `MockCodebase` shape. Pure data — no state.
 */

// Mock workflows
export const mockWorkflows = [
  { name: "code-review", description: "Review code changes and provide feedback" },
  { name: "test-generation", description: "Generate unit tests for code" },
  { name: "refactor", description: "Suggest code refactoring improvements" },
];

// Mock sessions
export const mockSessions = [
  {
    identifier: "session-2024-01-15-001",
    path: "/home/user/.golish/sessions/session-2024-01-15-001.json",
    workspace_label: "golish",
    workspace_path: "/home/user/golish",
    model: "claude-opus-4.5",
    provider: "anthropic_vertex",
    started_at: "2024-01-15T10:00:00Z",
    ended_at: "2024-01-15T11:30:00Z",
    total_messages: 24,
    distinct_tools: ["read_file", "write_file", "run_command"],
    first_prompt_preview: "Can you help me refactor the authentication module?",
    first_reply_preview: "I'll help you refactor the authentication module...",
  },
  {
    identifier: "session-2024-01-14-002",
    path: "/home/user/.golish/sessions/session-2024-01-14-002.json",
    workspace_label: "golish",
    workspace_path: "/home/user/golish",
    model: "claude-opus-4.5",
    provider: "anthropic_vertex",
    started_at: "2024-01-14T14:00:00Z",
    ended_at: "2024-01-14T16:45:00Z",
    total_messages: 42,
    distinct_tools: ["read_file", "run_command"],
    first_prompt_preview: "Help me add unit tests for the PTY manager",
    first_reply_preview: "I'll help you add unit tests for the PTY manager...",
  },
];

// Mock approval patterns
export const mockApprovalPatterns = [
  {
    tool_name: "read_file",
    total_requests: 50,
    approvals: 50,
    denials: 0,
    always_allow: true,
    last_updated: "2024-01-15T10:00:00Z",
    justifications: [],
  },
  {
    tool_name: "write_file",
    total_requests: 20,
    approvals: 18,
    denials: 2,
    always_allow: false,
    last_updated: "2024-01-15T09:30:00Z",
    justifications: ["Writing config file", "Updating source code"],
  },
  {
    tool_name: "run_command",
    total_requests: 30,
    approvals: 25,
    denials: 5,
    always_allow: false,
    last_updated: "2024-01-15T11:00:00Z",
    justifications: ["Running tests", "Building project"],
  },
];

// Mock prompts
export const mockPrompts = [
  { name: "review", path: "/home/user/.golish/prompts/review.md", source: "global" as const },
  { name: "explain", path: "/home/user/.golish/prompts/explain.md", source: "global" as const },
  { name: "project-context", path: ".golish/prompts/project-context.md", source: "local" as const },
];

// Mock skills
export const mockSkills = [
  {
    name: "code-review",
    path: "/home/user/.golish/skills/code-review",
    source: "global",
    description: "Review code for quality and best practices",
    license: undefined,
    compatibility: undefined,
    metadata: undefined,
    allowed_tools: ["read_file", "glob", "grep"],
    has_scripts: false,
    has_references: false,
    has_assets: false,
  },
  {
    name: "refactor",
    path: "/home/user/.golish/skills/refactor",
    source: "global",
    description: "Refactor code for improved readability and maintainability",
    license: undefined,
    compatibility: undefined,
    metadata: undefined,
    allowed_tools: ["read_file", "write_file", "glob"],
    has_scripts: false,
    has_references: false,
    has_assets: false,
  },
];

// Mock codebase shape (mutable state lives in the IPC module)
export interface MockCodebase {
  path: string;
  file_count: number;
  status: "synced" | "indexing" | "not_indexed" | "error";
  error?: string;
  memory_file?: string;
}

// Mock per-project settings (stored in .golish/project.toml)
export const mockProjectSettings: {
  provider: string | null;
  model: string | null;
  agent_mode: string | null;
} = {
  provider: null,
  model: null,
  agent_mode: null,
};
