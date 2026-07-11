import { describe, expect, it } from "vitest";
import type { PortInfo } from "@/lib/pentest/types";
import type { ApiEndpoint, JsAnalysisResult } from "@/lib/security-analysis";
import { getEndpointParamNames } from "./endpointParams";
import {
  buildSitemapItems,
  buildSitemapJsSources,
  buildSitemapTree,
  formatTime,
  isHttpPort,
} from "./surfaceModel";

const port = (p: Partial<PortInfo>): PortInfo => p as PortInfo;
const endpoint = (p: Partial<ApiEndpoint>): ApiEndpoint => ({
  id: "endpoint",
  targetId: "target-1",
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
  source: "",
  riskLevel: "info",
  tested: false,
  capturePath: null,
  discoveredAt: "2026-06-30T00:00:00Z",
  updatedAt: "2026-06-30T00:00:00Z",
  ...p,
});
const jsResult = (p: Partial<JsAnalysisResult>): JsAnalysisResult => ({
  id: "js-1",
  targetId: "target-1",
  projectPath: null,
  url: "https://example.com/static/app.js",
  filename: "static/app.js",
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
  analyzedAt: "2026-06-30T00:00:00Z",
  ...p,
});

describe("isHttpPort", () => {
  it("detects http via service name", () => {
    expect(isHttpPort(port({ service: "http" }))).toBe(true);
    expect(isHttpPort(port({ service: "https" }))).toBe(true);
  });

  it("detects http via status or title even when service is not http", () => {
    expect(isHttpPort(port({ service: "ssh", http_status: 200 }))).toBe(true);
    expect(isHttpPort(port({ service: "ssh", http_title: "Home" }))).toBe(true);
  });

  it("returns false for non-web services", () => {
    expect(isHttpPort(port({ service: "ssh" }))).toBe(false);
    expect(isHttpPort(port({}))).toBe(false);
  });
});

describe("formatTime", () => {
  it("echoes the raw value when unparseable", () => {
    expect(formatTime("not-a-date")).toBe("not-a-date");
  });

  it("formats a parseable ISO string into a clock time", () => {
    // locale-tolerant: assert HH:MM:SS appears (AM/PM suffix allowed)
    expect(formatTime("2026-01-01T13:05:09Z")).toMatch(/\d{2}:\d{2}:\d{2}/);
  });

  it("accepts epoch millis", () => {
    expect(formatTime(Date.parse("2026-01-01T13:05:09Z"))).toMatch(/\d{2}:\d{2}:\d{2}/);
  });
});

describe("endpoint params", () => {
  it("normalizes persisted api_endpoints params into stable names", () => {
    expect(
      getEndpointParamNames([
        " q ",
        "page",
        "q",
        { name: "bodyToken" },
        { parameter: "fallback" },
        { nope: true },
      ])
    ).toEqual(["bodyToken", "fallback", "page", "q"]);
  });
});

describe("buildSitemapItems", () => {
  it("includes JS-derived API endpoints (tagged endpoint) and excludes route-probe noise", () => {
    const jsEndpoint = endpoint({
      id: "ep-1",
      url: "https://example.com/api/search?q=one",
      path: "/api/search",
      method: "GET",
      params: ["q"],
      headers: { "content-type": "application/json" },
      source: "js_analysis",
      statusCode: 200,
      responseType: "application/json",
      capturePath: ".golish/captures/example.com/api/search.http",
      discoveredAt: "2026-06-30T00:00:00Z",
    });
    const routeProbeEndpoint = endpoint({
      id: "ep-2",
      url: "https://example.com/admin",
      path: "/admin",
      source: "route_probe_paths",
    });

    expect(buildSitemapItems([jsEndpoint, routeProbeEndpoint])).toEqual([
      {
        id: "ep-1",
        url: "https://example.com/api/search?q=one",
        method: "GET",
        path: "/api/search",
        source: "js_analysis",
        kind: "endpoint",
        sizeBytes: null,
        params: ["q"],
        headers: { "content-type": "application/json" },
        statusCode: 200,
        contentType: "application/json",
        capturePath: ".golish/captures/example.com/api/search.http",
        discoveredAt: "2026-06-30T00:00:00Z",
      },
    ]);
  });

  it("includes collected JS files as script items (Burp-style sitemap)", () => {
    const items = buildSitemapItems(
      [],
      [
        jsResult({
          id: "js-9",
          url: "https://example.com/static/umi.js",
          filename: "static/umi.js",
          sizeBytes: 3240643,
        }),
      ]
    );

    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      id: "js-9",
      url: "https://example.com/static/umi.js",
      kind: "script",
      source: "js_file",
      method: "GET",
      sizeBytes: 3240643,
      contentType: "application/javascript",
      capturePath: ".golish/captures/example.com/443/js/static/umi.js",
    });
  });

  it("uses stored JS file_path when present", () => {
    const [item] = buildSitemapItems(
      [],
      [
        jsResult({
          id: "js-stored",
          url: "https://example.com/static/umi.js",
          filename: "static/umi.js",
          filePath: ".golish/captures/example.com/443/js/static/abc_umi.js",
        }),
      ]
    );

    expect(item.capturePath).toBe(".golish/captures/example.com/443/js/static/abc_umi.js");
  });

  it("nests collected JS by capture path using filename, not the bare origin url", () => {
    // Real-world shape: js_extract_apis stores `url` as the bare target origin
    // (identical for every JS of a host) and the real nested path lives in
    // `filename`. The sitemap must reconstruct origin+filename so the tree
    // layers by directory instead of collapsing onto one origin node.
    const items = buildSitemapItems(
      [],
      [
        jsResult({ id: "a", url: "https://example.com", filename: "sd/baxia/login.js" }),
        jsResult({ id: "b", url: "https://example.com", filename: "assets/index.js" }),
      ]
    );

    expect(items.map((item) => item.url).sort()).toEqual([
      "https://example.com/assets/index.js",
      "https://example.com/sd/baxia/login.js",
    ]);

    const tree = buildSitemapTree(items);
    expect(tree).toHaveLength(1);
    expect(tree[0].label).toBe("https://example.com:443");
    expect(tree[0].children.map((node) => node.label).sort()).toEqual(["assets", "sd"]);
    expect(tree[0].children.find((node) => node.label === "sd")?.children[0].label).toBe("baxia");
  });

  it("merges endpoints and scripts into one list", () => {
    const items = buildSitemapItems(
      [
        endpoint({
          id: "ep-1",
          url: "https://example.com/api/x",
          path: "/api/x",
          source: "crawler",
        }),
      ],
      [jsResult({ id: "js-1", url: "https://example.com/static/app.js" })]
    );
    expect(items.map((item) => item.kind).sort()).toEqual(["endpoint", "script"]);
  });
});

describe("buildSitemapTree", () => {
  it("groups sitemap URLs by origin and path segments", () => {
    const tree = buildSitemapTree([
      {
        id: "ep-1",
        url: "https://example.com/api/search?q=one",
        method: "GET",
        path: "/api/search",
        source: "api:js_analysis",
        kind: "endpoint",
        sizeBytes: null,
        params: ["q"],
        headers: {},
        statusCode: 200,
        contentType: "application/json",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:00Z",
      },
      {
        id: "ep-2",
        url: "https://example.com/api/users",
        method: "GET",
        path: "/api/users",
        source: "route_probe_paths",
        kind: "endpoint",
        sizeBytes: null,
        params: [],
        headers: {},
        statusCode: 200,
        contentType: "application/json",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:01Z",
      },
      {
        id: "ep-3",
        url: "https://example.com/admin",
        method: "GET",
        path: "/admin",
        source: "sitemap_url",
        kind: "endpoint",
        sizeBytes: null,
        params: [],
        headers: {},
        statusCode: null,
        contentType: "text/html",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:02Z",
      },
    ]);

    expect(tree).toHaveLength(1);
    expect(tree[0].label).toBe("https://example.com:443");
    expect(tree[0].itemCount).toBe(3);
    expect(tree[0].children.map((node) => node.label)).toEqual(["admin", "api"]);
    expect(tree[0].children[1].children.map((node) => node.label)).toEqual(["search", "users"]);
    expect(tree[0].children[1].children[0].children[0].label).toBe("?q=one");
  });

  it("keeps explicit ports visible on sitemap origin roots", () => {
    const tree = buildSitemapTree([
      {
        id: "https-default",
        url: "https://secure.example.com/login",
        method: "GET",
        path: "/login",
        source: "crawler",
        kind: "endpoint",
        sizeBytes: null,
        params: [],
        headers: {},
        statusCode: 200,
        contentType: "text/html",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:00Z",
      },
      {
        id: "http-default",
        url: "http://plain.example.com/status",
        method: "GET",
        path: "/status",
        source: "crawler",
        kind: "endpoint",
        sizeBytes: null,
        params: [],
        headers: {},
        statusCode: 200,
        contentType: "text/html",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:01Z",
      },
      {
        id: "custom-port",
        url: "https://admin.example.com:8443/",
        method: "GET",
        path: "/",
        source: "crawler",
        kind: "endpoint",
        sizeBytes: null,
        params: [],
        headers: {},
        statusCode: 200,
        contentType: "text/html",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:02Z",
      },
    ]);

    expect(tree.map((node) => node.label).sort()).toEqual([
      "http://plain.example.com:80",
      "https://admin.example.com:8443",
      "https://secure.example.com:443",
    ]);
  });
});

describe("buildSitemapJsSources", () => {
  it("maps a selected sitemap endpoint back to its JS call-site", () => {
    const [item] = buildSitemapItems([
      endpoint({
        id: "ep-1",
        url: "https://example.com/api/search?q=one",
        path: "/api/search",
        method: "GET",
        source: "js_analysis",
      }),
    ]);

    expect(
      buildSitemapJsSources(item, [
        jsResult({
          id: "js-1",
          endpointsFound: [
            {
              method: "GET",
              path: "/api/search?q=${term}",
              source_file: "static/app.js",
              line: 42,
              confidence: 0.91,
              kind: "fetch",
            },
            {
              method: "POST",
              path: "/api/search",
              source_file: "static/app.js",
              line: 84,
            },
          ],
        }),
      ])
    ).toEqual([
      {
        id: "js-1:0",
        filename: "static/app.js",
        url: "https://example.com/static/app.js",
        sourceFile: "static/app.js",
        method: "GET",
        path: "/api/search?q=${term}",
        line: 42,
        confidence: 0.91,
        kind: "fetch",
      },
    ]);
  });
});
