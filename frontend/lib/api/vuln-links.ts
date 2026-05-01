import { invoke } from "./client";

export async function updatePoc(pocId: string, name: string, content: string): Promise<void> {
  await invoke("vuln_link_update_poc", { pocId, name, content });
}

export async function removePoc(pocId: string): Promise<void> {
  await invoke("vuln_link_remove_poc", { pocId });
}

export async function addWikiLink(cveId: string, wikiPath: string): Promise<void> {
  await invoke("vuln_link_add_wiki", { cveId, wikiPath });
}

export async function removeWikiLink(cveId: string, wikiPath: string): Promise<void> {
  await invoke("vuln_link_remove_wiki", { cveId, wikiPath });
}

export async function addScan(params: { cveId: string; target: string; [key: string]: unknown }): Promise<void> {
  await invoke("vuln_link_add_scan", params);
}
