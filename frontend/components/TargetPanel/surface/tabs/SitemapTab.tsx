import jsBeautify from "js-beautify";
import {
  ChevronDown,
  ChevronRight,
  FileCode2,
  FileText,
  FolderTree,
  Loader2,
  Network,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { readCaptureText } from "@/lib/api/security-analysis";
import type { JsAnalysisResult } from "@/lib/security-analysis";
import { cn } from "@/lib/utils";
import { EndpointParamChips } from "../EndpointParamChips";
import { Inspector } from "../Inspector/Inspector";
import { EmptyPanel } from "../SurfaceParts";
import { buildSitemapJsSources, buildSitemapTree } from "../surfaceModel";
import type { SitemapItem, SitemapItemKind, SitemapJsSource, SitemapTreeNode } from "../types";

type SitemapFilter = "all" | SitemapItemKind;

const FILTERS: Array<{ id: SitemapFilter; label: string }> = [
  { id: "all", label: "All" },
  { id: "directory", label: "URLs" },
  { id: "endpoint", label: "Endpoints" },
  { id: "script", label: "Scripts" },
];
const MAX_SOURCE_PREVIEW_CHARS = 200_000;

function formatBytes(bytes: number | null): string | null {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null;
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function countArray(value: unknown): number {
  return Array.isArray(value) ? value.length : 0;
}

function flattenTreeItems(nodes: SitemapTreeNode[]): SitemapItem[] {
  return nodes.flatMap((node) => [...node.items, ...flattenTreeItems(node.children)]);
}

function collectExpandableNodeIds(nodes: SitemapTreeNode[]): Set<string> {
  const ids = new Set<string>();
  for (const node of nodes) {
    if (node.children.length > 0) ids.add(node.id);
    for (const id of collectExpandableNodeIds(node.children)) ids.add(id);
  }
  return ids;
}

function rootExpandedNodeIds(nodes: SitemapTreeNode[]): Set<string> {
  return new Set(nodes.filter((node) => node.children.length > 0).map((node) => node.id));
}

function flattenVisibleTreeItems(
  nodes: SitemapTreeNode[],
  expandedIds: Set<string>
): SitemapItem[] {
  const out: SitemapItem[] = [];
  for (const node of nodes) {
    const hasChildren = node.children.length > 0;
    const inlineItem = node.items.length === 1 && !hasChildren ? node.items[0] : null;
    if (inlineItem) out.push(inlineItem);
    if (!hasChildren || !expandedIds.has(node.id)) continue;
    if (!inlineItem) out.push(...node.items);
    out.push(...flattenVisibleTreeItems(node.children, expandedIds));
  }
  return out;
}

function SitemapItemMeta({ item }: { item: SitemapItem }) {
  const isScript = item.kind === "script";
  const sizeLabel = formatBytes(item.sizeBytes);

  if (isScript) {
    return (
      <>
        <span className="rounded bg-[var(--ansi-yellow)]/10 px-1.5 py-0.5 text-[9px] text-[var(--ansi-yellow)]/90">
          js
        </span>
        {sizeLabel && (
          <span className="font-mono text-[10px] text-muted-foreground">{sizeLabel}</span>
        )}
      </>
    );
  }

  if (item.kind === "directory") {
    return (
      <>
        <span className="rounded bg-green-500/10 px-1.5 py-0.5 text-[9px] text-green-300">url</span>
        {item.statusCode != null && (
          <span className="font-mono text-[10px] text-green-300">{item.statusCode}</span>
        )}
        {sizeLabel && (
          <span className="font-mono text-[10px] text-muted-foreground">{sizeLabel}</span>
        )}
      </>
    );
  }

  return (
    <>
      <span className="w-10 text-right font-mono text-[10px] text-blue-300">{item.method}</span>
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
  );
}

function SitemapItemRow({
  item,
  label,
  depth,
  selectedId,
  onSelect,
}: {
  item: SitemapItem;
  label: string;
  depth: number;
  selectedId: string | null;
  onSelect: (item: SitemapItem) => void;
}) {
  const paddingLeft = Math.min(depth * 14, 70);
  const selected = item.id === selectedId;
  const isScript = item.kind === "script";

  return (
    <div
      className={cn(
        "flex min-h-7 cursor-pointer items-center gap-1.5 border-b border-border/10 px-2 py-1 text-[11px] last:border-b-0 hover:bg-muted/20",
        selected && "bg-accent/10 text-accent"
      )}
      style={{ paddingLeft: `${paddingLeft + 8}px` }}
      title={item.url}
      onClick={() => onSelect(item)}
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        onSelect(item);
      }}
      role="button"
      tabIndex={0}
    >
      <span className="flex h-4 w-4 flex-shrink-0 items-center justify-center text-muted-foreground">
        {isScript ? (
          <FileCode2 className="h-3 w-3 text-[var(--ansi-yellow)]/80" />
        ) : item.kind === "directory" ? (
          <FolderTree className="h-3 w-3 text-green-300/80" />
        ) : (
          <FileText className="h-3 w-3" />
        )}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-foreground/80">{label}</span>
      <SitemapItemMeta item={item} />
    </div>
  );
}

function SitemapNode({
  node,
  depth,
  selectedId,
  expandedIds,
  onSelect,
  onToggle,
}: {
  node: SitemapTreeNode;
  depth: number;
  selectedId: string | null;
  expandedIds: Set<string>;
  onSelect: (item: SitemapItem) => void;
  onToggle: (id: string) => void;
}) {
  const hasChildren = node.children.length > 0;
  const expanded = !hasChildren || expandedIds.has(node.id);
  const inlineItem = node.items.length === 1 && !hasChildren ? node.items[0] : null;
  const selected = node.items.some((candidate) => candidate.id === selectedId);
  const paddingLeft = Math.min(depth * 14, 70);
  const isScript = inlineItem?.kind === "script";
  const isDirectory = inlineItem?.kind === "directory";
  const interactive = hasChildren || Boolean(inlineItem);

  return (
    <div>
      <div
        className={cn(
          "flex min-h-7 items-center gap-1.5 border-b border-border/10 px-2 py-1 text-[11px] last:border-b-0",
          interactive && "cursor-pointer hover:bg-muted/20",
          selected && "bg-accent/10 text-accent"
        )}
        style={{ paddingLeft: `${paddingLeft + 8}px` }}
        title={node.url ?? node.label}
        onClick={() => {
          if (hasChildren) onToggle(node.id);
          else if (inlineItem) onSelect(inlineItem);
        }}
        onKeyDown={(event) => {
          if (!interactive || (event.key !== "Enter" && event.key !== " ")) return;
          event.preventDefault();
          if (hasChildren) onToggle(node.id);
          else if (inlineItem) onSelect(inlineItem);
        }}
        role={interactive ? "button" : undefined}
        tabIndex={interactive ? 0 : undefined}
      >
        <span className="flex h-4 w-4 flex-shrink-0 items-center justify-center text-muted-foreground">
          {hasChildren && expanded ? (
            <ChevronDown className="h-3 w-3" />
          ) : hasChildren ? (
            <ChevronRight className="h-3 w-3" />
          ) : isScript ? (
            <FileCode2 className="h-3 w-3 text-[var(--ansi-yellow)]/80" />
          ) : isDirectory ? (
            <FolderTree className="h-3 w-3 text-green-300/80" />
          ) : (
            <FileText className="h-3 w-3" />
          )}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-foreground/80">{node.label}</span>
        {node.itemCount > 1 && (
          <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] tabular-nums text-muted-foreground">
            {node.itemCount}
          </span>
        )}
        {inlineItem && <SitemapItemMeta item={inlineItem} />}
      </div>
      {expanded &&
        !inlineItem &&
        node.items.map((item) => (
          <SitemapItemRow
            key={item.id}
            item={item}
            label={item.path || item.url}
            depth={depth + 1}
            selectedId={selectedId}
            onSelect={onSelect}
          />
        ))}
      {expanded &&
        node.children.map((child) => (
          <SitemapNode
            key={child.id}
            node={child}
            depth={depth + 1}
            selectedId={selectedId}
            expandedIds={expandedIds}
            onSelect={onSelect}
            onToggle={onToggle}
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

function EndpointDetail({
  item,
  jsSources,
  projectPath,
}: {
  item: SitemapItem;
  jsSources: SitemapJsSource[];
  projectPath: string | null;
}) {
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

        <Inspector projectPath={projectPath} capturePath={item.capturePath} />

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

function MetricChip({ label, value }: { label: string; value: number }) {
  return (
    <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
      {label} {value}
    </span>
  );
}

function beautifyJavaScript(content: string): string {
  try {
    return jsBeautify.js(content, {
      indent_size: 2,
      max_preserve_newlines: 2,
      preserve_newlines: true,
      wrap_line_length: 120,
    });
  } catch {
    return content;
  }
}

function sourcePreviewText(content: string): { text: string; truncated: boolean } {
  const source =
    content.length > MAX_SOURCE_PREVIEW_CHARS
      ? content.slice(0, MAX_SOURCE_PREVIEW_CHARS)
      : content;
  const beautified = beautifyJavaScript(source);
  if (content.length <= MAX_SOURCE_PREVIEW_CHARS && beautified.length <= MAX_SOURCE_PREVIEW_CHARS) {
    return { text: beautified, truncated: false };
  }
  return {
    text: `${beautified.slice(0, MAX_SOURCE_PREVIEW_CHARS)}\n\n/* preview truncated */`,
    truncated: true,
  };
}

function ScriptSourcePreview({
  projectPath,
  capturePath,
}: {
  projectPath: string | null;
  capturePath: string | null;
}) {
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const preview = useMemo(() => (content == null ? null : sourcePreviewText(content)), [content]);

  useEffect(() => {
    let cancelled = false;
    setContent(null);
    setError(null);
    if (!projectPath || !capturePath) {
      setLoading(false);
      return undefined;
    }

    setLoading(true);
    readCaptureText(projectPath, capturePath)
      .then((text) => {
        if (!cancelled) setContent(text);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [capturePath, projectPath]);

  if (!capturePath) {
    return (
      <div className="rounded bg-background/25 px-2 py-1.5 text-[10px] text-muted-foreground">
        No JS capture path stored for this file.
      </div>
    );
  }

  if (!projectPath) {
    return (
      <div className="rounded bg-background/25 px-2 py-1.5 text-[10px] text-muted-foreground">
        Open a project workspace to read this JS file.
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded border border-red-500/25 bg-red-500/5 px-2 py-1.5 text-[10px] text-red-300">
        {error}
      </div>
    );
  }

  if (loading || preview == null) {
    return (
      <div className="flex items-center gap-1.5 rounded bg-background/25 px-2 py-1.5 text-[10px] text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        Loading JS source
      </div>
    );
  }

  return (
    <div className="rounded border border-border/15 bg-background/25 px-2 py-1.5">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[9px] uppercase text-muted-foreground">Source · beautified</p>
        <span className="truncate font-mono text-[9px] text-muted-foreground">{capturePath}</span>
      </div>
      <pre className="mt-1 overflow-x-auto whitespace-pre rounded bg-[#07090c]/80 px-2 py-2 font-mono text-[10px] leading-relaxed text-foreground/80 [tab-size:2]">
        {preview.text}
      </pre>
      {preview.truncated && (
        <p className="mt-1 text-[9px] text-muted-foreground">
          Showing first {MAX_SOURCE_PREVIEW_CHARS.toLocaleString()} characters.
        </p>
      )}
    </div>
  );
}

function ScriptDetail({
  item,
  result,
  projectPath,
}: {
  item: SitemapItem;
  result: JsAnalysisResult | null;
  projectPath: string | null;
}) {
  const sizeLabel = formatBytes(item.sizeBytes ?? result?.sizeBytes ?? null);
  const skipped = result?.rawAnalysis?.skipped === true;
  const skipReason =
    typeof result?.rawAnalysis?.skipped_reason === "string"
      ? (result.rawAnalysis.skipped_reason as string)
      : null;
  const aiReview = result?.rawAnalysis?.ai_review;

  return (
    <div className="rounded border border-border/20 bg-muted/5">
      <div className="border-b border-border/15 px-2.5 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <FileCode2 className="h-3.5 w-3.5 flex-shrink-0 text-[var(--ansi-yellow)]/85" />
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/85">
            {result?.filename || item.path || item.url}
          </span>
          {sizeLabel && (
            <span className="font-mono text-[10px] text-muted-foreground">{sizeLabel}</span>
          )}
        </div>
        <div className="mt-1 truncate pl-6 font-mono text-[9px] text-muted-foreground">
          {item.url}
        </div>
      </div>
      <div className="space-y-2 p-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="rounded bg-[var(--ansi-yellow)]/10 px-1.5 py-0.5 text-[9px] text-[var(--ansi-yellow)]/90">
            script
          </span>
          {skipped && (
            <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-[9px] text-red-300">
              {skipReason ? `skipped · ${skipReason}` : "skipped · oversized"}
            </span>
          )}
          {result && (
            <>
              <MetricChip label="endpoints" value={countArray(result.endpointsFound)} />
              <MetricChip label="secrets" value={countArray(result.secretsFound)} />
              <MetricChip label="frameworks" value={countArray(result.frameworks)} />
              <MetricChip label="libraries" value={countArray(result.libraries)} />
              {result.sourceMaps && (
                <span className="rounded bg-[var(--ansi-magenta)]/10 px-1.5 py-0.5 text-[9px] text-[var(--ansi-magenta)]/90">
                  source map
                </span>
              )}
            </>
          )}
        </div>

        {result?.riskSummary && (
          <div className="rounded bg-background/25 px-2 py-1.5 text-[10px] leading-relaxed text-foreground/75">
            {result.riskSummary}
          </div>
        )}

        {typeof aiReview === "string" && aiReview.trim() && (
          <div className="rounded bg-background/25 px-2 py-1.5">
            <p className="text-[9px] uppercase text-muted-foreground">AI review</p>
            <p className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap text-[10px] text-foreground/80">
              {aiReview}
            </p>
          </div>
        )}

        {!result && (
          <p className="text-[10px] text-muted-foreground">
            This script was collected but not analyzed (no stored analysis row).
          </p>
        )}

        <ScriptSourcePreview projectPath={projectPath} capturePath={item.capturePath} />
      </div>
    </div>
  );
}

function DetailPanel({
  item,
  jsResults,
  jsSources,
  projectPath,
}: {
  item: SitemapItem | null;
  jsResults: JsAnalysisResult[];
  jsSources: SitemapJsSource[];
  projectPath: string | null;
}) {
  if (!item) {
    return (
      <div className="rounded border border-border/20 bg-muted/5 p-3 text-[11px] text-muted-foreground">
        No sitemap entry selected.
      </div>
    );
  }
  if (item.kind === "script") {
    const result = jsResults.find((candidate) => candidate.id === item.id) ?? null;
    return <ScriptDetail item={item} result={result} projectPath={projectPath} />;
  }
  return <EndpointDetail item={item} jsSources={jsSources} projectPath={projectPath} />;
}

function unassignedCount(unassignedWebData?: SitemapTabProps["unassignedWebData"]): number {
  if (!unassignedWebData) return 0;
  return (
    unassignedWebData.urls.length +
    unassignedWebData.apis.length +
    unassignedWebData.js.length +
    unassignedWebData.params.length
  );
}

function UnassignedWebDataPanel({
  unassignedWebData,
}: {
  unassignedWebData?: SitemapTabProps["unassignedWebData"];
}) {
  const count = unassignedCount(unassignedWebData);
  if (!unassignedWebData || count === 0) return null;
  return (
    <div className="rounded border border-yellow-500/25 bg-yellow-500/5 px-2 py-1.5 text-[10px] text-yellow-200/90">
      <div className="font-medium text-yellow-100">未归属 Web 数据</div>
      <p className="mt-0.5 leading-relaxed text-yellow-100/75">{unassignedWebData.reason}</p>
      <div className="mt-1 flex flex-wrap gap-1">
        <span className="rounded bg-background/25 px-1.5 py-0.5">
          URLs {unassignedWebData.urls.length}
        </span>
        <span className="rounded bg-background/25 px-1.5 py-0.5">
          APIs {unassignedWebData.apis.length}
        </span>
        <span className="rounded bg-background/25 px-1.5 py-0.5">
          JS {unassignedWebData.js.length}
        </span>
        <span className="rounded bg-background/25 px-1.5 py-0.5">
          Params {unassignedWebData.params.length}
        </span>
      </div>
    </div>
  );
}

interface SitemapTabProps {
  items: SitemapItem[];
  jsResults: JsAnalysisResult[];
  loading: boolean;
  projectPath: string | null;
  originLabel?: string;
  unassignedWebData?: {
    urls: unknown[];
    apis: unknown[];
    js: unknown[];
    params: unknown[];
    reason: string;
  };
}

export function SitemapTab({
  items,
  jsResults,
  loading,
  projectPath,
  originLabel,
  unassignedWebData,
}: SitemapTabProps) {
  const [filter, setFilter] = useState<SitemapFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  const counts = useMemo(
    () => ({
      all: items.length,
      directory: items.filter((item) => item.kind === "directory").length,
      endpoint: items.filter((item) => item.kind === "endpoint").length,
      script: items.filter((item) => item.kind === "script").length,
    }),
    [items]
  );

  const filteredItems = useMemo(
    () => (filter === "all" ? items : items.filter((item) => item.kind === filter)),
    [items, filter]
  );
  const tree = useMemo(() => buildSitemapTree(filteredItems), [filteredItems]);
  const flatItems = useMemo(() => flattenTreeItems(tree), [tree]);
  const visibleItems = useMemo(
    () => flattenVisibleTreeItems(tree, expandedIds),
    [tree, expandedIds]
  );
  const selectedItem =
    (selectedId ? flatItems.find((item) => item.id === selectedId) : null) ??
    visibleItems[0] ??
    null;
  const toggleExpanded = useCallback((id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  useEffect(() => {
    setExpandedIds((prev) => {
      const validIds = collectExpandableNodeIds(tree);
      const next = new Set([...prev].filter((id) => validIds.has(id)));
      if (next.size === 0) return rootExpandedNodeIds(tree);
      return next;
    });
  }, [tree]);

  const selectedJsSources = useMemo(
    () =>
      selectedItem && selectedItem.kind === "endpoint"
        ? buildSitemapJsSources(selectedItem, jsResults)
        : [],
    [selectedItem, jsResults]
  );

  if (items.length === 0 && unassignedCount(unassignedWebData) === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<Network className="w-5 h-5" />}
        title="No sitemap data yet"
        body="Collect URLs, JavaScript, or browser runtime APIs to populate the site map."
      />
    );
  }

  return (
    <section className="flex h-full min-h-0 flex-col rounded border border-border/25 bg-background/15">
      <div className="flex h-8 flex-shrink-0 items-center justify-between border-b border-border/20 px-2.5">
        <h4 className="min-w-0 truncate text-[11px] font-medium text-foreground">
          {originLabel ? `Sitemap · ${originLabel}` : "Sitemap"}
        </h4>
        <span className="text-[9px] text-muted-foreground">
          {counts.directory} URLs · {counts.endpoint} endpoints · {counts.script} scripts across{" "}
          {tree.length} roots
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col p-2.5">
        <div className="mb-2 flex-shrink-0">
          <UnassignedWebDataPanel unassignedWebData={unassignedWebData} />
        </div>
        <div className="mb-2 flex flex-shrink-0 items-center gap-1">
          {FILTERS.map((option) => (
            <button
              key={option.id}
              type="button"
              onClick={() => setFilter(option.id)}
              className={cn(
                "inline-flex items-center gap-1 rounded px-2 py-0.5 text-[10px] transition-colors",
                filter === option.id
                  ? "bg-muted/30 text-foreground"
                  : "text-muted-foreground hover:bg-muted/20 hover:text-foreground"
              )}
            >
              {option.label}
              <span className="rounded bg-background/40 px-1 py-0.5 text-[8px] tabular-nums">
                {counts[option.id]}
              </span>
            </button>
          ))}
        </div>
        {tree.length === 0 ? (
          <EmptyPanel
            loading={loading}
            icon={<Network className="w-5 h-5" />}
            title="No assigned sitemap data"
            body="The collected rows exist, but none include a complete origin for this view."
          />
        ) : (
          <div className="grid min-h-0 flex-1 items-start gap-2.5 overflow-hidden lg:grid-cols-[minmax(250px,0.7fr)_minmax(460px,1.3fr)]">
            <div className="h-full min-h-0 overflow-y-auto rounded border border-border/20 bg-muted/5">
              {tree.map((node) => (
                <SitemapNode
                  key={node.id}
                  node={node}
                  depth={0}
                  selectedId={selectedItem?.id ?? null}
                  expandedIds={expandedIds}
                  onSelect={(item) => setSelectedId(item.id)}
                  onToggle={toggleExpanded}
                />
              ))}
            </div>
            <div className="h-full min-h-0 overflow-y-auto">
              <DetailPanel
                item={selectedItem}
                jsResults={jsResults}
                jsSources={selectedJsSources}
                projectPath={projectPath}
              />
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
