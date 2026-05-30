import type { Note } from "@/lib/generated/Note";
import { invoke } from "./client";

// `Note` is generated from the Rust IPC DTO via ts-rs (golish/src/tools/notes.rs).
// Re-exported so existing `@/lib/api/notes` consumers keep their import path.
export type { Note };

export async function listNotes(params: {
  entityType: string;
  entityId: string;
  projectPath: string | null;
}): Promise<Note[]> {
  return invoke<Note[]>("notes_list", params);
}

export async function addNote(params: {
  entityType: string;
  entityId: string;
  content: string;
  color: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("notes_add", params);
}

export async function updateNote(params: {
  id: string;
  content: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("notes_update", params);
}

export async function deleteNote(params: {
  id: string;
  projectPath: string | null;
}): Promise<void> {
  await invoke("notes_delete", params);
}
