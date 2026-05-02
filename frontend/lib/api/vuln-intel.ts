import type {
  DbVulnLinkFull,
  PocTemplate,
  VulnEntry,
  VulnFeed,
} from "@/components/VulnIntelPanel/types";
import { invoke } from "@/lib/api/client";

export interface GithubPocResult {
  name: string;
  url: string;
  description: string;
  language: string;
  stars: number;
  updated: string;
}

export interface NucleiTemplateResult {
  id: string;
  name: string;
  severity: string;
  author: string;
  tags: string[];
  description: string;
  template_content: string;
}

export interface NucleiDiscoverResult {
  total: number;
  templates: Array<{
    cve_id: string;
    template_id: string;
    name: string;
    severity: string;
    content: string;
  }>;
}

export interface WikiPageInfo {
  path: string;
  title: string;
  category: string;
  tags: string[];
  status: string | null;
  word_count?: number;
  updated_at?: string;
}

export interface WikiBacklinkInfo {
  source_path: string;
  context: string;
}

export const vulnIntelApi = {
  // Intel data
  getCached: () => invoke<VulnEntry[]>("intel_get_cached"),
  fetch: () => invoke<VulnEntry[]>("intel_fetch"),
  fetchPage: (page: number) => invoke<VulnEntry[]>("intel_fetch_page", { page }),
  search: (query: string) => invoke<VulnEntry[]>("intel_search", { query }),
  searchRemote: (query: string) => invoke<VulnEntry[]>("intel_search_remote", { query }),
  searchRemotePage: (query: string, startIndex: number) =>
    invoke<VulnEntry[]>("intel_search_remote_page", { query, startIndex }),
  matchTargets: (projectPath: string) =>
    invoke<VulnEntry[]>("intel_match_targets", { projectPath }),

  // Feeds
  listFeeds: () => invoke<VulnFeed[]>("intel_list_feeds"),
  addFeed: (name: string, feedType: string, url: string) =>
    invoke("intel_add_feed", { name, feedType, url }),
  toggleFeed: (id: string, enabled: boolean) => invoke("intel_toggle_feed", { id, enabled }),
  deleteFeed: (id: string) => invoke("intel_delete_feed", { id }),

  // Vuln links
  getAllLinks: () => invoke<Record<string, DbVulnLinkFull>>("vuln_link_get_all"),
  getLink: (cveId: string) => invoke<DbVulnLinkFull>("vuln_link_get", { cveId }),
  addWiki: (cveId: string, wikiPath: string) => invoke("vuln_link_add_wiki", { cveId, wikiPath }),
  removeWiki: (cveId: string, wikiPath: string) =>
    invoke("vuln_link_remove_wiki", { cveId, wikiPath }),
  removePoc: (pocId: string) => invoke("vuln_link_remove_poc", { pocId }),
  updatePoc: (pocId: string, name: string, content: string) =>
    invoke("vuln_link_update_poc", { pocId, name, content }),
  addScan: (cveId: string, target: string, pocId: string, result: string, details?: string) =>
    invoke("vuln_link_add_scan", { cveId, target, pocId, result, details }),

  // PoC search
  searchGithubPoc: (cveId: string) =>
    invoke<GithubPocResult[]>("intel_search_github_poc", { cveId }),
  searchNucleiTemplates: (cveId: string) =>
    invoke<NucleiTemplateResult[]>("intel_search_nuclei_templates", { cveId }),
  addPocFromSource: (
    cveId: string,
    name: string,
    type: string,
    language: string,
    content: string,
    source: string,
    sourceUrl: string,
    severity: string,
    description: string,
    tags: string[]
  ) =>
    invoke<PocTemplate>("vuln_link_add_poc", {
      cveId,
      name,
      pocType: type,
      language,
      content,
      source,
      sourceUrl,
      severity,
      description,
      tags,
    }),
  discoverAllNuclei: () => invoke<NucleiDiscoverResult>("intel_discover_all_nuclei"),

  // KB Research
  researchLoad: (cveId: string) =>
    invoke<{ turns: Array<Record<string, unknown>>; status: string } | null>("kb_research_load", {
      cveId,
    }),
  researchSaveTurn: (cveId: string, sessionId: string, turn: unknown) =>
    invoke("kb_research_save_turn", { cveId, sessionId, turn }),
  researchSetStatus: (cveId: string, status: string) =>
    invoke("kb_research_set_status", { cveId, status }),
  researchClear: (cveId: string) => invoke("kb_research_clear", { cveId }),

  // Wiki page info
  wikiPagesForPaths: (paths: string[]) => invoke<WikiPageInfo[]>("wiki_pages_for_paths", { paths }),
  wikiSuggestForCve: (cveId: string, limit: number) =>
    invoke<WikiPageInfo[]>("wiki_suggest_for_cve", { cveId, limit }),
  wikiBacklinks: (path: string) => invoke<WikiBacklinkInfo[]>("wiki_backlinks", { path }),
  wikiSearch: (query: string) =>
    invoke<
      Array<{
        path: string;
        title: string;
        category: string;
        tags: string[];
        status: string | null;
      }>
    >("wiki_search_pages", { query }),
};
