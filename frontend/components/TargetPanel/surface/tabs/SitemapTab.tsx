import { Network } from "lucide-react";
import { EmptyPanel, Section } from "../SurfaceParts";
import type { SitemapItem } from "../types";

export function SitemapTab({ items, loading }: { items: SitemapItem[]; loading: boolean }) {
  if (items.length === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<Network className="w-5 h-5" />}
        title="No sitemap data yet"
        body="Run baseline recon or a crawler-backed collection step to populate paths, robots, and sitemap entries."
      />
    );
  }

  return (
    <Section title="Sitemap / Paths" subtitle={`${items.length} discovered`}>
      <div className="space-y-1">
        {items.slice(0, 80).map((item) => (
          <div
            key={`${item.source}:${item.url}`}
            className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
          >
            <div className="flex items-center gap-2 text-[11px]">
              <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                {item.source}
              </span>
              {item.statusCode != null && (
                <span className="font-mono text-[10px] text-green-300">{item.statusCode}</span>
              )}
              <span className="min-w-0 flex-1 truncate font-mono text-foreground/80">
                {item.url}
              </span>
              {item.contentType && (
                <span className="text-[9px] text-muted-foreground">{item.contentType}</span>
              )}
            </div>
          </div>
        ))}
      </div>
    </Section>
  );
}
