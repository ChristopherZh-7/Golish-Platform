import { FileCode2 } from "lucide-react";
import type { ApiEndpoint, JsAnalysisResult } from "@/lib/security-analysis";
import { EmptyPanel, Section } from "../SurfaceParts";

export function JsApiTab({
  endpoints,
  jsResults,
  loading,
}: {
  endpoints: ApiEndpoint[];
  jsResults: JsAnalysisResult[];
  loading: boolean;
}) {
  if (endpoints.length === 0 && jsResults.length === 0) {
    return (
      <EmptyPanel
        loading={loading}
        icon={<FileCode2 className="w-5 h-5" />}
        title="No JS or API evidence yet"
        body="Collect JavaScript to extract API endpoints, source maps, routes, and front-end security signals."
      />
    );
  }
  return (
    <div className="grid items-start grid-cols-2 gap-2.5">
      <Section title="API Endpoints" subtitle={`${endpoints.length} extracted`}>
        <div className="space-y-1">
          {endpoints.slice(0, 20).map((endpoint) => (
            <div
              key={endpoint.id}
              className="rounded border border-border/20 bg-muted/5 px-2 py-1.5"
            >
              <div className="flex items-center gap-2">
                <span className="w-10 text-right font-mono text-[10px] text-blue-300">
                  {endpoint.method}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/80">
                  {endpoint.path || endpoint.url}
                </span>
                <span className="rounded bg-muted/30 px-1.5 py-0.5 text-[9px] text-muted-foreground">
                  {endpoint.riskLevel || "info"}
                </span>
              </div>
            </div>
          ))}
        </div>
      </Section>
      <Section title="JavaScript Files" subtitle={`${jsResults.length} analyzed`}>
        <div className="space-y-1">
          {jsResults.slice(0, 20).map((result) => (
            <div key={result.id} className="rounded border border-border/20 bg-muted/5 px-2 py-1.5">
              <div className="flex items-center gap-2">
                <FileCode2 className="w-3.5 h-3.5 text-yellow-300" />
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/80">
                  {result.filename || result.url}
                </span>
                {result.sourceMaps && (
                  <span className="rounded bg-yellow-500/10 px-1.5 py-0.5 text-[9px] text-yellow-300">
                    map
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      </Section>
    </div>
  );
}
