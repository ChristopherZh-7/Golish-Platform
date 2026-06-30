import type { PortInfo } from "@/lib/pentest/types";
import type { ApiEndpoint, JsAnalysisResult, PassiveScanLog } from "@/lib/security-analysis";
import { formatClockTime as formatTime } from "@/lib/time";
import type { SensitiveFinding, SitemapItem, SitemapJsSource, SitemapTreeNode } from "./types";

export function isHttpPort(port: PortInfo): boolean {
  const service = port.service?.toLowerCase() ?? "";
  return service.includes("http") || port.http_status != null || Boolean(port.http_title);
}

function isDeterministicJsEndpointSource(source: string): boolean {
  const normalized = source.toLowerCase();
  return normalized === "crawler" || normalized === "js_analysis" || normalized.includes("js");
}

export function buildSitemapItems(endpoints: ApiEndpoint[] = []): SitemapItem[] {
  const seen = new Set<string>();
  const out: SitemapItem[] = [];
  for (const endpoint of endpoints) {
    const url = endpoint.url || endpoint.path;
    if (!url || !isDeterministicJsEndpointSource(endpoint.source)) continue;
    const key = `${endpoint.method}:${url}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      id: endpoint.id,
      url,
      method: endpoint.method || "GET",
      path: endpoint.path || url,
      source: endpoint.source || "api_endpoint",
      params: endpoint.params,
      headers: endpoint.headers,
      statusCode: endpoint.statusCode,
      contentType: endpoint.responseType ?? String(endpoint.headers["content-type"] ?? ""),
      capturePath: endpoint.capturePath,
      discoveredAt: endpoint.discoveredAt,
    });
  }
  return out;
}

interface MutableSitemapTreeNode extends SitemapTreeNode {
  childMap: Map<string, MutableSitemapTreeNode>;
}

function createSitemapTreeNode(
  id: string,
  label: string,
  url: string | null
): MutableSitemapTreeNode {
  return {
    id,
    label,
    url,
    items: [],
    children: [],
    childMap: new Map(),
    itemCount: 0,
  };
}

function sitemapPathParts(rawUrl: string): { root: string; parts: string[] } {
  const trimmed = rawUrl.trim();
  if (!trimmed) return { root: "unknown", parts: [] };

  if (/^https?:\/\//i.test(trimmed)) {
    try {
      const parsed = new URL(trimmed);
      const parts = parsed.pathname.split("/").filter(Boolean);
      if (parsed.search) parts.push(parsed.search);
      return { root: parsed.origin, parts };
    } catch {
      return { root: trimmed, parts: [] };
    }
  }

  if (trimmed.startsWith("/")) {
    const [pathPart, queryPart] = trimmed.split("?", 2);
    const parts = pathPart.split("/").filter(Boolean);
    if (queryPart) parts.push(`?${queryPart}`);
    return { root: "relative paths", parts };
  }

  const [hostPart, ...pathParts] = trimmed.split("/");
  const parts = pathParts.flatMap((part, index) => {
    if (index !== pathParts.length - 1 || !part.includes("?")) return part ? [part] : [];
    const [pathPart, queryPart] = part.split("?", 2);
    return [pathPart, queryPart ? `?${queryPart}` : ""].filter(Boolean);
  });
  return { root: hostPart || trimmed, parts };
}

function stripMutableFields(node: MutableSitemapTreeNode): SitemapTreeNode {
  const children = [...node.childMap.values()]
    .sort((a, b) => a.label.localeCompare(b.label))
    .map(stripMutableFields);
  const itemCount = node.items.length + children.reduce((sum, child) => sum + child.itemCount, 0);
  return {
    id: node.id,
    label: node.label,
    url: node.url,
    items: node.items,
    children,
    itemCount,
  };
}

export function buildSitemapTree(items: SitemapItem[]): SitemapTreeNode[] {
  const roots = new Map<string, MutableSitemapTreeNode>();

  for (const item of items) {
    const { root, parts } = sitemapPathParts(item.url);
    const rootId = `root:${root}`;
    let node = roots.get(rootId);
    if (!node) {
      node = createSitemapTreeNode(rootId, root, root);
      roots.set(rootId, node);
    }

    for (const part of parts) {
      const id = `${node.id}/${part}`;
      let child = node.childMap.get(part);
      if (!child) {
        child = createSitemapTreeNode(id, part, null);
        node.childMap.set(part, child);
      }
      node = child;
    }

    node.items.push(item);
    if (!node.url) node.url = item.url;
  }

  return [...roots.values()].sort((a, b) => a.label.localeCompare(b.label)).map(stripMutableFields);
}

type JsonRecord = Record<string, unknown>;

function isRecord(value: unknown): value is JsonRecord {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function stringValue(record: JsonRecord, ...keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return "";
}

function numberValue(record: JsonRecord, ...keys: string[]): number | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "string" && value.trim()) {
      const parsed = Number(value);
      if (Number.isFinite(parsed)) return parsed;
    }
  }
  return null;
}

function normalizeComparablePath(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";

  let path = trimmed;
  if (/^https?:\/\//i.test(trimmed)) {
    try {
      path = new URL(trimmed).pathname;
    } catch {
      path = trimmed;
    }
  }

  path = path
    .split("#", 1)[0]
    .split("?", 1)[0]
    .replace(/\/{2,}/g, "/");
  if (!path) return "/";
  if (!path.startsWith("/")) path = `/${path}`;
  return path.length > 1 && path.endsWith("/") ? path.slice(0, -1) : path;
}

function jsEndpointSource(
  candidate: unknown,
  result: JsAnalysisResult,
  index: number
): SitemapJsSource | null {
  if (typeof candidate === "string" && candidate.trim()) {
    return {
      id: `${result.id}:${index}`,
      filename: result.filename,
      url: result.url,
      sourceFile: result.filename,
      method: "",
      path: candidate.trim(),
      line: null,
      confidence: null,
      kind: "",
    };
  }
  if (!isRecord(candidate)) return null;

  const path = stringValue(candidate, "path", "url", "endpoint");
  if (!path) return null;
  return {
    id: `${result.id}:${index}`,
    filename: result.filename,
    url: result.url,
    sourceFile: stringValue(candidate, "sourceFile", "source_file") || result.filename,
    method: stringValue(candidate, "method").toUpperCase(),
    path,
    line: numberValue(candidate, "line"),
    confidence: numberValue(candidate, "confidence"),
    kind: stringValue(candidate, "kind"),
  };
}

export function buildSitemapJsSources(
  item: SitemapItem,
  jsResults: JsAnalysisResult[] = []
): SitemapJsSource[] {
  const itemPath = normalizeComparablePath(item.path || item.url);
  const itemMethod = (item.method || "GET").toUpperCase();
  const seen = new Set<string>();
  const sources: SitemapJsSource[] = [];

  for (const result of jsResults) {
    for (const [index, candidate] of result.endpointsFound.entries()) {
      const source = jsEndpointSource(candidate, result, index);
      if (!source) continue;
      if (normalizeComparablePath(source.path) !== itemPath) continue;
      if (source.method && source.method !== itemMethod) continue;

      const key = `${source.sourceFile}:${source.line ?? ""}:${source.method}:${source.path}`;
      if (seen.has(key)) continue;
      seen.add(key);
      sources.push(source);
    }
  }

  return sources.slice(0, 12);
}

export function buildSensitiveFindings(
  jsResults: JsAnalysisResult[],
  passiveScans: PassiveScanLog[]
): SensitiveFinding[] {
  const findings: SensitiveFinding[] = [];
  for (const result of jsResults) {
    const secrets = Array.isArray(result.secretsFound) ? result.secretsFound : [];
    if (secrets.length > 0) {
      findings.push({
        source: "js",
        label: result.filename || result.url,
        url: result.url,
        count: secrets.length,
      });
    }
    if (result.sourceMaps) {
      findings.push({
        source: "sourcemap",
        label: result.filename || result.url,
        url: result.url,
        count: 1,
      });
    }
  }
  for (const scan of passiveScans) {
    if (!["vulnerable", "potential"].includes(scan.result)) continue;
    findings.push({
      source: scan.toolUsed || "passive",
      label: scan.testType || scan.evidence || scan.result,
      url: scan.url,
      count: 1,
    });
  }
  return findings;
}

export function formatLatestEvidence(
  timelineCreatedAt?: string,
  logCreatedAt?: number
): string | null {
  if (timelineCreatedAt) return formatTime(timelineCreatedAt);
  if (logCreatedAt) return formatTime(logCreatedAt);
  return null;
}

// `formatTime` (clock time-of-day HH:MM:SS) is the canonical helper in
// `@/lib/time` (`formatClockTime`); imported above and re-exported here so the
// existing `surfaceModel` import sites stay stable.
export { formatTime };
