import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const eventMocks = vi.hoisted(() => ({
  onCustomEvent: vi.fn(async () => vi.fn()),
  onEvent: vi.fn(async () => vi.fn()),
}));

vi.mock("@/lib/events", () => eventMocks);

vi.mock("@/lib/pentest/api", () => ({
  listDirectoryEntries: vi.fn(),
}));

vi.mock("@/lib/security-analysis", () => ({
  apiEndpointsList: vi.fn(),
  fingerprintsList: vi.fn(),
  jsAnalysisList: vi.fn(),
  oplogListByTarget: vi.fn(),
  passiveScansList: vi.fn(),
  targetAssetsList: vi.fn(),
  targetSurfaceHierarchyGet: vi.fn(),
  targetTimeline: vi.fn(),
}));

import { listDirectoryEntries } from "@/lib/pentest/api";
import {
  apiEndpointsList,
  fingerprintsList,
  jsAnalysisList,
  oplogListByTarget,
  passiveScansList,
  targetAssetsList,
  targetSurfaceHierarchyGet,
  targetTimeline,
} from "@/lib/security-analysis";
import { useTargetSurfaceData } from "./useTargetSurfaceData";

const mockListDirectoryEntries = vi.mocked(listDirectoryEntries);
const mockApiEndpointsList = vi.mocked(apiEndpointsList);
const mockFingerprintsList = vi.mocked(fingerprintsList);
const mockJsAnalysisList = vi.mocked(jsAnalysisList);
const mockOplogListByTarget = vi.mocked(oplogListByTarget);
const mockPassiveScansList = vi.mocked(passiveScansList);
const mockTargetAssetsList = vi.mocked(targetAssetsList);
const mockTargetSurfaceHierarchyGet = vi.mocked(targetSurfaceHierarchyGet);
const mockTargetTimeline = vi.mocked(targetTimeline);

describe("useTargetSurfaceData", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockTargetAssetsList.mockResolvedValue([]);
    mockApiEndpointsList.mockResolvedValue([]);
    mockFingerprintsList.mockResolvedValue([]);
    mockJsAnalysisList.mockResolvedValue([]);
    mockPassiveScansList.mockResolvedValue([]);
    mockTargetTimeline.mockResolvedValue([]);
    mockListDirectoryEntries.mockResolvedValue([]);
    mockOplogListByTarget.mockResolvedValue([]);
  });

  it("keeps successful fingerprints and backend hierarchy when directory entries fail", async () => {
    const fingerprint = {
      id: "fingerprint-1",
      targetId: "target-1",
      projectPath: "/workspace",
      category: "technology",
      name: "nginx",
      version: null,
      confidence: 0.8,
      evidence: [],
      cpe: null,
      source: "WhatWeb",
      detectedAt: "2026-07-12T00:00:00Z",
    };
    const hierarchy = {
      mode: "ip",
      dataSource: "backend_identity",
      endpoints: [{ id: "endpoint-1" }],
      webOrigins: [],
    };
    mockFingerprintsList.mockResolvedValue([fingerprint]);
    mockListDirectoryEntries.mockRejectedValue(
      new Error('directory_entry_list: column reference "id" is ambiguous')
    );
    mockTargetSurfaceHierarchyGet.mockResolvedValue(hierarchy as never);
    const relatedTargetIds: string[] = [];
    const options = { loadBackendHierarchy: true };

    const { result } = renderHook(() =>
      useTargetSurfaceData("target-1", relatedTargetIds, options)
    );

    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.data.fingerprints).toEqual([fingerprint]);
    expect(result.current.data.directoryEntries).toEqual([]);
    expect(result.current.backendHierarchy).toBe(hierarchy);
    expect(result.current.backendHierarchyStatus).toBe("success");
    expect(result.current.error).toContain("directoryEntries (target-1)");
    expect(result.current.error).toContain('column reference "id" is ambiguous');
    expect(result.current.sourceErrors).toEqual([
      expect.objectContaining({
        targetId: "target-1",
        source: "directoryEntries",
      }),
    ]);
  });

  it("keeps the no-target state empty without an error", () => {
    const { result } = renderHook(() => useTargetSurfaceData(null));

    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
    expect(result.current.sourceErrors).toEqual([]);
    expect(result.current.data).toEqual({
      assets: [],
      endpoints: [],
      fingerprints: [],
      jsResults: [],
      passiveScans: [],
      timeline: [],
      directoryEntries: [],
      logs: [],
    });
  });

  it("keeps reload callbacks and Tauri listeners stable for an equivalent target-id set", async () => {
    const { rerender, result } = renderHook(
      ({ relatedTargetIds }: { relatedTargetIds: string[] }) =>
        useTargetSurfaceData("target-1", relatedTargetIds),
      { initialProps: { relatedTargetIds: ["target-3", "target-2"] } }
    );

    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect(eventMocks.onEvent).toHaveBeenCalledTimes(1));
    expect(eventMocks.onCustomEvent).toHaveBeenCalledTimes(1);
    const loadsBeforeEquivalentRender = mockTargetAssetsList.mock.calls.length;

    rerender({ relatedTargetIds: ["target-2", "target-3", "target-2"] });

    expect(eventMocks.onEvent).toHaveBeenCalledTimes(1);
    expect(eventMocks.onCustomEvent).toHaveBeenCalledTimes(1);
    expect(mockTargetAssetsList).toHaveBeenCalledTimes(loadsBeforeEquivalentRender);
  });
});
