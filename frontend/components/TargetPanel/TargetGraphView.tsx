import { useCallback, useEffect, useRef, useState } from "react";
import { Network } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getProjectPath } from "@/lib/projects";
import { useGraphLayout } from "./hooks/useGraphLayout";
import { GraphSidebar, GraphNodeDetail } from "./GraphElements";
import { type Target } from "@/lib/pentest/types";

export function TargetGraphView({ targets }: { targets: Target[] }) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const [nodeFindings, setNodeFindings] = useState<{ id: string; title: string; severity: string; status: string }[]>([]);

  const {
    cyRef,
    selectedTarget,
    setSelectedTarget,
    focusNode,
  } = useGraphLayout(containerRef, targets);

  useEffect(() => {
    if (!selectedTarget) { setNodeFindings([]); return; }
    const host = selectedTarget.real_ip || selectedTarget.value;
    invoke<{ id: string; title: string; severity: string; status: string }[]>("findings_for_host", {
      host,
      projectPath: getProjectPath(),
    }).then(setNodeFindings).catch(() => setNodeFindings([]));
  }, [selectedTarget]);

  const handleNavigateChild = useCallback((child: Target) => {
    const cy = cyRef.current;
    if (cy) {
      const node = cy.getElementById(`target:${child.id}`);
      if (node.length > 0) {
        cy.stop();
        cy.animate({
          center: { eles: node },
          zoom: Math.max(cy.zoom(), 1.2),
        }, {
          duration: 200,
          easing: "ease-out-quad",
          complete: () => setSelectedTarget(child),
        });
        return;
      }
    }
    setSelectedTarget(child);
  }, [cyRef, setSelectedTarget]);

  return (
    <div className="relative h-full w-full overflow-hidden flex">
      <GraphSidebar
        targets={targets}
        selectedTarget={selectedTarget}
        onFocusNode={focusNode}
      />

      <div className="flex-1 min-w-0 relative">
        <div
          ref={containerRef}
          className="absolute inset-0"
          style={{
            background: "var(--background)",
            backgroundImage: "radial-gradient(circle, hsl(var(--muted-foreground) / 0.08) 1px, transparent 1px)",
            backgroundSize: "24px 24px",
          }}
        />

        {targets.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="flex flex-col items-center gap-3 text-muted-foreground/60">
              <Network className="w-12 h-12" />
              <p className="text-sm">{t("targets.noTargets")}</p>
            </div>
          </div>
        )}

        {selectedTarget && (
          <GraphNodeDetail
            target={selectedTarget}
            targets={targets}
            findings={nodeFindings}
            onClose={() => setSelectedTarget(null)}
            onNavigate={handleNavigateChild}
          />
        )}
      </div>
    </div>
  );
}
