import { invoke } from "./client";

export interface WordlistMeta {
  id: string;
  name: string;
  category: string;
  description: string;
  line_count: number;
  file_size: number;
  filename: string;
  tags: string[];
  created_at: number;
}

export async function listWordlists(): Promise<WordlistMeta[]> {
  const list = await invoke<WordlistMeta[]>("wordlist_list");
  return Array.isArray(list) ? list : [];
}

export async function importWordlist(params: {
  name: string;
  category: string;
  description: string;
  contentBase64: string;
  originalFilename: string;
}): Promise<void> {
  await invoke("wordlist_import", params);
}

export async function deleteWordlist(id: string): Promise<void> {
  await invoke("wordlist_delete", { id });
}

export async function deduplicateWordlist(id: string): Promise<WordlistMeta> {
  return invoke<WordlistMeta>("wordlist_deduplicate", { id });
}

export async function previewWordlist(id: string, lines = 30): Promise<string[]> {
  return invoke<string[]>("wordlist_preview", { id, lines });
}

export async function getWordlistPath(id: string): Promise<string> {
  return invoke<string>("wordlist_path", { id });
}

export async function mergeWordlists(params: {
  ids: string[];
  newName: string;
  deduplicate: boolean;
}): Promise<void> {
  await invoke("wordlist_merge", params);
}
