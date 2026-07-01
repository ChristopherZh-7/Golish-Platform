export type SurfaceTab = "identity" | "surface" | "sitemap" | "sensitive" | "evidence";

export const SURFACE_TABS: Array<{ id: SurfaceTab; label: string }> = [
  { id: "identity", label: "Identity" },
  { id: "surface", label: "Surface" },
  { id: "sitemap", label: "Sitemap" },
  { id: "sensitive", label: "Sensitive" },
  { id: "evidence", label: "Evidence" },
];

export type SitemapItemKind = "endpoint" | "script" | "directory";

export interface SitemapItem {
  id: string;
  url: string;
  method: string;
  path: string;
  source: string;
  // Burp-style sitemap: endpoints (API/XHR paths) and scripts (.js assets)
  // live in the same host→path tree, distinguished by `kind`.
  kind: SitemapItemKind;
  // Byte size for `script` items (from js_analysis_results.size_bytes); null
  // for endpoints or when the size was not recorded.
  sizeBytes: number | null;
  params: unknown[];
  headers: Record<string, unknown>;
  statusCode: number | null;
  contentType: string;
  capturePath: string | null;
  discoveredAt: string;
}

export interface SitemapJsSource {
  id: string;
  filename: string;
  url: string;
  sourceFile: string;
  method: string;
  path: string;
  line: number | null;
  confidence: number | null;
  kind: string;
}

export interface SitemapTreeNode {
  id: string;
  label: string;
  url: string | null;
  items: SitemapItem[];
  children: SitemapTreeNode[];
  itemCount: number;
}

export interface SensitiveFinding {
  source: string;
  label: string;
  url: string;
  count: number;
}
