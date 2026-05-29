import { Activity, Building2, Database, ExternalLink, Globe2, Server } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TopologyNode } from "./types";

export function TopologyInspector({ node }: { node: TopologyNode | null }) {
  if (!node) {
    return (
      <aside className="flex h-full w-[292px] shrink-0 flex-col border-l border-border/35 bg-card/35 p-4">
        <div className="text-[10px] font-bold uppercase text-muted-foreground">Inspector</div>
        <div className="mt-8 flex flex-1 items-center justify-center text-center text-sm text-muted-foreground">
          Select a graph node.
        </div>
      </aside>
    );
  }

  return (
    <aside className="flex h-full w-[292px] shrink-0 flex-col border-l border-border/35 bg-card/35">
      <div className="border-b border-border/25 px-4 py-4">
        <div className="text-[10px] font-bold uppercase text-muted-foreground">Inspector</div>
        <div className="mt-2 flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="truncate text-[15px] font-semibold text-foreground">{node.label}</div>
            <div className="mt-1 text-[11px] text-muted-foreground">{node.subtitle}</div>
          </div>
          {node.scope && (
            <span
              className={cn(
                "rounded-md border px-2 py-1 text-[10px] font-bold uppercase",
                node.scope === "in"
                  ? "border-emerald-400/25 bg-emerald-400/10 text-emerald-300"
                  : "border-rose-400/25 bg-rose-400/10 text-rose-300"
              )}
            >
              {node.scope}
            </span>
          )}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        <SummarySection node={node} />
        {node.kind === "target" && <TargetSurfaceSummary node={node} />}
        {node.kind === "organization" && <OrganizationSummary node={node} />}
        {node.kind === "service" && <ServiceSummary node={node} />}
        {node.kind === "evidence" && <EvidenceSummary />}
      </div>

      <div className="grid grid-cols-2 gap-2 border-t border-border/25 p-4">
        <button
          type="button"
          className="inline-flex h-8 items-center justify-center gap-1.5 rounded-md border border-cyan-400/20 bg-cyan-400/10 text-[10px] font-semibold text-cyan-200"
        >
          <ExternalLink className="h-3 w-3" />
          Workbench
        </button>
        <button
          type="button"
          className="inline-flex h-8 items-center justify-center gap-1.5 rounded-md border border-border/35 bg-background/20 text-[10px] font-semibold text-muted-foreground"
        >
          <Activity className="h-3 w-3" />
          Recon
        </button>
      </div>
    </aside>
  );
}

function SummarySection({ node }: { node: TopologyNode }) {
  const Icon =
    node.kind === "organization"
      ? Building2
      : node.kind === "target"
        ? Globe2
        : node.kind === "service"
          ? Server
          : Database;
  return (
    <section>
      <div className="text-[10px] font-bold uppercase text-muted-foreground">Summary</div>
      <div className="mt-2 rounded-lg border border-border/35 bg-background/25 p-3">
        <div className="flex items-center gap-2 text-[12px] font-medium text-foreground">
          <Icon className="h-3.5 w-3.5 text-cyan-300" />
          {node.kind}
        </div>
        <div className="mt-3 space-y-2 text-[11px]">
          <Kv label="Label" value={node.label} />
          <Kv label="Type" value={node.target?.type ?? node.kind} />
          {node.target?.source && <Kv label="Source" value={node.target.source} />}
          {node.target?.real_ip && <Kv label="Resolved IP" value={node.target.real_ip} mono />}
        </div>
      </div>
    </section>
  );
}

function TargetSurfaceSummary({ node }: { node: TopologyNode }) {
  return (
    <section className="mt-5">
      <div className="text-[10px] font-bold uppercase text-muted-foreground">Surface</div>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <Metric label="Open ports" value={node.metrics?.ports ?? 0} />
        <Metric label="Evidence" value={node.metrics?.evidence ?? 0} tone="cyan" />
        <Metric label="HTTP" value={node.target?.http_status ?? "—"} />
        <Metric label="Status" value={node.target?.status ?? "new"} />
      </div>
    </section>
  );
}

function OrganizationSummary({ node }: { node: TopologyNode }) {
  return (
    <section className="mt-5">
      <div className="text-[10px] font-bold uppercase text-muted-foreground">Ownership</div>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <Metric label="Targets" value={node.metrics?.targets ?? 0} />
        <Metric label="In scope" value={node.metrics?.inScopeTargets ?? 0} tone="green" />
      </div>
    </section>
  );
}

function ServiceSummary({ node }: { node: TopologyNode }) {
  return (
    <section className="mt-5">
      <div className="text-[10px] font-bold uppercase text-muted-foreground">Service</div>
      <div className="mt-2 rounded-lg border border-border/35 bg-background/25 p-3 text-[11px]">
        <Kv label="Port" value={String(node.port?.port ?? "—")} mono />
        <Kv label="Protocol" value={node.port?.protocol ?? "tcp"} />
        <Kv label="Service" value={node.port?.service ?? node.port?.webserver ?? "—"} />
        <Kv label="HTTP" value={String(node.port?.http_status ?? "—")} />
      </div>
    </section>
  );
}

function EvidenceSummary() {
  return (
    <section className="mt-5">
      <div className="text-[10px] font-bold uppercase text-muted-foreground">Evidence trail</div>
      <div className="mt-2 space-y-2">
        {["target source", "service observation", "surface metadata"].map((item, index) => (
          <div key={item} className="rounded-lg border border-border/35 bg-background/25 px-3 py-2">
            <div className="text-[11px] font-medium text-foreground">{item}</div>
            <div className="mt-0.5 text-[10px] text-muted-foreground">evidence row {index + 1}</div>
          </div>
        ))}
      </div>
    </section>
  );
}

function Metric({
  label,
  value,
  tone = "default",
}: {
  label: string;
  value: string | number;
  tone?: "default" | "cyan" | "green";
}) {
  return (
    <div className="rounded-lg border border-border/35 bg-background/25 p-3">
      <div
        className={cn(
          "text-[15px] font-semibold",
          tone === "cyan"
            ? "text-cyan-200"
            : tone === "green"
              ? "text-emerald-300"
              : "text-foreground"
        )}
      >
        {value}
      </div>
      <div className="mt-1 text-[10px] text-muted-foreground">{label}</div>
    </div>
  );
}

function Kv({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="grid grid-cols-[76px_1fr] gap-2">
      <span className="text-muted-foreground">{label}</span>
      <span className={cn("min-w-0 truncate text-foreground", mono && "font-mono tabular-nums")}>
        {value}
      </span>
    </div>
  );
}
