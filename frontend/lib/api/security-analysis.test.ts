import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@/lib/api/client";
import {
  apiEndpointsList,
  fingerprintsList,
  jsAnalysisList,
  normalizeBackendSurfaceHierarchy,
  normalizeCapturePayload,
  oplogListByTarget,
  targetAssetsList,
} from "./security-analysis";

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
          file_path: ".golish/captures/example.com/443/js/app.js",
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
        filePath: ".golish/captures/example.com/443/js/app.js",
        endpointsFound: ["/api"],
        secretsFound: ["token"],
        sourceMaps: true,
        riskSummary: "review",
        analyzedAt: "2026-06-30T00:00:02Z",
      }),
    ]);
  });

  it("normalizes legacy fingerprint evidence objects into observation arrays", async () => {
    const evidence = {
      source: "whatweb",
      origin: "https://app.example.test:443",
      technology: "React",
    };
    mockInvoke.mockResolvedValueOnce([
      {
        id: "fingerprint-1",
        target_id: "target-1",
        category: "technology",
        name: "React",
        confidence: 0.7,
        evidence,
        source: "whatweb",
      },
    ]);

    await expect(fingerprintsList("target-1")).resolves.toEqual([
      expect.objectContaining({
        id: "fingerprint-1",
        targetId: "target-1",
        evidence: [evidence],
      }),
    ]);
  });

  it("keeps evidence-ledger outcome fields used by Target WhatWeb status", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: 29047,
        target_id: "target-1",
        audit_role: "evidence",
        evidence_technique: "GOLISH-EAS-WEB-FINGERPRINT",
        evidence_outcome: "blocked",
        evidence_asset: "http://123.6.40.244:8000",
        detail: {},
        created_at: 1,
      },
    ]);

    await expect(oplogListByTarget("target-1")).resolves.toEqual([
      expect.objectContaining({
        auditRole: "evidence",
        evidenceTechnique: "GOLISH-EAS-WEB-FINGERPRINT",
        evidenceOutcome: "blocked",
        evidenceAsset: "http://123.6.40.244:8000",
      }),
    ]);
  });

  it("normalizes crawl observations on backend Web Origins", () => {
    const hierarchy = normalizeBackendSurfaceHierarchy({
      root_target: {
        id: "target-ip",
        target_type: "ip",
      },
      mode: "ip",
      data_source: "backend_identity",
      web_origins: [
        {
          id: "wo-a",
          scheme: "https",
          host: "a.example.com",
          host_type: "domain",
          port: 443,
          origin: "https://a.example.com:443",
          counts: {},
          content_counts: {},
          refs: [],
          crawl_observations: [
            {
              id: "crawl-1",
              origin_target_id: "target-domain",
              origin_url: "https://a.example.com:443",
              origin_key: "https://a.example.com:443",
              observed_url: "https://cdn.example.net/lib.js",
              observed_host: "cdn.example.net",
              observed_path: "/lib.js",
              source_tool: "katana",
              discovered_at: 10,
              updated_at: 11,
            },
          ],
        },
      ],
      summary: {},
      unassigned_web_data: {},
    });

    expect(hierarchy.webOrigins[0].crawlObservations).toEqual([
      expect.objectContaining({
        id: "crawl-1",
        originTargetId: "target-domain",
        observedUrl: "https://cdn.example.net/lib.js",
        observedHost: "cdn.example.net",
        kind: "url",
        sourceTool: "katana",
        discoveredAt: 10,
      }),
    ]);
  });
});

describe("normalizeCapturePayload", () => {
  it("normalizes a v2 capture (request headers/body present)", () => {
    const c = normalizeCapturePayload({
      version: 2,
      captured_at: "2026-06-30T00:00:00Z",
      request: {
        method: "post",
        url: "https://h/api/x",
        resource_type: "fetch",
        headers: { "content-type": "application/json" },
        body: '{"a":1}',
      },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        content_type: "application/json",
        body_text_sample: "{}",
        body_len: 2,
      },
    });
    expect(c.version).toBe(2);
    expect(c.request.method).toBe("POST");
    expect(c.request.headers["content-type"]).toBe("application/json");
    expect(c.request.body).toBe('{"a":1}');
    expect(c.response.status).toBe(200);
    expect(c.response.bodyTextSample).toBe("{}");
  });

  it("degrades a v1 capture (no request headers/body)", () => {
    const c = normalizeCapturePayload({
      version: 1,
      request: { method: "GET", url: "https://h/api/y" },
      response: { status: 204, headers: {}, body_text_sample: "" },
    });
    expect(c.version).toBe(1);
    expect(c.request.headers).toEqual({});
    expect(c.request.body).toBeNull();
    expect(c.response.status).toBe(204);
  });
});
