// Shared types and utilities for VulnIntelPanel
export interface VulnFeed {
  id: string;
  name: string;
  feed_type: string;
  url: string;
  enabled: boolean;
  last_fetched: number | null;
}

export interface VulnEntry {
  cve_id: string;
  title: string;
  description: string;
  severity: string;
  cvss_score: number | null;
  published: string;
  source: string;
  references: string[];
  affected_products: string[];
}

export interface PocTemplate {
  id: string;
  name: string;
  type: "nuclei" | "script" | "manual";
  language: string;
  content: string;
  source: string;
  source_url: string;
  severity: string;
  verified: boolean;
  description: string;
  tags: string[];
  created: number;
}

export interface VulnLink {
  wikiPaths: string[];
  pocTemplates: PocTemplate[];
  scanHistory: ScanHistoryEntry[];
}

export interface ScanHistoryEntry {
  target: string;
  date: number;
  result: "vulnerable" | "not_vulnerable" | "error" | "pending";
  details?: string;
}

export const SEV_COLORS: Record<string, string> = {
  critical: "text-red-400 bg-red-500/10 border-red-500/20",
  high: "text-orange-400 bg-orange-500/10 border-orange-500/20",
  medium: "text-yellow-400 bg-yellow-500/10 border-yellow-500/20",
  low: "text-blue-400 bg-blue-500/10 border-blue-500/20",
  info: "text-slate-400 bg-slate-500/10 border-slate-500/20",
};

export const SEV_DOT: Record<string, string> = {
  critical: "bg-red-500",
  high: "bg-orange-500",
  medium: "bg-yellow-500",
  low: "bg-blue-500",
  info: "bg-slate-500",
};

export interface DbVulnLinkFull {
  wiki_paths: string[];
  poc_templates: Array<{
    id: string; name: string; type: string; language: string; content: string;
    source: string; source_url: string; severity: string; verified: boolean;
    description: string; tags: string[]; created: number;
  }>;
  scan_history: Array<{ id: string; target: string; date: number; result: string; details?: string }>;
}

export function dbToVulnLink(db: DbVulnLinkFull): VulnLink {
  return {
    wikiPaths: db.wiki_paths,
    pocTemplates: db.poc_templates.map((p) => ({
      id: p.id,
      name: p.name,
      type: p.type as PocTemplate["type"],
      language: p.language,
      content: p.content,
      source: p.source ?? "manual",
      source_url: p.source_url ?? "",
      severity: p.severity ?? "unknown",
      verified: p.verified ?? false,
      description: p.description ?? "",
      tags: p.tags ?? [],
      created: p.created,
    })),
    scanHistory: db.scan_history.map((s) => ({
      target: s.target,
      date: s.date,
      result: s.result as ScanHistoryEntry["result"],
      details: s.details,
    })),
  };
}

export const EMPTY_LINK: VulnLink = { wikiPaths: [], pocTemplates: [], scanHistory: [] };

export function getOrCreateLink(links: Record<string, VulnLink>, cveId: string): VulnLink {
  return links[cveId] || EMPTY_LINK;
}

export type ViewMode = "feed" | "matched" | "feeds-config";
export type DetailTab = "intel" | "wiki" | "poc" | "history" | "research";
export type FilterMode = "all" | "has-poc" | "has-wiki" | "no-poc";
export type SeverityFilter = "all" | "critical" | "high" | "medium" | "low" | "info";
export type SourceFilter = "all" | "cve" | "cnvd" | "other";
export type TopTab = "intel" | "wiki" | "poc-library";

import { lazy } from "react";
export const WikiPanelEmbed = lazy(() =>
  import("@/components/WikiPanel/WikiPanel").then((m) => ({ default: m.WikiPanel }))
);
