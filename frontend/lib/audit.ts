import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";

interface AuditPayload {
  action: string;
  category: string;
  details: string;
  entityType?: string;
  entityId?: string;
}

export function logAudit(payload: AuditPayload): void {
  invoke("audit_log", {
    ...payload,
    projectPath: getProjectPath(),
  }).catch(() => {});
}
