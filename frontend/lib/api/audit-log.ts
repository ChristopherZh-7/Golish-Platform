import { invoke } from "@/lib/api/client";

export interface AgentLogEntry {
  id: string;
  sessionId: string;
  taskId: string | null;
  subtaskId: string | null;
  initiator: string;
  executor: string;
  task: string;
  result: string | null;
  durationMs: number | null;
  createdAt: string;
}

export interface TerminalLogEntry {
  id: string;
  sessionId: string;
  taskId: string | null;
  subtaskId: string | null;
  stream: string;
  content: string;
  createdAt: string;
}

export interface SearchLogEntry {
  id: string;
  sessionId: string;
  taskId: string | null;
  subtaskId: string | null;
  initiator: string | null;
  engine: string;
  query: string;
  result: string | null;
  createdAt: string;
}

export interface PassiveScanEntry {
  id: string;
  targetId: string;
  testType: string;
  payload: string;
  url: string;
  result: string;
  severity: string;
  toolUsed: string;
  testedAt: string;
}

export interface WikiChangeEntry {
  id: number;
  pagePath: string;
  action: string;
  title: string;
  category: string;
  actor: string;
  summary: string;
  createdAt: string;
}

export const auditLogApi = {
  agentLogsList: (projectPath: string | null, limit: number) =>
    invoke<AgentLogEntry[]>("agent_logs_list", { projectPath, limit }),
  terminalLogsList: (projectPath: string | null, limit: number) =>
    invoke<TerminalLogEntry[]>("terminal_logs_list", { projectPath, limit }),
  searchLogsList: (projectPath: string | null, limit: number) =>
    invoke<SearchLogEntry[]>("search_logs_list", { projectPath, limit }),
  passiveScansGlobal: (projectPath: string | null, limit: number) =>
    invoke<PassiveScanEntry[]>("passive_scans_global", { projectPath, limit }),
  wikiChangelogList: (limit: number) => invoke<WikiChangeEntry[]>("wiki_changelog_list", { limit }),
  auditClear: (projectPath: string | null) => invoke("audit_clear", { projectPath }),
};
