import { invoke } from "@/lib/api/client";

export interface FindingsStore {
  findings: Finding[];
}

export interface Finding {
  id: string;
  title: string;
  severity: string;
  cvss?: number;
  url: string;
  target: string;
  targetId?: string;
  description: string;
  steps: string;
  remediation: string;
  tags: string[];
  tool: string;
  template: string;
  references: string[];
  evidence: Evidence[];
  status: string;
  created_at: number;
  updated_at: number;
}

export interface Evidence {
  id: string;
  filename: string;
  mime_type: string;
  caption: string;
  added_at: number;
}

// Tauri commands accept Option<String> for projectPath, so the frontend can
// pass `null` directly without coercing to "" first.
type ProjectPath = string | null;

export const findingsApi = {
  list: (projectPath: ProjectPath) => invoke<FindingsStore>("findings_list", { projectPath }),
  add: (finding: Record<string, unknown>, projectPath: ProjectPath) =>
    invoke("findings_add", { finding, projectPath }),
  delete: (id: string, projectPath: ProjectPath) => invoke("findings_delete", { id, projectPath }),
  update: (finding: Record<string, unknown>, projectPath: ProjectPath) =>
    invoke("findings_update", { finding, projectPath }),
  addEvidence: (
    findingId: string,
    filename: string,
    mimeType: string,
    caption: string,
    dataBase64: string,
    projectPath: ProjectPath
  ) =>
    invoke("findings_add_evidence", {
      findingId,
      filename,
      mimeType,
      caption,
      dataBase64,
      projectPath,
    }),
  removeEvidence: (findingId: string, evidenceId: string, projectPath: ProjectPath) =>
    invoke("findings_remove_evidence", { findingId, evidenceId, projectPath }),
  evidencePath: (findingId: string, evidenceId: string, projectPath: ProjectPath) =>
    invoke<string>("findings_evidence_path", { findingId, evidenceId, projectPath }),
  deduplicate: (projectPath: ProjectPath) => invoke<number>("findings_deduplicate", { projectPath }),
  /**
   * Bulk-import findings parsed by the output parser. Items must be
   * vulnerability-shaped objects (each map of field name → value).
   * Returns the number of new findings actually inserted (dedup-aware).
   */
  importParsed: (
    items: Record<string, string>[],
    toolName: string,
    projectPath: ProjectPath
  ) => invoke<number>("findings_import_parsed", { items, toolName, projectPath }),
};
