import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import type { ReconArtifactRef } from "@/lib/generated/ReconArtifactRef";

export function applyOrganizationReconEvent(
  current: OrganizationReconRunSnapshot | undefined,
  incoming: OrganizationReconRunSnapshot
): OrganizationReconRunSnapshot {
  if (current && current.runId !== incoming.runId) return current;
  return incoming;
}

export function isOrganizationReconRunning(run: OrganizationReconRunSnapshot | undefined): boolean {
  return run?.status === "queued" || run?.status === "running";
}

export function findReconAssetsWorkbook(
  run: OrganizationReconRunSnapshot | undefined
): ReconArtifactRef | undefined {
  if (!run || isOrganizationReconRunning(run)) return undefined;
  const processingTask = run.tasks.find((task) => task.stage === "processing");
  if (
    !processingTask ||
    (processingTask.status !== "completed" && processingTask.status !== "checked_empty")
  ) {
    return undefined;
  }
  return processingTask.artifacts.find(
    (artifact) => artifact.kind === "asset_workbook" && artifact.path.endsWith("recon-assets.xlsx")
  );
}

export function suggestedReconAssetsFilename(orgName: string, runId?: string): string {
  const stem =
    orgName
      .trim()
      .replace(/[\\/:*?"<>|]/g, "_")
      .slice(0, 80) || "organization";
  const runSuffix = runId ? `-${runId.slice(0, 8)}` : "";
  return `${stem}-recon-assets${runSuffix}.xlsx`;
}

export function canExportCurrentReconAssets(hydrateStatus?: string): boolean {
  return hydrateStatus === "completed" || hydrateStatus === "partial";
}
