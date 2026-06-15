import type { AssetIntelHydrateConfig } from "@/lib/generated/AssetIntelHydrateConfig";
import type { OrganizationReconEvent } from "@/lib/generated/OrganizationReconEvent";
import type { OrganizationReconExportResult } from "@/lib/generated/OrganizationReconExportResult";
import type { OrganizationReconRunSnapshot } from "@/lib/generated/OrganizationReconRunSnapshot";
import type { OrganizationReconStartArgs as GeneratedOrganizationReconStartArgs } from "@/lib/generated/OrganizationReconStartArgs";
import { listen } from "@/lib/tauri-listen";
import { invoke } from "./client";

export const ORGANIZATION_RECON_EVENT = "organization-recon:event";

export type { OrganizationReconRunSnapshot } from "@/lib/generated/OrganizationReconRunSnapshot";

export interface OrganizationReconStartArgs {
  organizationId: string;
  providerIds?: string[];
  config?: AssetIntelHydrateConfig;
  allowExternal?: boolean;
  allowActive?: boolean;
}

export async function startRun(
  args: OrganizationReconStartArgs
): Promise<OrganizationReconRunSnapshot> {
  const request = {
    organizationId: args.organizationId,
    providerIds: args.providerIds ?? [],
    config: args.config ?? {},
    allowExternal: args.allowExternal ?? false,
    allowActive: args.allowActive ?? false,
  } satisfies GeneratedOrganizationReconStartArgs;
  return invoke<OrganizationReconRunSnapshot>("organization_recon_start_run", {
    args: request,
  });
}

export async function getRun(runId: string): Promise<OrganizationReconRunSnapshot> {
  return invoke<OrganizationReconRunSnapshot>("organization_recon_get_run", { runId });
}

export async function exportAssets(
  runId: string,
  outputPath: string
): Promise<OrganizationReconExportResult> {
  return invoke<OrganizationReconExportResult>("organization_recon_export_assets", {
    runId,
    outputPath,
  });
}

export async function exportCurrentAssets(
  organizationId: string,
  outputPath: string
): Promise<OrganizationReconExportResult> {
  return invoke<OrganizationReconExportResult>("organization_recon_export_current_assets", {
    organizationId,
    outputPath,
  });
}

/**
 * Backfill `targets.real_ip` from existing `dns_records` A answers (no new DNS
 * resolution). One-off migration aid + manual refresh for the IP-centric host
 * tree. Returns the number of target rows updated. Omit `projectPath` for all.
 */
export async function backfillRealIp(projectPath?: string): Promise<number> {
  return invoke<number>("recon_backfill_real_ip", {
    projectPath: projectPath ?? null,
  });
}

export async function listenStream(
  onEvent: (run: OrganizationReconRunSnapshot) => void
): Promise<() => void> {
  const unlisten = await listen<OrganizationReconEvent>(ORGANIZATION_RECON_EVENT, (envelope) =>
    onEvent(envelope.payload.run)
  );
  return unlisten;
}
