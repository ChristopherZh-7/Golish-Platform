import { ChevronDown, FileCode2, FileText, Network } from "lucide-react";
import { useMemo, useState } from "react";
import type { JsAnalysisResult } from "@/lib/security-analysis";
import { cn } from "@/lib/utils";
import { EndpointParamChips } from "../EndpointParamChips";
import { EmptyPanel, Section } from "../SurfaceParts";
import { buildSitemapJsSources } from "../surfaceModel";
import type { SitemapItem, SitemapJsSource, SitemapTreeNode } from "../types";

function flattenTreeItems(nodes: SitemapTreeNode[]): SitemapItem[] {
  return nodes.flatMap((node) => [...node.items, ...flattenTreeItems(node.children)]);
}

function SitemapNode({
  node,
  depth,
  selectedId,
  onSelect,
}: {
  node: SitemapTreeNode;
  depth: number;
  selectedId: string | null;
  onSelect: (item: SitemapItem) => void;
}) {
  const item = node.items[0];
  const hasChildren = node.children.length > 0;
  const selected = node.items.some((candidate) => candidate.id === selectedId);
  const paddingLeft = Math.min(depth * 14, 70);

  return (
    <div>
      <div
        className={cn(
          "flex min-h-7 items-center gap-1.5 border-b border-border/10 px-2 py-1 text-[11px] last:border-b-0",
          item && "cursor-pointer hover:bg-muted/20",
          selected && "bg-accent/10 text-accent"
        )}
        style={{ paddingLeft: `${paddingLeft + 8}px` }}
        title={node.url ?? node.label}
        onClick={() => {
          if (item) onSelect(item);
        }}
        onKeyDown={(event) => {
          if (!item || (event.key !== "Enter" && event.key !== " ")) return;
          event.preventDefault();
          onSelect(item);
        }}
        role={item ? "button" : undefined}
        tabIndex={item ? 0 : undefined}
      >
        <span className="flex h-4 w-4 flex-shrink-0 items-center justify-center text-muted-foreground">
          {hasChildren ? <ChevronDown className="h-3 w-3" /> : <FileText className="h-3 w-3" />}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-foreground/80">{node.label}</span>
        {node.itemCount > 1 && (
          <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] tabular-nums text-muted-foreground">
            {node.itemCount}
          </span>
        )}
        {item && (
          <>
            <span className="w-10 text-right font-mono text-[10px] text-blue-300">
              {item.method}
            </span>
            <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
              {item.source}
            </span>
            {item.statusCode != null && (
              <span className="font-mono text-[10px] text-green-300">{item.statusCode}</span>
            )}
            {item.contentType && (
              <span className="max-w-28 truncate text-[9px] text-muted-foreground">
                {item.contentType}
              </span>
            )}
          </>
        )}
      </div>
      {node.children.map((child) => (
        <SitemapNode
          key={child.id}
          node={child}
          depth={depth + 1}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}

function JsSourceList({ sources }: { sources: SitemapJsSource[] }) {
  return (
    <div className="rounded bg-background/25 px-2 py-1.5">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[9px] uppercase text-muted-foreground">JavaScript Source</p>
        {sources.length > 0 && (
          <span className="text-[9px] tabular-nums text-muted-foreground">{sources.length}</span>
        )}
      </div>
      {sources.length > 0 ? (
        <div className="mt-1.5 max-h-40 space-y-1 overflow-auto">
          {sources.map((source) => (
            <div
              key={source.id}
              className="rounded border border-border/10 bg-muted/10 px-2 py-1.5"
              title={source.url}
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <FileCode2 className="h-3 w-3 flex-shrink-0 text-accent" />
                <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-foreground/85">
                  {source.sourceFile || source.filename || source.url}
                </span>
                {source.line != null && (
                  <span className="font-mono text-[9px] text-muted-foreground">L{source.line}</span>
                )}
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-1">
                {source.method && (
                  <span className="rounded bg-blue-500/10 px-1.5 py-0.5 font-mono text-[9px] text-blue-300">
                    {source.method}
                  </span>
                )}
                {source.kind && (
                  <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                    {source.kind}
                  </span>
                )}
                {source.confidence != null && (
                  <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                    {Math.round(source.confidence * 100)}%
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p className="mt-1 text-[10px] text-muted-foreground">No JS source mapping stored.</p>
      )}
    </div>
  );
}

function ResponseDetail({
  item,
  jsSources,
}: {
  item: SitemapItem | null;
  jsSources: SitemapJsSource[];
}) {
  if (!item) {
    return (
      <div className="rounded border border-border/20 bg-muted/5 p-3 text-[11px] text-muted-foreground">
        No JS-derived endpoint selected.
      </div>
    );
  }

  const headerEntries = Object.entries(item.headers ?? {});
  return (
    <div className="rounded border border-border/20 bg-muted/5">
      <div className="border-b border-border/15 px-2.5 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="w-12 text-right font-mono text-[10px] text-blue-300">{item.method}</span>
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/85">
            {item.path || item.url}
          </span>
          {item.statusCode != null && (
            <span className="font-mono text-[10px] text-green-300">{item.statusCode}</span>
          )}
        </div>
        <div className="mt-1 truncate pl-14 font-mono text-[9px] text-muted-foreground">
          {item.url}
        </div>
      </div>
      <div className="space-y-2 p-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <EndpointParamChips params={item.params} emptyLabel="0 params" />
          <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
            {item.source}
          </span>
          {item.contentType && (
            <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
              {item.contentType}
            </span>
          )}
        </div>

        <JsSourceList sources={jsSources} />

        <div className="rounded bg-background/25 px-2 py-1.5">
          <p className="text-[9px] uppercase text-muted-foreground">Capture</p>
          <p className="mt-1 truncate font-mono text-[10px] text-foreground/80">
            {item.capturePath || "No response capture stored"}
          </p>
        </div>

        <div className="rounded bg-background/25 px-2 py-1.5">
          <p className="text-[9px] uppercase text-muted-foreground">Response Headers</p>
          {headerEntries.length > 0 ? (
            <div className="mt-1 max-h-44 space-y-1 overflow-auto">
              {headerEntries.slice(0, 16).map(([key, value]) => (
                <div key={key} className="grid grid-cols-[96px_minmax(0,1fr)] gap-2 text-[10px]">
                  <span className="truncate font-mono text-muted-foreground">{key}</span>
                  <span className="min-w-0 truncate font-mono text-foreground/80">
                    {String(value)}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <p className="mt-1 text-[10px] text-muted-foreground">No headers stored.</p>
          )}
        </div>
      </div>
    </div>
  );
}

export function SitemapTab({
  items,
  tree,
  jsResults,
  loading,
}: {
  items: SitemapItem[];
  tree: SitemapTreeNode[];
  jsResults: JsAnalysisResult[];
  loading: boolean;
}) {
  const flatItems = useMemo(() => flattenTreeItems(tree), [tree]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selectedItem = flatItems.find((item) => item.id === selectedId) ?? flatItems[0] ?? null;
  const selectedJsSources = useMemo(
    () => (selectedItem ? buildSitemapJsSources(selectedItem, jsResults) : []),
    [selectedItem, jsResults]
  );

  if (items.length === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<Network className="w-5 h-5" />}
        title="No JS sitemap data yet"
        body="Collect JavaScript or browser runtime APIs to populate deterministic endpoints."
      />
    );
  }

  return (
    <Section title="JS Sitemap" subtitle={`${items.length} endpoints across ${tree.length} roots`}>
      <div className="grid items-start gap-2.5 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.8fr)]">
        <div className="overflow-hidden rounded border border-border/20 bg-muted/5">
          {tree.map((node) => (
            <SitemapNode
              key={node.id}
              node={node}
              depth={0}
              selectedId={selectedItem?.id ?? null}
              onSelect={(item) => setSelectedId(item.id)}
            />
          ))}
        </div>
        <ResponseDetail item={selectedItem} jsSources={selectedJsSources} />
      </div>
    </Section>
  );
}
