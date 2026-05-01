import { invoke } from "./client";

export async function saveTurn(cveId: string, sessionId: string, turn: unknown): Promise<void> {
  await invoke("kb_research_save_turn", { cveId, sessionId, turn });
}

export async function setStatus(cveId: string, status: string): Promise<void> {
  await invoke("kb_research_set_status", { cveId, status });
}

export async function clearResearch(cveId: string): Promise<void> {
  await invoke("kb_research_clear", { cveId });
}
