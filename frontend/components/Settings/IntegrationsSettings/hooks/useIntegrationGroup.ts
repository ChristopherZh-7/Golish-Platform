/**
 * Data hook for one (toolId, groupId) credential form.
 *
 * Owns the local edit buffer + server snapshot + busy flags. Components
 * stay thin — they just render hook output + dispatch hook actions.
 *
 * State machine summary:
 *
 *   mount   → status "loading", call integrations_get
 *           → success: status "ready"   (snapshot populated)
 *           → failure: status "error"   (error message captured)
 *
 *   user typing   → just updates the local `values` map
 *
 *   save          → status "saving"   (calls integrations_set)
 *                 → on success, re-fetch snapshot so `displayHint` /
 *                   `has_value` refresh; clears edit buffer for secrets
 *                 → on failure, status "ready" with `lastError`
 *
 *   clear         → status "clearing"  (calls integrations_clear)
 *                 → on success, snapshot becomes empty; edit buffer
 *                   reset to ""
 *
 *   test          → status "testing"   (calls integrations_test)
 *                 → result stored in `lastHealth`
 */

import { useCallback, useEffect, useState } from "react";
import { integrations } from "@/lib/api";
import type { Field, FieldValue, IntegrationHealth } from "@/lib/api/integrations";

export type GroupStatus = "loading" | "ready" | "saving" | "clearing" | "testing" | "error";

export interface UseIntegrationGroupOptions {
  toolId: string;
  groupId: string;
  fields: Field[];
}

export interface UseIntegrationGroupResult {
  /** Current high-level state machine state. */
  status: GroupStatus;
  /** Server's view of each field (what `has_value`, when `updated_at`). */
  snapshot: Record<string, FieldValue>;
  /** Local edit buffer; what the user typed but hasn't saved yet. */
  values: Record<string, string>;
  /** Latest test result (cleared on edit). */
  lastHealth: IntegrationHealth | null;
  /** Latest error message (cleared on next successful op). */
  lastError: string | null;
  /** True iff any field's local value differs from default ("" or
   *  the non-secret snapshot value). */
  dirty: boolean;
  /** Whether the form has any required field still blank. */
  invalid: boolean;
  setField: (key: string, next: string) => void;
  save: () => Promise<void>;
  clear: () => Promise<void>;
  test: () => Promise<void>;
  /** Force-reload the server snapshot (after parent edits). */
  reload: () => Promise<void>;
}

function defaultsFromSnapshot(
  fields: Field[],
  snapshot: Record<string, FieldValue>
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of fields) {
    const sv = snapshot[f.key];
    if (sv?.value != null && (f.type === "text" || f.type === "url" || f.type === "boolean")) {
      out[f.key] = sv.value;
    } else {
      out[f.key] = "";
    }
  }
  return out;
}

export function useIntegrationGroup({
  toolId,
  groupId,
  fields,
}: UseIntegrationGroupOptions): UseIntegrationGroupResult {
  const [status, setStatus] = useState<GroupStatus>("loading");
  const [snapshot, setSnapshot] = useState<Record<string, FieldValue>>({});
  const [values, setValues] = useState<Record<string, string>>({});
  const [lastHealth, setLastHealth] = useState<IntegrationHealth | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  const load = useCallback(async () => {
    setStatus("loading");
    setLastError(null);
    try {
      const snap = await integrations.get({ toolId, groupId });
      setSnapshot(snap);
      setValues(defaultsFromSnapshot(fields, snap));
      setDirty(false);
      setStatus("ready");
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
      setStatus("error");
    }
  }, [toolId, groupId, fields]);

  useEffect(() => {
    void load();
  }, [load]);

  const setField = useCallback((key: string, next: string) => {
    setValues((prev) => {
      const updated = { ...prev, [key]: next };
      return updated;
    });
    setDirty(true);
    setLastHealth(null);
  }, []);

  const save = useCallback(async () => {
    setStatus("saving");
    setLastError(null);
    try {
      // Only send fields the user actually filled in. Empty secret
      // fields are skipped so existing vault rows survive a "save
      // metadata only" gesture.
      const toSend: Record<string, string> = {};
      for (const f of fields) {
        const v = values[f.key] ?? "";
        const isSecret = f.type === "secret_text" || f.type === "secret_textarea";
        if (v !== "" || f.required) {
          // Non-empty or required (server will reject blank required).
          toSend[f.key] = v;
        } else if (!isSecret && v === "") {
          // Non-secret optional field cleared by user: send empty to
          // overwrite stored value.
          toSend[f.key] = "";
        }
      }
      await integrations.set({ toolId, groupId, fields: toSend });
      await load();
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
      setStatus("ready");
    }
  }, [toolId, groupId, fields, values, load]);

  const clear = useCallback(async () => {
    setStatus("clearing");
    setLastError(null);
    try {
      await integrations.clear({ toolId, groupId });
      await load();
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
      setStatus("ready");
    }
  }, [toolId, groupId, load]);

  const test = useCallback(async () => {
    setStatus("testing");
    setLastError(null);
    try {
      const h = await integrations.test({ toolId, groupId });
      setLastHealth(h);
      setStatus("ready");
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
      setStatus("ready");
    }
  }, [toolId, groupId]);

  const invalid = fields.some((f) => {
    if (!f.required) return false;
    const v = values[f.key] ?? "";
    // For secrets, "" is OK iff the server already has a value (we
    // treat that as "keep existing"). For non-secret required fields
    // (e.g. text / url), blank is invalid.
    const isSecret = f.type === "secret_text" || f.type === "secret_textarea";
    if (isSecret) {
      return v.trim() === "" && !snapshot[f.key]?.has_value;
    }
    return v.trim() === "";
  });

  return {
    status,
    snapshot,
    values,
    lastHealth,
    lastError,
    dirty,
    invalid,
    setField,
    save,
    clear,
    test,
    reload: load,
  };
}
