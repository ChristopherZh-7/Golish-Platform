import type {
  DbVulnLinkFull,
  PocTemplate,
  VulnEntry,
  VulnFeed,
} from "@/components/VulnIntelPanel/types";
import { invoke } from "@/lib/api/client";
import type { WikiBacklinkInfo, WikiPageInfo } from "@/lib/wiki";

// Shapes mirror the backend Rust structs (golish-vuln-intel):
//   github_poc.rs::GithubPocResult, nuclei_search.rs::NucleiTemplateResult,
//   nuclei_discover.rs::NucleiDiscoverResult.
export interface GithubPocResult {
  full_name: string;
  html_url: string;
  description: string | null;
  language: string | null;
  stars: number;
  updated_at: string;
  topics: string[];
}

export interface NucleiTemplateResult {
  name: string;
  path: string;
  html_url: string;
  content: string | null;
  severity: string | null;
}

export interface NucleiDiscoverResult {
  total_files: number;
  total_cves: number;
  imported: number;
  skipped: number;
  errors: number;
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
  addScan: (params: { cveId: string; target: string; result: string; details?: string }) =>
    invoke("vuln_link_add_scan", params),

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
  // Create a PoC from manual form input (no source metadata).
  addPoc: (params: {
    cveId: string;
    name: string;
    pocType: string;
    language: string;
    content: string;
  }) => invoke<PocTemplate>("vuln_link_add_poc", params),
  // Import a full PoC (e.g. a Nuclei template) as-is.
  addPocFull: (params: {
    cveId: string;
    name: string;
    pocType: string;
    language: string;
    content: string;
    source: string;
    sourceUrl: string;
    severity: string;
    description: string;
    tags: string[];
  }) => invoke<PocTemplate>("vuln_link_add_poc_full", params),

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
  wikiSearchDb: (query: string, limit: number) =>
    invoke<
      Array<{
        path: string;
        title: string;
        category: string;
        tags: string[];
        status: string | null;
      }>
    >("wiki_search_db", { query, limit }),
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
