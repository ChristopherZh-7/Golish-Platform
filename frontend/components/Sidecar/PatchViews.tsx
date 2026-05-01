import { useState } from "react";
import { Check, ChevronDown, ChevronRight, Clock, FileCode, GitCommit } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { StagedPatch } from "@/lib/sidecar";
import { cn } from "@/lib/utils";

export interface PatchWithStatus extends StagedPatch {
  status: "staged" | "applied";
}

interface PatchesViewProps {
  patches: PatchWithStatus[];
  selectedPatchId: number | null;
  selectedPatch: PatchWithStatus | null;
  onSelectPatch: (id: number | null) => void;
}

export function PatchesView({ patches, selectedPatchId, selectedPatch, onSelectPatch }: PatchesViewProps) {
  if (patches.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="text-center text-muted-foreground">
          <GitCommit className="w-8 h-8 mx-auto mb-2 opacity-50" />
          <p className="text-sm">No patches generated yet</p>
          <p className="text-xs mt-1">Patches will appear here as you work</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
      <div className="border-b border-border">
        <ScrollArea className="max-h-48">
          <div className="p-2 space-y-1">
            {patches.map((patch) => (
              <PatchListItem
                key={patch.meta.id}
                patch={patch}
                isSelected={selectedPatchId === patch.meta.id}
                onSelect={() => onSelectPatch(selectedPatchId === patch.meta.id ? null : patch.meta.id)}
              />
            ))}
          </div>
        </ScrollArea>
      </div>
      <div className="flex-1 min-h-0 overflow-hidden">
        {selectedPatch ? (
          <PatchDetail patch={selectedPatch} />
        ) : (
          <div className="h-full flex items-center justify-center text-muted-foreground text-sm">
            Select a patch to view details
          </div>
        )}
      </div>
    </div>
  );
}

function PatchListItem({ patch, isSelected, onSelect }: { patch: PatchWithStatus; isSelected: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "w-full p-2 rounded text-left transition-colors border border-transparent",
        isSelected ? "bg-[var(--ansi-blue)]/15 border-[var(--ansi-blue)]/50" : "hover:bg-muted/50"
      )}
    >
      <div className="flex items-start gap-2">
        <div className={cn("mt-0.5 p-1 rounded", patch.status === "applied" ? "bg-[var(--ansi-green)]/20" : "bg-[var(--ansi-yellow)]/20")}>
          {patch.status === "applied" ? <Check className="w-3 h-3 text-[var(--ansi-green)]" /> : <Clock className="w-3 h-3 text-[var(--ansi-yellow)]" />}
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-xs font-medium leading-tight line-clamp-2">{patch.subject}</p>
          <div className="flex items-center gap-2 mt-1 text-[10px] text-muted-foreground">
            <span>{patch.files.length} file{patch.files.length !== 1 ? "s" : ""}</span>
            <span>•</span>
            <span>{new Date(patch.meta.created_at).toLocaleTimeString()}</span>
            {patch.status === "applied" && patch.meta.applied_sha && (
              <>
                <span>•</span>
                <span className="font-mono">{patch.meta.applied_sha.slice(0, 7)}</span>
              </>
            )}
          </div>
        </div>
      </div>
    </button>
  );
}

function PatchDetail({ patch }: { patch: PatchWithStatus }) {
  const [showFiles, setShowFiles] = useState(true);

  return (
    <ScrollArea className="h-full">
      <div className="p-3 space-y-3">
        <div>
          <div className="flex items-center gap-2 mb-1">
            <span className={cn("text-[10px] px-1.5 py-0.5 rounded font-medium",
              patch.status === "applied" ? "bg-[var(--ansi-green)]/20 text-[var(--ansi-green)]" : "bg-[var(--ansi-yellow)]/20 text-[var(--ansi-yellow)]")}>
              {patch.status.toUpperCase()}
            </span>
            <span className="text-[10px] text-muted-foreground">#{patch.meta.id} • {new Date(patch.meta.created_at).toLocaleString()}</span>
          </div>
          <h3 className="text-sm font-medium leading-snug">{patch.subject}</h3>
        </div>

        {patch.message !== patch.subject && (
          <div>
            <p className="text-[10px] text-muted-foreground mb-1 font-medium">COMMIT MESSAGE</p>
            <pre className="text-xs font-mono whitespace-pre-wrap bg-muted p-2 rounded">{patch.message}</pre>
          </div>
        )}

        {patch.files.length > 0 && (
          <div>
            <button type="button" onClick={() => setShowFiles(!showFiles)}
              className="flex items-center gap-1 text-[10px] text-muted-foreground mb-1 font-medium hover:text-foreground">
              {showFiles ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
              FILES CHANGED ({patch.files.length})
            </button>
            {showFiles && (
              <div className="space-y-0.5">
                {patch.files.map((file) => (
                  <div key={file} className="flex items-center gap-1.5 text-xs font-mono py-1 px-2 bg-muted/50 rounded">
                    <FileCode className="w-3 h-3 text-[var(--ansi-blue)] shrink-0" />
                    <span className="truncate" title={file}>{file}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {patch.status === "applied" && patch.meta.applied_sha && (
          <div>
            <p className="text-[10px] text-muted-foreground mb-1 font-medium">COMMIT SHA</p>
            <code className="text-xs font-mono bg-muted px-2 py-1 rounded">{patch.meta.applied_sha}</code>
          </div>
        )}

        {patch.patch_content && (
          <div>
            <p className="text-[10px] text-muted-foreground mb-1 font-medium">DIFF</p>
            <DiffViewer content={patch.patch_content} />
          </div>
        )}
      </div>
    </ScrollArea>
  );
}

export function DiffViewer({ content }: { content: string }) {
  const lines = content.split("\n");
  const diffSections: { file: string; lines: { text: string; type: "add" | "del" | "hunk" | "context" | "header" }[] }[] = [];

  let currentFile = "";
  let currentLines: { text: string; type: "add" | "del" | "hunk" | "context" | "header" }[] = [];

  for (const line of lines) {
    if (line.startsWith("diff --git ")) {
      if (currentFile && currentLines.length > 0) diffSections.push({ file: currentFile, lines: currentLines });
      const match = line.match(/diff --git a\/(.+?) b\//);
      currentFile = match?.[1] ?? "unknown";
      currentLines = [];
    } else if (line.startsWith("@@")) {
      currentLines.push({ text: line, type: "hunk" });
    } else if (line.startsWith("+") && !line.startsWith("+++")) {
      currentLines.push({ text: line, type: "add" });
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      currentLines.push({ text: line, type: "del" });
    } else if (line.startsWith("index ") || line.startsWith("--- ") || line.startsWith("+++ ")) {
      currentLines.push({ text: line, type: "header" });
    } else if (currentFile) {
      currentLines.push({ text: line, type: "context" });
    }
  }
  if (currentFile && currentLines.length > 0) diffSections.push({ file: currentFile, lines: currentLines });

  if (diffSections.length === 0) return <p className="text-xs text-muted-foreground">No diff content</p>;

  return (
    <div className="space-y-2">
      {diffSections.map((section, idx) => (
        <div key={`${section.file}-${idx}`} className="rounded overflow-hidden border border-border">
          <div className="bg-muted px-2 py-1 text-[10px] font-mono text-muted-foreground border-b border-border">{section.file}</div>
          <pre className="text-[11px] font-mono overflow-x-auto">
            {section.lines.map((line, lineIdx) => (
              <div key={`${lineIdx}-${line.type}-${line.text.slice(0, 20)}`}
                className={cn("px-2 leading-5",
                  line.type === "add" && "bg-[var(--ansi-green)]/10 text-[var(--ansi-green)]",
                  line.type === "del" && "bg-[var(--ansi-red)]/10 text-[var(--ansi-red)]",
                  line.type === "hunk" && "bg-[var(--ansi-blue)]/10 text-[var(--ansi-blue)]",
                  line.type === "header" && "text-muted-foreground",
                  line.type === "context" && "text-foreground/70")}>
                {line.text || " "}
              </div>
            ))}
          </pre>
        </div>
      ))}
    </div>
  );
}
