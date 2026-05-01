import { Package } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { Artifact } from "@/lib/sidecar";
import { cn } from "@/lib/utils";

interface ArtifactsViewProps {
  artifacts: Artifact[];
  selectedArtifact: string | null;
  selectedArtifactData: Artifact | null;
  artifactPreview: string | null;
  onSelectArtifact: (filename: string | null) => void;
}

export function ArtifactsView({
  artifacts,
  selectedArtifact,
  selectedArtifactData,
  artifactPreview,
  onSelectArtifact,
}: ArtifactsViewProps) {
  if (artifacts.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="text-center text-muted-foreground">
          <Package className="w-8 h-8 mx-auto mb-2 opacity-50" />
          <p className="text-sm">No artifacts generated yet</p>
          <p className="text-xs mt-1">Documentation artifacts will appear here</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
      <div className="border-b border-border">
        <ScrollArea className="max-h-48">
          <div className="p-2 space-y-1">
            {artifacts.map((artifact) => (
              <ArtifactListItem
                key={artifact.filename}
                artifact={artifact}
                isSelected={selectedArtifact === artifact.filename}
                onSelect={() =>
                  onSelectArtifact(
                    selectedArtifact === artifact.filename ? null : artifact.filename
                  )
                }
              />
            ))}
          </div>
        </ScrollArea>
      </div>
      <div className="flex-1 min-h-0 overflow-hidden">
        {selectedArtifactData ? (
          <ArtifactDetail artifact={selectedArtifactData} preview={artifactPreview} />
        ) : (
          <div className="h-full flex items-center justify-center text-muted-foreground text-sm">
            Select an artifact to view details
          </div>
        )}
      </div>
    </div>
  );
}

function ArtifactListItem({
  artifact,
  isSelected,
  onSelect,
}: {
  artifact: Artifact;
  isSelected: boolean;
  onSelect: () => void;
}) {
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
        <div className="mt-0.5 p-1 rounded bg-[var(--ansi-cyan)]/20">
          <Package className="w-3 h-3 text-[var(--ansi-cyan)]" />
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-xs font-medium font-mono">{artifact.filename}</p>
          <p className="text-[10px] text-muted-foreground mt-0.5 truncate">
            → {artifact.meta.target}
          </p>
          <p className="text-[10px] text-muted-foreground">
            Based on {artifact.meta.based_on_patches.length} patch
            {artifact.meta.based_on_patches.length !== 1 ? "es" : ""}
          </p>
        </div>
      </div>
    </button>
  );
}

function ArtifactDetail({ artifact, preview }: { artifact: Artifact; preview: string | null }) {
  return (
    <ScrollArea className="h-full">
      <div className="p-3 space-y-3">
        <div>
          <span className="text-[10px] px-1.5 py-0.5 rounded font-medium bg-[var(--ansi-cyan)]/20 text-[var(--ansi-cyan)]">
            PENDING
          </span>
          <h3 className="text-sm font-medium font-mono mt-1">{artifact.filename}</h3>
        </div>
        <div>
          <p className="text-[10px] text-muted-foreground mb-1 font-medium">TARGET PATH</p>
          <code className="text-xs font-mono bg-muted px-2 py-1 rounded block">
            {artifact.meta.target}
          </code>
        </div>
        <div>
          <p className="text-[10px] text-muted-foreground mb-1 font-medium">REASON</p>
          <p className="text-xs">{artifact.meta.reason}</p>
        </div>
        <div>
          <p className="text-[10px] text-muted-foreground mb-1 font-medium">BASED ON PATCHES</p>
          <div className="flex flex-wrap gap-1">
            {artifact.meta.based_on_patches.map((id) => (
              <span key={id} className="text-[10px] font-mono bg-muted px-1.5 py-0.5 rounded">
                #{id}
              </span>
            ))}
          </div>
        </div>
        <div>
          <p className="text-[10px] text-muted-foreground mb-1 font-medium">CONTENT PREVIEW</p>
          {preview ? (
            <pre className="text-xs font-mono whitespace-pre-wrap bg-muted p-2 rounded overflow-x-auto">
              {preview}
            </pre>
          ) : (
            <div className="text-xs text-muted-foreground animate-pulse">Loading preview...</div>
          )}
        </div>
      </div>
    </ScrollArea>
  );
}
