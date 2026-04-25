import { invoke } from "@tauri-apps/api/core";

export interface WikiEntry {
  path: string;
  name: string;
  is_dir: boolean;
  children?: WikiEntry[];
  size?: number;
  modified?: number;
}

export interface WikiSearchResult {
  path: string;
  name: string;
  line: number;
  content: string;
}

export interface WikiStats {
  total_pages: number;
  total_words: number;
  orphan_count: number;
  by_category: Record<string, number>;
  by_status: Record<string, number>;
  recent_changes: Array<{
    id: number;
    page_path: string;
    action: string;
    title: string;
    category: string;
    actor: string;
    summary: string;
    created_at: string;
  }>;
}

export interface WikiPageInfo {
  path: string;
  title: string;
  category: string;
  tags: string[];
  status: string;
  word_count: number;
  updated_at?: string;
}

export interface WikiBacklinkInfo {
  source_path: string;
  context: string;
}

export interface WikiTreeNode {
  path: string;
  name: string;
  is_dir: boolean;
  children?: WikiTreeNode[];
}

export const wikiApi = {
  list: () => invoke<WikiEntry[]>("wiki_list"),
  read: (path: string) => invoke<string>("wiki_read", { path }),
  write: (path: string, content: string) => invoke<void>("wiki_write", { path, content }),
  delete: (path: string) => invoke<void>("wiki_delete", { path }),
  createDir: (path: string) => invoke<void>("wiki_create_dir", { path }),
  search: (query: string) => invoke<WikiSearchResult[]>("wiki_search", { query }),
  reindex: () => invoke<void>("wiki_reindex"),
  statsFull: () => invoke<WikiStats>("wiki_stats_full"),
};
