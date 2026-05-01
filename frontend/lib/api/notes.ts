import { invoke } from "./client";

export interface Note {
  id: string;
  entity_type: string;
  entity_id: string;
  content: string;
  color: string;
  created_at: number;
  updated_at: number;
}

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
