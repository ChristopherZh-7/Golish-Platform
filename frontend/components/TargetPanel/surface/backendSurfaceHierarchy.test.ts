import { describe, expect, it } from "vitest";
import type { DirectoryEntry } from "@/lib/pentest/api";
import type { PortInfo, Target } from "@/lib/pentest/types";
import type {
  ApiEndpoint,
  BackendSurfaceHierarchyDto,
  BackendWebOriginContentCountsDto,
  BackendWebOriginContentRefDto,
  JsAnalysisResult,
} from "@/lib/security-analysis";
import { IP_SURFACE_TABS } from "../TargetSurfaceWorkbench";
import { composeBackendSurfaceHierarchy } from "./backendSurfaceHierarchy";
import { buildSurfaceHierarchy } from "./surfaceHierarchy";

const target = (p: Partial<Target> = {}): Target =>
  ({
    id: "target-ip",
    name: "1.2.3.4",
    type: "ip",
    value: "1.2.3.4",
    tags: [],
    notes: "",
    scope: "in",
    status: "active",
    grp: "",
    owner: "",
    time_window_start: null,
    time_window_end: null,
    organization_id: null,
    source: "manual",
    parent_id: null,
    real_ip: "",
    cdn_waf: "",
    http_title: "",
    http_status: null,
    webserver: "",
    os_info: "",
    content_type: "",
    created_at: 0,
    updated_at: 0,
    ports: [],
    technologies: [],
    ...p,
  }) as Target;

const port = (p: Partial<PortInfo>): PortInfo => p as PortInfo;

const endpoint = (p: Partial<ApiEndpoint>): ApiEndpoint => ({
  id: "api-1",
  targetId: "target-domain",
  projectPath: null,
  url: "",
  method: "GET",
  path: "",
  params: [],
  headers: {},
  authType: null,
  responseType: null,
  statusCode: null,
  notes: "",
  source: "js_analysis",
  riskLevel: "info",
  tested: false,
  capturePath: null,
  discoveredAt: "2026-07-02T00:00:00Z",
  updatedAt: "2026-07-02T00:00:00Z",
  ...p,
});

const jsResult = (p: Partial<JsAnalysisResult>): JsAnalysisResult => ({
  id: "js-1",
  targetId: "target-domain",
  projectPath: null,
  url: "",
  filename: "",
  filePath: null,
  sizeBytes: null,
  hashSha256: null,
  frameworks: [],
  libraries: [],
  endpointsFound: [],
  secretsFound: [],
  comments: [],
  sourceMaps: false,
  riskSummary: "",
  rawAnalysis: {},
  analyzedAt: "2026-07-02T00:00:00Z",
  ...p,
});

const directoryEntry = (p: Partial<DirectoryEntry>): DirectoryEntry => ({
  id: "dir-1",
  target_id: "target-domain",
  url: "",
  status_code: null,
  content_length: null,
  lines: null,
  words: null,
  content_type: "",
  tool: "route_probe_paths",
  created_at: 0,
  ...p,
});

function backendSummary(
  p: Partial<BackendSurfaceHierarchyDto["summary"]> = {}
): BackendSurfaceHierarchyDto["summary"] {
  return {
    endpointCount: 0,
    webOriginCount: 0,
    observationCount: 0,
    inferredObservationCount: 0,
    confirmedObservationCount: 0,
    relatedDomainCount: 0,
    unassignedCount: 0,
    urlCount: null,
    apiCount: null,
    jsCount: null,
    paramCount: null,
    directoryEntryCount: null,
    passiveLogCount: null,
    evidenceCount: null,
    contentUnassignedCount: null,
    contentUnmatchedOriginCount: null,
    ...p,
  };
}

function backendHierarchy(
  p: Partial<BackendSurfaceHierarchyDto> = {}
): BackendSurfaceHierarchyDto {
  const { summary, unassignedWebData, ...rest } = p;
  return {
    rootTarget: {
      id: "target-ip",
      name: "1.2.3.4",
      targetType: "ip",
      value: "1.2.3.4",
      tags: [],
      notes: "",
      scope: "in",
      group: "",
      projectPath: "proj",
      organizationId: null,
      createdAt: 0,
      updatedAt: 0,
    },
    mode: "ip",
    dataSource: "backend_identity",
    generatedAt: 0,
    endpoints: [],
    webOrigins: [],
    observations: [],
    relatedDomains: [],
    unassignedWebData: unassignedWebData ?? {
      urls: [],
      apis: [],
      js: [],
      params: [],
      reason: "",
      counts: null,
      refs: [],
    },
    summary: backendSummary(summary),
    fallbackReason: null,
    ...rest,
  };
}

const contentCounts = (
  p: Partial<BackendWebOriginContentCountsDto> = {}
): BackendWebOriginContentCountsDto => ({
  urlCount: 0,
  apiCount: 0,
  jsCount: 0,
  paramCount: 0,
  directoryEntryCount: 0,
  passiveLogCount: 0,
  evidenceCount: 0,
  ...p,
});

function backendEndpoint(id = "ne-443", originIds = ["wo-a"]) {
  return {
    id,
    ip: "1.2.3.4",
    port: 443,
    transport: "tcp",
    state: "open",
    serviceName: "https",
    serviceProduct: null,
    serviceVersion: null,
    banner: null,
    tlsDetected: true,
    source: "backfill",
    confidence: 0.9,
    firstSeenAt: 1,
    lastSeenAt: 2,
    lastConfirmedAt: 2,
    webOriginIds: originIds,
    observationIds: ["obs-1"],
  };
}

function backendOrigin(
  id = "wo-a",
  origin = "https://a.example.com:443",
  portNumber = 443,
  content: BackendWebOriginContentCountsDto | null = null,
  refs: BackendWebOriginContentRefDto[] = []
) {
  return {
    id,
    scheme: "https",
    host: origin.replace(/^https:\/\//, "").replace(/:\d+$/, ""),
    hostType: "domain",
    port: portNumber,
    origin,
    source: "backfill",
    confidence: 0.9,
    firstSeenAt: 1,
    lastSeenAt: 2,
    lastConfirmedAt: 2,
    endpointIds: ["ne-443"],
    observationIds: ["obs-1"],
    counts: { endpointCount: 1, observationCount: 1 },
    contentCounts: content,
    refs,
  };
}

const contentRef = (p: Partial<BackendWebOriginContentRefDto> = {}): BackendWebOriginContentRefDto => ({
  kind: "api",
  id: "ref-1",
  url: "https://a.example.com/api/login",
  method: "GET",
  statusCode: 200,
  capturePath: null,
  source: "api_endpoint",
  ...p,
});

function backendObservation() {
  return {
    id: "obs-1",
    webOriginId: "wo-a",
    networkEndpointId: "ne-443",
    targetId: "target-ip",
    observedIp: "1.2.3.4",
    sni: "a.example.com",
    hostHeader: "a.example.com",
    statusCode: 200,
    title: "OK",
    finalUrl: "https://a.example.com/",
    redirectChain: [],
    bodyHash: null,
    faviconHash: null,
    screenshotPath: null,
    capturePath: null,
    observedAt: 3,
    confidence: 0.9,
    source: "httpx",
  };
}

describe("composeBackendSurfaceHierarchy", () => {
  it("merges backend identity with frontend content on the same origin key", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [
        endpoint({
          id: "api-a",
          url: "https://a.example.com/api/login",
          path: "/api/login",
          params: ["token"],
        }),
      ],
      jsResults: [
        jsResult({
          id: "js-a",
          url: "https://a.example.com/static/app.js",
          filename: "static/app.js",
        }),
      ],
      directoryEntries: [directoryEntry({ id: "dir-a", url: "https://a.example.com/admin" })],
    });

    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
        observations: [backendObservation()],
      })
    );

    expect(result.status).toBe("backend_identity");
    expect(result.hierarchy.endpoints[0]).toMatchObject({ id: "ne-443", source: "backend_identity" });
    expect(result.hierarchy.observations).toHaveLength(1);
    expect(result.hierarchy.webOrigins).toHaveLength(1);
    expect(result.hierarchy.webOrigins[0]).toMatchObject({
      origin: "https://a.example.com:443",
      source: "backend_identity",
      backendIdentityOnly: false,
    });
    expect(result.hierarchy.webOrigins[0].counts).toMatchObject({
      apis: 1,
      js: 1,
      params: 1,
      directoryEntries: 1,
    });
  });

  it("keeps a backend origin when legacy frontend content has not linked to it yet", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target() });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
      })
    );

    expect(result.hierarchy.webOrigins.map((origin) => origin.origin)).toEqual([
      "https://a.example.com:443",
    ]);
    expect(result.hierarchy.webOrigins[0].backendIdentityOnly).toBe(true);
    expect(result.hierarchy.webOrigins[0].counts).toMatchObject({ urls: 0, apis: 0, js: 0 });
  });

  it("preserves a frontend-only origin as frontend_inferred", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [endpoint({ id: "api-b", url: "https://b.example.com/api" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
      })
    );

    const frontendOnly = result.hierarchy.webOrigins.find(
      (origin) => origin.origin === "https://b.example.com:443"
    );
    expect(frontendOnly).toMatchObject({
      source: "frontend_inferred",
      confidence: "inferred",
    });
    expect(frontendOnly?.counts.apis).toBe(1);
  });

  it("falls back to frontend hierarchy when backend is unavailable", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target(),
      apiEndpoints: [endpoint({ id: "api-b", url: "https://b.example.com/api" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        dataSource: "backend_unavailable",
        fallbackReason: "Backend unavailable.",
      })
    );

    expect(result.status).toBe("fallback");
    expect(result.fallbackReason).toBe("Backend unavailable.");
    expect(result.hierarchy.webOrigins[0]).toMatchObject({
      origin: "https://b.example.com:443",
      source: "frontend_inferred",
    });
  });

  it("does not apply backend legacy mode to a domain target", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target({ type: "domain", value: "a.example.com" }) });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({ mode: "legacy", dataSource: "legacy_fallback" })
    );

    expect(result.status).toBe("fallback");
    expect(result.hierarchy.mode).toBe("domain");
    expect(result.hierarchy.dataSource).toBe("frontend_inferred");
  });

  it("keeps IP-literal Web Origins out of Related Domains", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target({ value: "1.1.1.1" }) });
    const ipOrigin = {
      ...backendOrigin("wo-ip", "https://1.1.1.1:443"),
      host: "1.1.1.1",
      hostType: "ip",
    };
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [{ ...backendEndpoint("ne-ip", ["wo-ip"]), ip: "1.1.1.1", webOriginIds: ["wo-ip"] }],
        webOrigins: [ipOrigin],
      })
    );

    expect(result.hierarchy.webOrigins.map((origin) => origin.origin)).toEqual([
      "https://1.1.1.1:443",
    ]);
    expect(result.hierarchy.relatedDomains).toEqual([]);
  });

  it("merges by exact origin key and does not duplicate the same origin", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target(),
      apiEndpoints: [endpoint({ id: "api-a", url: "https://a.example.com/api" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
      })
    );

    expect(
      result.hierarchy.webOrigins.filter((origin) => origin.origin === "https://a.example.com:443")
    ).toHaveLength(1);
  });

  it("does not merge explicit ports for the same host", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target(),
      apiEndpoints: [endpoint({ id: "api-8443", url: "https://a.example.com:8443/api" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
      })
    );

    expect(result.hierarchy.webOrigins.map((origin) => origin.origin)).toEqual([
      "https://a.example.com:443",
      "https://a.example.com:8443",
    ]);
  });
});

describe("Phase 2.5B backend content counts", () => {
  it("A: uses backend contentCounts for display counts while rows stay frontend", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [
        endpoint({ id: "api-a", url: "https://a.example.com/api/login", path: "/api/login" }),
      ],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [
          backendOrigin(
            "wo-a",
            "https://a.example.com:443",
            443,
            contentCounts({ apiCount: 2, urlCount: 2 })
          ),
        ],
        summary: backendSummary({ apiCount: 2, urlCount: 2 }),
      })
    );

    const origin = result.hierarchy.webOrigins.find(
      (item) => item.origin === "https://a.example.com:443"
    );
    expect(origin?.counts.apis).toBe(2);
    expect(origin?.contentCountSource).toBe("backend_content_counts");
    // Detail rows remain the single frontend-inferred API row, never fabricated.
    expect(origin?.apiEndpoints).toHaveLength(1);
    expect(result.hierarchy.summary.apiCount).toBe(2);
    expect(result.hierarchy.summary.contentCountSource).toBe("backend_content_counts");
  });

  it("B: falls back to frontend counts when backend omits contentCounts", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [endpoint({ id: "api-a", url: "https://a.example.com/api" })],
      jsResults: [jsResult({ id: "js-a", url: "https://a.example.com/app.js" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
      })
    );

    const origin = result.hierarchy.webOrigins.find(
      (item) => item.origin === "https://a.example.com:443"
    );
    expect(origin?.contentCountSource).toBe("frontend_content_inferred");
    expect(origin?.counts.apis).toBe(1);
    expect(origin?.counts.js).toBe(1);
  });

  it("C: keeps a backend origin with contentCounts but no frontend rows and flags it", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target() });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [
          backendOrigin(
            "wo-a",
            "https://a.example.com:443",
            443,
            contentCounts({ apiCount: 1, urlCount: 1 })
          ),
        ],
        summary: backendSummary({ apiCount: 1, urlCount: 1 }),
      })
    );

    const origin = result.hierarchy.webOrigins[0];
    expect(origin.origin).toBe("https://a.example.com:443");
    expect(origin.counts.apis).toBe(1);
    expect(origin.apiEndpoints).toHaveLength(0);
    expect(origin.contentCountSource).toBe("backend_content_counts");
    expect(origin.backendIdentityOnly).toBe(true);
  });

  it("D: frontend-only origin uses frontend counts and frontend_content_inferred", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [endpoint({ id: "api-b", url: "https://b.example.com/api" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [
          backendOrigin("wo-a", "https://a.example.com:443", 443, contentCounts({ apiCount: 5 })),
        ],
        summary: backendSummary({ apiCount: 5 }),
      })
    );

    const frontendOnly = result.hierarchy.webOrigins.find(
      (item) => item.origin === "https://b.example.com:443"
    );
    expect(frontendOnly?.source).toBe("frontend_inferred");
    expect(frontendOnly?.contentCountSource).toBe("frontend_content_inferred");
    expect(frontendOnly?.counts.apis).toBe(1);
  });

  it("E: backend unavailable keeps frontend inferred content source", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target(),
      apiEndpoints: [endpoint({ id: "api-b", url: "https://b.example.com/api" })],
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({ dataSource: "backend_unavailable", fallbackReason: "boom" })
    );

    expect(result.status).toBe("fallback");
    expect(result.hierarchy.summary.contentCountSource).toBe("frontend_content_inferred");
    expect(result.hierarchy.webOrigins[0].contentCountSource).toBe("frontend_content_inferred");
  });

  it("F: exposes backend unassigned/unmatched counts without materializing origins", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target() });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [
          backendOrigin("wo-a", "https://a.example.com:443", 443, contentCounts({ apiCount: 1 })),
        ],
        summary: backendSummary({
          apiCount: 1,
          contentUnassignedCount: 3,
          contentUnmatchedOriginCount: 2,
        }),
      })
    );

    expect(result.hierarchy.summary.contentUnassignedCount).toBe(3);
    expect(result.hierarchy.summary.contentUnmatchedOriginCount).toBe(2);
    expect(result.hierarchy.webOrigins.map((item) => item.origin)).toEqual([
      "https://a.example.com:443",
    ]);
  });

  it("G: IP-literal backend origin with contentCounts shows in Web Origins, not Related Domains", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target({ value: "1.1.1.1" }) });
    const ipOrigin = {
      ...backendOrigin(
        "wo-ip",
        "https://1.1.1.1:443",
        443,
        contentCounts({ apiCount: 1, urlCount: 1 })
      ),
      host: "1.1.1.1",
      hostType: "ip",
    };
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [
          { ...backendEndpoint("ne-ip", ["wo-ip"]), ip: "1.1.1.1", webOriginIds: ["wo-ip"] },
        ],
        webOrigins: [ipOrigin],
        summary: backendSummary({ apiCount: 1, urlCount: 1 }),
      })
    );

    const origin = result.hierarchy.webOrigins.find(
      (item) => item.origin === "https://1.1.1.1:443"
    );
    expect(origin?.counts.apis).toBe(1);
    expect(origin?.contentCountSource).toBe("backend_content_counts");
    expect(result.hierarchy.relatedDomains).toEqual([]);
  });

  it("H: explicit ports keep separate backend contentCounts", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target() });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [
          backendOrigin("wo-443", "https://a.example.com:443", 443, contentCounts({ apiCount: 1 })),
          backendOrigin(
            "wo-8443",
            "https://a.example.com:8443",
            8443,
            contentCounts({ apiCount: 2 })
          ),
        ],
        summary: backendSummary({ apiCount: 3 }),
      })
    );

    const byOrigin = new Map(
      result.hierarchy.webOrigins.map((item) => [item.origin, item.counts.apis])
    );
    expect(byOrigin.get("https://a.example.com:443")).toBe(1);
    expect(byOrigin.get("https://a.example.com:8443")).toBe(2);
  });

  it("J: domain target ignores backend contentCounts and stays frontend inferred", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ type: "domain", value: "a.example.com" }),
    });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        mode: "legacy",
        dataSource: "legacy_fallback",
        summary: backendSummary({ apiCount: 9 }),
      })
    );

    expect(result.status).toBe("fallback");
    expect(result.hierarchy.mode).toBe("domain");
    expect(result.hierarchy.summary.contentCountSource).toBe("frontend_content_inferred");
    expect(result.hierarchy.summary.apiCount).toBe(frontend.summary.apiCount);
  });
});

describe("Phase 2.5C lightweight legacy refs", () => {
  it("maps backend refs onto the WebOrigin as contentRefs", () => {
    const frontend = buildSurfaceHierarchy({ rootTarget: target() });
    const result = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [
          backendOrigin("wo-a", "https://a.example.com:443", 443, contentCounts({ apiCount: 1 }), [
            contentRef({ id: "api-ref-1", url: "https://a.example.com/api/login" }),
          ]),
        ],
        summary: backendSummary({ apiCount: 1 }),
      })
    );

    const origin = result.hierarchy.webOrigins.find(
      (item) => item.origin === "https://a.example.com:443"
    );
    expect(origin?.contentRefs).toHaveLength(1);
    expect(origin?.contentRefs[0]).toMatchObject({
      kind: "api",
      url: "https://a.example.com/api/login",
      method: "GET",
    });
    // Refs must NOT be promoted into full frontend rows.
    expect(origin?.apiEndpoints).toHaveLength(0);
  });

  it("leaves contentRefs empty for frontend-only and fallback origins", () => {
    const frontend = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [endpoint({ id: "api-b", url: "https://b.example.com/api" })],
    });
    const merged = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({
        endpoints: [backendEndpoint()],
        webOrigins: [backendOrigin()],
      })
    );
    const frontendOnly = merged.hierarchy.webOrigins.find(
      (item) => item.origin === "https://b.example.com:443"
    );
    expect(frontendOnly?.contentRefs).toEqual([]);

    const fallback = composeBackendSurfaceHierarchy(
      frontend,
      backendHierarchy({ dataSource: "backend_unavailable", fallbackReason: "boom" })
    );
    for (const origin of fallback.hierarchy.webOrigins) {
      expect(origin.contentRefs).toEqual([]);
    }
  });
});

describe("IP surface tabs", () => {
  it("keeps IP targets on the aggregate tab set without a global Sitemap tab", () => {
    expect(IP_SURFACE_TABS.map((tab) => tab.id)).toEqual([
      "overview",
      "endpoints",
      "origins",
      "domains",
      "sensitive",
      "evidence",
    ]);
    expect(IP_SURFACE_TABS.map((tab) => String(tab.id)).includes("sitemap")).toBe(false);
  });
});
