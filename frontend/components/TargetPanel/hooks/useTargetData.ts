import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { logAudit } from "@/lib/audit";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import { type Target } from "@/lib/pentest/types";

interface TargetStore {
  targets: Target[];
}

interface AddForm {
  name: string;
  value: string;
  notes: string;
  tags: string;
}



export function useTargetData() {
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const workspaceReady = useStore((s) => s.workspaceDataReady);
  const [store, setStore] = useState<TargetStore>({ targets: [] });

  const loadTargets = useCallback(async () => {
    try {
      const data = await invoke<TargetStore>("target_list", { projectPath: getProjectPath() });
      setStore(data && data.targets ? data : { targets: [] });
    } catch (e) {
      console.error("Failed to load targets:", e);
      setTimeout(() => {
        invoke<TargetStore>("target_list", { projectPath: getProjectPath() })
          .then((data) => setStore(data && data.targets ? data : { targets: [] }))
          .catch(() => {});
      }, 3000);
    }
  }, []);

  useEffect(() => {
    if (workspaceReady) loadTargets();
  }, [loadTargets, currentProjectPath, workspaceReady]);

  useEffect(() => {
    const REFRESH_TOOLS = new Set(["manage_targets", "record_finding", "run_pipeline"]);
    const unlistenAi = listen<{ type: string; tool_name?: string }>("ai-event", (event) => {
      if (event.payload.type === "tool_result" && event.payload.tool_name && REFRESH_TOOLS.has(event.payload.tool_name)) {
        loadTargets();
      }
    });
    const unlistenPipeline = listen<{ status: string }>("pipeline-event", (event) => {
      if (event.payload.status === "completed" || event.payload.status === "error") {
        loadTargets();
      }
    });
    const unlistenDb = listen("db-ready", () => loadTargets());
    const unlistenTargets = listen("targets-changed", () => loadTargets());
    const pollInterval = setInterval(loadTargets, 15000);
    return () => {
      runTauriUnlistenFromPromise(unlistenAi);
      runTauriUnlistenFromPromise(unlistenPipeline);
      runTauriUnlistenFromPromise(unlistenDb);
      runTauriUnlistenFromPromise(unlistenTargets);
      clearInterval(pollInterval);
    };
  }, [loadTargets]);

  const handleAdd = useCallback(async (addForm: AddForm): Promise<string | null> => {
    if (!addForm.value.trim()) return null;
    try {
      await invoke("target_add", {
        name: addForm.name,
        value: addForm.value.trim(),
        notes: addForm.notes,
        tags: addForm.tags ? addForm.tags.split(",").map((s) => s.trim()).filter(Boolean) : [],
        projectPath: getProjectPath(),
      });
      loadTargets();
      emit("targets-changed").catch(() => {});
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
  }, [loadTargets]);

  const handleBatchAdd = useCallback(async (batchInput: string) => {
    if (!batchInput.trim()) return;
    try {
      const added = await invoke<Target[]>("target_batch_add", {
        values: batchInput,
        projectPath: getProjectPath(),
      });
      loadTargets();
      if (added.length > 0) {
        console.info(`Imported ${added.length} targets`);
      }
    } catch (e) {
      console.error("Failed to batch add:", e);
    }
  }, [loadTargets]);

  const handleDelete = useCallback(async (id: string) => {
    try {
      await invoke("target_delete", { id, projectPath: getProjectPath() });
      loadTargets();
      logAudit({ action: "target_deleted", category: "targets", details: id, entityType: "target", entityId: id });
    } catch (e) {
      console.error("Failed to delete target:", e);
    }
  }, [loadTargets]);

  const handleToggleScope = useCallback(async (target: Target) => {
    try {
      await invoke("target_update", {
        id: target.id,
        scope: target.scope === "in" ? "out" : "in",
        projectPath: getProjectPath(),
      });
      loadTargets();
    } catch (e) {
      console.error("Failed to update scope:", e);
    }
  }, [loadTargets]);

  const handleUpdateNotes = useCallback(async (id: string, notes: string) => {
    try {
      await invoke("target_update", { id, notes, projectPath: getProjectPath() });
      loadTargets();
    } catch (e) {
      console.error("Failed to update notes:", e);
    }
  }, [loadTargets]);

  const handleClearAll = useCallback(async (confirmMsg: string) => {
    if (!confirm(confirmMsg)) return;
    try {
      await invoke("target_clear_all", { projectPath: getProjectPath() });
      loadTargets();
    } catch (e) {
      console.error("Failed to clear:", e);
    }
  }, [loadTargets]);

  const safeTargets = store?.targets ?? [];

  const stats = useMemo(() => ({
    total: safeTargets.length,
    inScope: safeTargets.filter((t) => t.scope === "in").length,
    outOfScope: safeTargets.filter((t) => t.scope === "out").length,
  }), [safeTargets]);

  return {
    safeTargets,
    stats,
    handleAdd,
    handleBatchAdd,
    handleDelete,
    handleToggleScope,
    handleUpdateNotes,
    handleClearAll,
  };
}
