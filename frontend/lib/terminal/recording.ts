import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";

export interface RecordingEvent {
  elapsed: number; // seconds since start
  data: string;
}

export interface RecordingMeta {
  id: string;
  title: string;
  session_id: string;
  width: number;
  height: number;
  duration_ms: number;
  event_count: number;
  created_at: string;
}

export interface Recording {
  meta: RecordingMeta;
  events: [number, string][];
}

let activeRecordings: Map<
  string,
  { startTime: number; events: RecordingEvent[]; title: string }
> = new Map();

export function startRecording(sessionId: string, title?: string) {
  activeRecordings.set(sessionId, {
    startTime: Date.now(),
    events: [],
    title: title || `Recording ${new Date().toLocaleString()}`,
  });
}

export function isRecording(sessionId: string): boolean {
  return activeRecordings.has(sessionId);
}

export function appendRecordingData(sessionId: string, data: string) {
  const rec = activeRecordings.get(sessionId);
  if (!rec) return;
  const elapsed = (Date.now() - rec.startTime) / 1000;
  rec.events.push({ elapsed, data });
}

export async function stopRecording(
  sessionId: string,
  width: number,
  height: number,
): Promise<string | null> {
  const rec = activeRecordings.get(sessionId);
  if (!rec) return null;
  activeRecordings.delete(sessionId);

  const durationMs = Date.now() - rec.startTime;
  const id = crypto.randomUUID().replace(/-/g, "").slice(0, 12);

  const recording: Recording = {
    meta: {
      id,
      title: rec.title,
      session_id: sessionId,
      width,
      height,
      duration_ms: durationMs,
      event_count: rec.events.length,
      created_at: new Date().toISOString(),
    },
    events: rec.events.map((e) => [e.elapsed, e.data]),
  };

  try {
    await invoke("recording_save", {
      recording,
      projectPath: getProjectPath(),
    });
    return id;
  } catch (e) {
    console.error("Failed to save recording:", e);
    return null;
  }
}

export async function listRecordings(): Promise<RecordingMeta[]> {
  try {
    return await invoke<RecordingMeta[]>("recording_list", {
      projectPath: getProjectPath(),
    });
  } catch {
    return [];
  }
}

export async function loadRecording(id: string): Promise<Recording | null> {
  try {
    return await invoke<Recording>("recording_load", {
      id,
      projectPath: getProjectPath(),
    });
  } catch {
    return null;
  }
}

export async function deleteRecording(id: string): Promise<boolean> {
  try {
    await invoke("recording_delete", { id, projectPath: getProjectPath() });
    return true;
  } catch {
    return false;
  }
}
