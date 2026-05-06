/**
 * Store-aware `logAudit` helper — auto-injects projectPath from
 * the global store, then fire-and-forget delegates to the typed
 * facade in `@/lib/api/audit-log`.
 *
 * IPC layer lives at `@/lib/api/audit-log`.
 */

import { logAuditEntry } from "@/lib/api/audit-log";
import { getProjectPath } from "@/lib/projects";

interface AuditPayload {
  action: string;
  category: string;
  details: string;
  entityType?: string;
  entityId?: string;
}

export function logAudit(payload: AuditPayload): void {
  logAuditEntry({ ...payload, projectPath: getProjectPath() }).catch(() => {});
}
