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

export const findingsApi = {
  list: (projectPath: string) =>
    invoke<FindingsStore>("findings_list", { projectPath }),
  add: (finding: Record<string, unknown>, projectPath: string) =>
    invoke("findings_add", { finding, projectPath }),
  delete: (id: string, projectPath: string) =>
    invoke("findings_delete", { id, projectPath }),
  update: (finding: Record<string, unknown>, projectPath: string) =>
    invoke("findings_update", { finding, projectPath }),
  addEvidence: (findingId: string, filename: string, mimeType: string, caption: string, dataBase64: string, projectPath: string) =>
    invoke("findings_add_evidence", { findingId, filename, mimeType, caption, dataBase64, projectPath }),
  removeEvidence: (findingId: string, evidenceId: string, projectPath: string) =>
    invoke("findings_remove_evidence", { findingId, evidenceId, projectPath }),
  evidencePath: (findingId: string, evidenceId: string, projectPath: string) =>
    invoke<string>("findings_evidence_path", { findingId, evidenceId, projectPath }),
  deduplicate: (projectPath: string) =>
    invoke<number>("findings_deduplicate", { projectPath }),
};
