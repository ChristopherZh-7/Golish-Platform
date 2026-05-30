export type SurfaceTab = "identity" | "surface" | "sitemap" | "js-api" | "sensitive" | "evidence";

export const SURFACE_TABS: Array<{ id: SurfaceTab; label: string }> = [
  { id: "identity", label: "Identity" },
  { id: "surface", label: "Surface" },
  { id: "sitemap", label: "Sitemap" },
  { id: "js-api", label: "JS / API" },
  { id: "sensitive", label: "Sensitive" },
  { id: "evidence", label: "Evidence" },
];

export interface SitemapItem {
  url: string;
  source: string;
  statusCode: number | null;
  contentType: string;
}

export interface SensitiveFinding {
  source: string;
  label: string;
  url: string;
  count: number;
}
