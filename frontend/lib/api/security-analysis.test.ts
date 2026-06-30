import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@/lib/api/client";
import { apiEndpointsList, jsAnalysisList, targetAssetsList } from "./security-analysis";

vi.mock("@/lib/api/client", () => ({
  invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("security-analysis api normalization", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("normalizes snake_case target asset rows returned by Tauri", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "asset-1",
        target_id: "target-1",
        project_path: "/tmp/ws",
        asset_type: "sitemap_url",
        value: "https://example.com/admin",
        metadata: { content_type: "text/html" },
        discovered_at: "2026-06-30T00:00:00Z",
        updated_at: "2026-06-30T00:00:01Z",
      },
    ]);

    await expect(targetAssetsList("target-1")).resolves.toEqual([
      expect.objectContaining({
        id: "asset-1",
        targetId: "target-1",
        projectPath: "/tmp/ws",
        assetType: "sitemap_url",
        value: "https://example.com/admin",
        metadata: { content_type: "text/html" },
        discoveredAt: "2026-06-30T00:00:00Z",
        updatedAt: "2026-06-30T00:00:01Z",
      }),
    ]);
  });

  it("normalizes endpoint and JS analysis rows used by Target Surface", async () => {
    mockInvoke
      .mockResolvedValueOnce([
        {
          id: "endpoint-1",
          target_id: "target-1",
          url: "https://example.com/api",
          method: "GET",
          path: "/api",
          params: ["q"],
          response_type: "application/json",
          status_code: 200,
          capture_path: ".golish/captures/example.com/api/search.http",
          risk_level: "low",
          discovered_at: "2026-06-30T00:00:00Z",
          updated_at: "2026-06-30T00:00:01Z",
        },
      ])
      .mockResolvedValueOnce([
        {
          id: "js-1",
          target_id: "target-1",
          url: "https://example.com/app.js",
          filename: "app.js",
          size_bytes: 42,
          hash_sha256: "abc",
          endpoints_found: ["/api"],
          secrets_found: ["token"],
          source_maps: true,
          risk_summary: "review",
          raw_analysis: { ok: true },
          analyzed_at: "2026-06-30T00:00:02Z",
        },
      ]);

    await expect(apiEndpointsList("target-1")).resolves.toEqual([
      expect.objectContaining({
        targetId: "target-1",
        responseType: "application/json",
        statusCode: 200,
        capturePath: ".golish/captures/example.com/api/search.http",
        riskLevel: "low",
      }),
    ]);
    await expect(jsAnalysisList("target-1")).resolves.toEqual([
      expect.objectContaining({
        targetId: "target-1",
        sizeBytes: 42,
        hashSha256: "abc",
        endpointsFound: ["/api"],
        secretsFound: ["token"],
        sourceMaps: true,
        riskSummary: "review",
        analyzedAt: "2026-06-30T00:00:02Z",
      }),
    ]);
  });
});
