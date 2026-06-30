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
  it("includes only JS-derived API endpoint evidence", () => {
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
        params: ["q"],
        headers: { "content-type": "application/json" },
        statusCode: 200,
        contentType: "application/json",
        capturePath: ".golish/captures/example.com/api/search.http",
        discoveredAt: "2026-06-30T00:00:00Z",
      },
    ]);
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
        params: [],
        headers: {},
        statusCode: null,
        contentType: "text/html",
        capturePath: null,
        discoveredAt: "2026-06-30T00:00:02Z",
      },
    ]);

    expect(tree).toHaveLength(1);
    expect(tree[0].label).toBe("https://example.com");
    expect(tree[0].itemCount).toBe(3);
    expect(tree[0].children.map((node) => node.label)).toEqual(["admin", "api"]);
    expect(tree[0].children[1].children.map((node) => node.label)).toEqual(["search", "users"]);
    expect(tree[0].children[1].children[0].children[0].label).toBe("?q=one");
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
