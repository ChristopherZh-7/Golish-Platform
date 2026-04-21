import { AlertTriangle, ExternalLink } from "lucide-react";
import { cn } from "@/lib/utils";
import type { VulnEntry } from "./types";
import { SEV_COLORS } from "./types";

export function IntelTab({ entry }: { entry: VulnEntry }) {
  return (
    <div className="space-y-2.5">
      {/* Meta grid */}
      <div className="grid grid-cols-2 gap-x-4 gap-y-1.5">
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">CVE ID</span>
          <div className="text-[10px] font-mono text-accent">{entry.cve_id}</div>
        </div>
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">CVSS Score</span>
          <div className="flex items-center gap-1.5 mt-0.5">
            {entry.cvss_score != null ? (
              <span className={cn(
                "text-[11px] font-bold",
                entry.cvss_score >= 9 ? "text-red-400" :
                entry.cvss_score >= 7 ? "text-orange-400" :
                entry.cvss_score >= 4 ? "text-yellow-400" : "text-blue-400"
              )}>
                {entry.cvss_score.toFixed(1)}
              </span>
            ) : null}
            <span className={cn("text-[8px] px-1.5 py-0.5 rounded border capitalize",
              SEV_COLORS[entry.severity] || SEV_COLORS.info
            )}>{entry.severity}</span>
          </div>
        </div>
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Published</span>
          <div className="text-[10px] text-foreground/60">{entry.published ? new Date(entry.published).toLocaleDateString("zh-CN", { year: "numeric", month: "long", day: "numeric" }) : "Unknown"}</div>
        </div>
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Source</span>
          <div className="text-[10px] text-foreground/60">{entry.source || "N/A"}</div>
        </div>
      </div>

      {/* Description */}
      <div>
        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Description</span>
        <p className="text-[10px] text-foreground/60 leading-relaxed mt-0.5">{entry.description}</p>
      </div>

      {/* Affected Products */}
      {entry.affected_products.length > 0 && (
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Affected Products</span>
          <div className="flex flex-wrap gap-1 mt-0.5">
            {entry.affected_products.map((prod, i) => (
              <span key={i} className="text-[9px] px-1.5 py-0.5 bg-orange-500/10 text-orange-400 border border-orange-500/20 rounded">
                {prod}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Quick links */}
      <div className="flex items-center gap-3 pt-1 border-t border-border/10">
        <a href={`https://nvd.nist.gov/vuln/detail/${entry.cve_id}`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
          <ExternalLink className="w-2.5 h-2.5" /> NVD
        </a>
        <a href={`https://cve.mitre.org/cgi-bin/cvename.cgi?name=${entry.cve_id}`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
          <ExternalLink className="w-2.5 h-2.5" /> MITRE
        </a>
        <a href={`https://www.google.com/search?q=${entry.cve_id}+exploit+poc`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-red-400/60 hover:text-red-400 transition-colors">
          <AlertTriangle className="w-2.5 h-2.5" /> Search PoC
        </a>
        <a href={`https://github.com/search?q=${entry.cve_id}&type=repositories`} target="_blank" rel="noopener noreferrer"
          className="flex items-center gap-1 text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">
          <ExternalLink className="w-2.5 h-2.5" /> GitHub
        </a>
      </div>

      {/* References */}
      {entry.references.length > 0 && (
        <div>
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">References ({entry.references.length})</span>
          <div className="space-y-0.5 mt-0.5 max-h-24 overflow-y-auto">
            {entry.references.map((ref_, i) => (
              <a key={i} href={ref_} target="_blank" rel="noopener noreferrer"
                className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors truncate">
                <ExternalLink className="w-2.5 h-2.5 flex-shrink-0" />
                {ref_}
              </a>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

