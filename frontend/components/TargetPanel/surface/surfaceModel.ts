import type { DirectoryEntry } from "@/lib/pentest/api";
import type { PortInfo } from "@/lib/pentest/types";
import type { JsAnalysisResult, PassiveScanLog, TargetAsset } from "@/lib/security-analysis";
import { formatClockTime as formatTime } from "@/lib/time";
import type { SensitiveFinding, SitemapItem } from "./types";

export function isHttpPort(port: PortInfo): boolean {
  const service = port.service?.toLowerCase() ?? "";
  return service.includes("http") || port.http_status != null || Boolean(port.http_title);
}

export function buildSitemapItems(
  assets: TargetAsset[],
  directoryEntries: DirectoryEntry[]
): SitemapItem[] {
  const seen = new Set<string>();
  const out: SitemapItem[] = [];
  for (const entry of directoryEntries) {
    if (!entry.url || seen.has(entry.url)) continue;
    seen.add(entry.url);
    out.push({
      url: entry.url,
      source: entry.tool || "directory",
      statusCode: entry.status_code,
      contentType: entry.content_type,
    });
  }
  for (const asset of assets) {
    if (!asset.value || seen.has(asset.value)) continue;
    const type = asset.assetType.toLowerCase();
    if (!type.includes("path") && !type.includes("url") && !type.includes("sitemap")) continue;
    seen.add(asset.value);
    out.push({
      url: asset.value,
      source: asset.assetType,
      statusCode: null,
      contentType: String(asset.metadata?.content_type ?? ""),
    });
  }
  return out;
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
