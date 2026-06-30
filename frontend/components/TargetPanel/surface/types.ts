export type SurfaceTab = "identity" | "surface" | "sitemap" | "sensitive" | "evidence";

export const SURFACE_TABS: Array<{ id: SurfaceTab; label: string }> = [
  { id: "identity", label: "Identity" },
  { id: "surface", label: "Surface" },
  { id: "sitemap", label: "Sitemap" },
  { id: "sensitive", label: "Sensitive" },
  { id: "evidence", label: "Evidence" },
];

export interface SitemapItem {
  id: string;
  url: string;
  method: string;
  path: string;
  source: string;
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
