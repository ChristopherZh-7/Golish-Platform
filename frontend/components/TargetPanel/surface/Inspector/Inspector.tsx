import { Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { type CapturePayload, readCapture } from "@/lib/api/security-analysis";
import { cn } from "@/lib/utils";
import { bodyRenderMode, prettyBody } from "./inspectorModel";

type InspectorTab = "request" | "response";

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

function formatBytes(bytes: number | null): string | null {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null;
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function headerValue(headers: Record<string, string>, name: string): string {
  const normalized = name.toLowerCase();
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() === normalized) return value;
  }
  return "";
}

function HeaderTable({ headers }: { headers: Record<string, string> }) {
  const entries = Object.entries(headers);
  if (entries.length === 0) {
    return <p className="mt-1 text-[10px] text-muted-foreground">Not captured yet.</p>;
  }
  return (
    <div className="mt-1 max-h-44 space-y-1 overflow-auto">
      {entries.map(([key, value]) => (
        <div key={key} className="grid grid-cols-[112px_minmax(0,1fr)] gap-2 text-[10px]">
          <span className="truncate font-mono text-muted-foreground">{key}</span>
          <span className="min-w-0 break-all font-mono text-foreground/80">{value}</span>
        </div>
      ))}
    </div>
  );
}

function BodyBlock({
  contentType,
  body,
  emptyLabel,
}: {
  contentType: string;
  body: string;
  emptyLabel: string;
}) {
  const mode = bodyRenderMode(contentType);
  const text = useMemo(() => prettyBody(mode, body), [body, mode]);
  return (
    <div className="rounded bg-background/25 px-2 py-1.5">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[9px] uppercase text-muted-foreground">Body</p>
        {body && (
          <span className="rounded bg-muted/25 px-1.5 py-0.5 text-[9px] text-muted-foreground">
            {mode}
          </span>
        )}
      </div>
      {body ? (
        <pre className="mt-1 max-h-72 overflow-auto whitespace-pre-wrap break-all font-mono text-[10px] leading-relaxed text-foreground/80">
          {text}
        </pre>
      ) : (
        <p className="mt-1 text-[10px] text-muted-foreground">{emptyLabel}</p>
      )}
    </div>
  );
}

function CaptureContent({ data, tab }: { data: CapturePayload; tab: InspectorTab }) {
  const isRequest = tab === "request";
  const headers = isRequest ? data.request.headers : data.response.headers;
  const contentType = isRequest
    ? headerValue(data.request.headers, "content-type")
    : data.response.contentType || headerValue(data.response.headers, "content-type");
  const body = isRequest ? (data.request.body ?? "") : data.response.bodyTextSample;
  const responseSize = formatBytes(data.response.bodyLen);

  return (
    <div className="space-y-2 p-2.5">
      <div className="rounded bg-background/25 px-2 py-1.5">
        <div className="flex min-w-0 items-center gap-1.5 font-mono text-[10px] text-foreground/80">
          <span className="rounded bg-blue-500/10 px-1.5 py-0.5 text-blue-300">
            {data.request.method || "GET"}
          </span>
          <span className="min-w-0 flex-1 break-all">{data.request.url}</span>
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[9px] text-muted-foreground">
          {data.capturedAt && <span>{data.capturedAt}</span>}
          <span>v{data.version}</span>
          {data.request.resourceType && <span>{data.request.resourceType}</span>}
          {!isRequest && data.response.status != null && (
            <span className="font-mono text-green-300">{data.response.status}</span>
          )}
          {!isRequest && responseSize && <span className="font-mono">{responseSize}</span>}
          {contentType && <span className="max-w-56 truncate">{contentType}</span>}
        </div>
      </div>

      <div className="rounded bg-background/25 px-2 py-1.5">
        <p className="text-[9px] uppercase text-muted-foreground">Headers</p>
        <HeaderTable headers={headers} />
      </div>

      <BodyBlock
        contentType={contentType}
        body={body}
        emptyLabel={
          isRequest ? "Request body was not captured yet." : "No response body sample stored."
        }
      />
    </div>
  );
}

export function Inspector({
  projectPath,
  capturePath,
}: {
  projectPath: string | null;
  capturePath: string | null;
}) {
  const [data, setData] = useState<CapturePayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [tab, setTab] = useState<InspectorTab>("response");

  useEffect(() => {
    let cancelled = false;
    setData(null);
    setError(null);
    if (!capturePath || !projectPath) {
      setLoading(false);
      return undefined;
    }

    setLoading(true);
    readCapture(projectPath, capturePath)
      .then((capture) => {
        if (!cancelled) setData(capture);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(formatError(err));
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
      <div className="rounded border border-border/20 bg-muted/5 p-3 text-[11px] text-muted-foreground">
        No response capture stored for this entry.
      </div>
    );
  }

  if (!projectPath) {
    return (
      <div className="rounded border border-border/20 bg-muted/5 p-3 text-[11px] text-muted-foreground">
        Open a project workspace to read this capture.
      </div>
    );
  }

  return (
    <div className="rounded border border-border/20 bg-muted/5">
      <div className="flex items-center gap-1 border-b border-border/15 px-2 py-1.5">
        {(["request", "response"] as const).map((option) => (
          <button
            key={option}
            type="button"
            onClick={() => setTab(option)}
            className={cn(
              "rounded px-2 py-0.5 text-[10px] transition-colors",
              tab === option
                ? "bg-muted/30 text-foreground"
                : "text-muted-foreground hover:bg-muted/20 hover:text-foreground"
            )}
          >
            {option === "request" ? "Request" : "Response"}
          </button>
        ))}
        {loading && (
          <span className="ml-auto inline-flex items-center gap-1 text-[10px] text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            Loading
          </span>
        )}
        {!loading && data?.response.status != null && (
          <span className="ml-auto font-mono text-[10px] text-green-300">
            {data.response.status}
          </span>
        )}
      </div>

      {error ? (
        <div className="p-2.5">
          <div className="rounded border border-red-500/25 bg-red-500/5 p-3 text-[10px] text-red-300">
            {error}
          </div>
        </div>
      ) : data ? (
        <CaptureContent data={data} tab={tab} />
      ) : (
        <div className="p-3 text-[11px] text-muted-foreground">Loading capture...</div>
      )}
    </div>
  );
}
