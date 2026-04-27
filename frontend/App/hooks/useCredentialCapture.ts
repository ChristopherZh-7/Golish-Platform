import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { getProjectPath } from "@/lib/projects";
import { notify } from "@/lib/notify";

interface DetectedCredential {
  source_url: string;
  host: string;
  cred_type: string;
  username: string | null;
  value: string;
  field_name: string;
  zap_message_id: number;
}

interface VaultEntry {
  id: string;
  name: string;
  tags: string[];
}

const CRED_TYPE_MAP: Record<string, string> = {
  password: "password",
  bearer_token: "token",
  basic_auth: "password",
  api_key: "api_key",
  access_token: "token",
  refresh_token: "token",
  session_cookie: "cookie",
  o_auth_client_secret: "api_key",
};

function hashValue(v: string): string {
  let h = 0;
  for (let i = 0; i < v.length; i++) h = ((h << 5) - h + v.charCodeAt(i)) | 0;
  return h.toString(36);
}

/**
 * Auto-captures credentials detected by ZAP proxy and saves/updates vault entries.
 */
export function useCredentialCapture() {
  useEffect(() => {
    const knownEntries = new Map<string, { id: string; valueHash: string }>();
    let unlisten: (() => void) | null = null;

    (async () => {
      try {
        const existing = await invoke<VaultEntry[]>("vault_list", {
          projectPath: getProjectPath(),
        });
        if (Array.isArray(existing)) {
          for (const e of existing) {
            if (e.tags?.includes("auto-captured")) {
              knownEntries.set(e.name, { id: e.id, valueHash: "" });
            }
          }
        }
      } catch {
        /* vault might not be ready yet */
      }

      unlisten = await listen<DetectedCredential>("credential-detected", async (event) => {
        const cred = event.payload;
        const name = `${cred.host} - ${cred.field_name}`;
        const newHash = hashValue(cred.value);
        const existing = knownEntries.get(name);

        if (existing && existing.valueHash === newHash) return;

        const entryType = CRED_TYPE_MAP[cred.cred_type] || "other";

        try {
          if (existing) {
            await invoke("vault_update", {
              id: existing.id,
              value: cred.value,
              username: cred.username || null,
              notes: `Auto-captured from ${cred.source_url} (updated)`,
              projectPath: getProjectPath(),
            });
            knownEntries.set(name, { id: existing.id, valueHash: newHash });
            notify.info("Credential updated", {
              message: `${cred.host} — ${cred.field_name}`,
            });
          } else {
            const added = await invoke<{ id: string }>("vault_add", {
              name,
              entryType,
              value: cred.value,
              username: cred.username || null,
              notes: `Auto-captured from ${cred.source_url}`,
              project: cred.host,
              tags: ["auto-captured", "zap"],
              sourceUrl: cred.source_url,
              projectPath: getProjectPath(),
            });
            knownEntries.set(name, { id: added.id, valueHash: newHash });
            notify.success("Credential captured", {
              message: `${cred.host} — ${cred.field_name} (${cred.cred_type})`,
            });
          }
        } catch (e) {
          console.error("[CredAutoCapture] Failed to save credential:", e);
        }
      });
    })();

    return () => {
      unlisten?.();
    };
  }, []);
}
