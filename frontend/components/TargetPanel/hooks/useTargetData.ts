import { useCallback, useEffect, useMemo, useState } from "react";
import { targets } from "@/lib/api";
import type { TargetStore } from "@/lib/api/targets";
import { logAudit } from "@/lib/audit";
import { onCustomEvent, onEvent, sendCustomEvent } from "@/lib/events";
import type { Target } from "@/lib/pentest/types";
import { getProjectPath } from "@/lib/projects";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { useStore } from "@/store";

interface AddForm {
  name: string;
  value: string;
  notes: string;
  tags: string;
  grp: string;
  owner: string;
  timeWindowStart: string;
  timeWindowEnd: string;
  /** Required when the project is in `redteam` mode; pentest leaves it empty. */
  organizationId?: string;
}

function normalizeTarget(target: Target): Target {
  return {
    ...target,
    ports: Array.isArray(target.ports) ? target.ports : [],
  };
}

export function useTargetData() {
  const workspaceReady = useStore((s) => s.workspaceDataReady);
  const [store, setStore] = useState<TargetStore>({ targets: [] });

  const loadTargets = useCallback(async () => {
    try {
      const data = await targets.listTargets(getProjectPath());
      setStore(data?.targets ? data : { targets: [] });
    } catch (e) {
      console.error("Failed to load targets:", e);
      setTimeout(() => {
        targets
          .listTargets(getProjectPath())
          .then((data) => setStore(data?.targets ? data : { targets: [] }))
          .catch(() => {});
      }, 3000);
    }
  }, []);

  useEffect(() => {
    if (workspaceReady) loadTargets();
  }, [loadTargets, workspaceReady]);

  useEffect(() => {
    // Scoping/recon write organizations (manage_organizations,
    // recon_discover_subsidiaries), not just targets — without these the panel
    // never refreshed after the AI built the org tree.
    const REFRESH_TOOLS = new Set([
      "manage_targets",
      "record_finding",
      "manage_organizations",
      "recon_discover_subsidiaries",
    ]);
    const unlistenAi = onEvent("ai-event", (payload) => {
      const p = payload as { type: string; tool_name?: string };
      if (p.type === "tool_result" && p.tool_name && REFRESH_TOOLS.has(p.tool_name)) {
        loadTargets();
      }
    });
    const unlistenDb = onCustomEvent("db-ready", () => loadTargets());
    const unlistenTargets = onCustomEvent("targets-changed", () => loadTargets());
    const pollInterval = setInterval(loadTargets, 15000);
    return () => {
      runTauriUnlistenFromPromise(unlistenAi);
      runTauriUnlistenFromPromise(unlistenDb);
      runTauriUnlistenFromPromise(unlistenTargets);
      clearInterval(pollInterval);
    };
  }, [loadTargets]);

  const handleAdd = useCallback(
    async (addForm: AddForm): Promise<string | null> => {
      if (!addForm.value.trim()) return null;
      try {
        await targets.addTarget({
          name: addForm.name,
          value: addForm.value.trim(),
          grp: addForm.grp.trim() || undefined,
          owner: addForm.owner.trim() || undefined,
          timeWindowStart: addForm.timeWindowStart.trim() || undefined,
          timeWindowEnd: addForm.timeWindowEnd.trim() || undefined,
          organizationId: addForm.organizationId?.trim() || undefined,
          projectPath: getProjectPath(),
        });
        loadTargets();
        sendCustomEvent("targets-changed").catch(() => {});
        logAudit({ action: "target_added", category: "targets", details: addForm.value.trim() });
        return null;
      } catch (e) {
        const msg = String(e);
        if (msg.includes("duplicate") || msg.includes("unique") || msg.includes("already exists")) {
          return "Target already exists";
        }
        console.error("Failed to add target:", e);
        return msg.slice(0, 100);
      }
    },
    [loadTargets]
  );

  const handleBatchAdd = useCallback(
    async (batchInput: string, grp = "", organizationId?: string, source?: string) => {
      if (!batchInput.trim()) return [];
      try {
        const added = await targets.batchAddTargets({
          values: batchInput,
          grp,
          organizationId,
          source,
          projectPath: getProjectPath(),
        });
        loadTargets();
        const count = added.length;
        if (count > 0) {
          console.info(`Imported ${count} targets`);
        }
        logAudit({
          action: "targets_batch_added",
          category: "targets",
          details: `已添加 ${count} 个目标`,
        });
        return added;
      } catch (e) {
        console.error("Failed to batch add:", e);
        return [];
      }
    },
    [loadTargets]
  );

  const handleDelete = useCallback(
    async (id: string) => {
      try {
        await targets.deleteTarget(id, getProjectPath());
        loadTargets();
        logAudit({
          action: "target_deleted",
          category: "targets",
          details: id,
          entityType: "target",
          entityId: id,
        });
      } catch (e) {
        console.error("Failed to delete target:", e);
      }
    },
    [loadTargets]
  );

  const handleDeleteMany = useCallback(
    async (ids: string[]) => {
      // Bulk-delete the rows behind a synthetic group (unassigned / unresolved /
      // host). Reuse the per-row `target_delete` command (no batch command on
      // the backend) but reload + audit *once* so deleting dozens of rows
      // doesn't trigger one `target_list` round-trip per row.
      const unique = [...new Set(ids)].filter(Boolean);
      if (unique.length === 0) return;
      let deleted = 0;
      for (const id of unique) {
        try {
          await targets.deleteTarget(id, getProjectPath());
          deleted += 1;
        } catch (e) {
          console.error("Failed to delete target:", id, e);
        }
      }
      if (deleted > 0) {
        loadTargets();
        sendCustomEvent("targets-changed").catch(() => {});
        logAudit({
          action: "targets_bulk_deleted",
          category: "targets",
          details: `批量删除 ${deleted}/${unique.length} 个目标`,
        });
      }
    },
    [loadTargets]
  );

  const handleToggleScope = useCallback(
    async (target: Target) => {
      try {
        const newScope = target.scope === "in" ? "out" : "in";
        await targets.updateTarget({
          id: target.id,
          scope: newScope,
          projectPath: getProjectPath(),
        });
        loadTargets();
        logAudit({
          action: "target_scope_changed",
          category: "targets",
          details: `${target.id} scope → ${newScope}`,
          entityType: "target",
          entityId: target.id,
        });
      } catch (e) {
        console.error("Failed to update scope:", e);
      }
    },
    [loadTargets]
  );

  const handleUpdateNotes = useCallback(
    async (id: string, notes: string) => {
      try {
        await targets.updateTarget({ id, notes, projectPath: getProjectPath() });
        loadTargets();
      } catch (e) {
        console.error("Failed to update notes:", e);
      }
    },
    [loadTargets]
  );

  const handleUpdateGrp = useCallback(
    async (id: string, grp: string) => {
      try {
        await targets.updateTarget({ id, grp: grp.trim(), projectPath: getProjectPath() });
        loadTargets();
        logAudit({
          action: "target_grp_changed",
          category: "targets",
          details: `${id} grp → ${grp}`,
          entityType: "target",
          entityId: id,
        });
      } catch (e) {
        console.error("Failed to update grp:", e);
      }
    },
    [loadTargets]
  );

  const handleUpdateOwner = useCallback(
    async (id: string, owner: string) => {
      try {
        await targets.updateTarget({ id, owner: owner.trim(), projectPath: getProjectPath() });
        loadTargets();
      } catch (e) {
        console.error("Failed to update owner:", e);
      }
    },
    [loadTargets]
  );

  const handleUpdateTimeWindow = useCallback(
    async (id: string, timeWindowStart: string, timeWindowEnd: string) => {
      try {
        await targets.updateTarget({
          id,
          timeWindowStart,
          timeWindowEnd,
          projectPath: getProjectPath(),
        });
        loadTargets();
      } catch (e) {
        console.error("Failed to update time window:", e);
      }
    },
    [loadTargets]
  );

  const handleClearAll = useCallback(
    async (confirmMsg: string) => {
      if (!confirm(confirmMsg)) return;
      try {
        await targets.clearAllTargets(getProjectPath());
        loadTargets();
      } catch (e) {
        console.error("Failed to clear:", e);
      }
    },
    [loadTargets]
  );

  const safeTargets = useMemo(() => (store?.targets ?? []).map(normalizeTarget), [store?.targets]);

  const stats = useMemo(
    () => ({
      total: safeTargets.length,
      inScope: safeTargets.filter((t) => t.scope === "in").length,
      outOfScope: safeTargets.filter((t) => t.scope === "out").length,
    }),
    [safeTargets]
  );

  return {
    safeTargets,
    stats,
    handleAdd,
    handleBatchAdd,
    handleDelete,
    handleDeleteMany,
    handleToggleScope,
    handleUpdateNotes,
    handleUpdateGrp,
    handleUpdateOwner,
    handleUpdateTimeWindow,
    handleClearAll,
  };
}
