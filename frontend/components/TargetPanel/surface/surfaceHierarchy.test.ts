import { describe, expect, it } from "vitest";
import type { DirectoryEntry } from "@/lib/pentest/api";
import type { PortInfo, Target } from "@/lib/pentest/types";
import type { ApiEndpoint, JsAnalysisResult, TargetAsset } from "@/lib/security-analysis";
import {
  buildSurfaceHierarchy,
  normalizeEndpointKey,
  normalizeOriginKey,
  parseWebOrigin,
} from "./surfaceHierarchy";

const target = (p: Partial<Target>): Target =>
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
  discoveredAt: "2026-07-01T00:00:00Z",
  updatedAt: "2026-07-01T00:00:00Z",
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
  analyzedAt: "2026-07-01T00:00:00Z",
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
const asset = (p: Partial<TargetAsset>): TargetAsset => ({
  id: "asset-1",
  targetId: "target-ip",
  projectPath: null,
  assetType: "service",
  value: "1.2.3.4",
  port: null,
  protocol: null,
  service: null,
  version: null,
  metadata: {},
  status: "open",
  discoveredAt: "2026-07-01T00:00:00Z",
  updatedAt: "2026-07-01T00:00:00Z",
  ...p,
});

describe("surface hierarchy keys", () => {
  it("normalizes origins with default ports", () => {
    expect(normalizeOriginKey("https", "CSPM.MoreSec.CN", null)).toBe(
      "https://cspm.moresec.cn:443"
    );
    expect(normalizeOriginKey("http", "cspm.moresec.cn", "")).toBe(
      "http://cspm.moresec.cn:80"
    );
    expect(parseWebOrigin("https://1.94.38.88/admin")?.id).toBe("https://1.94.38.88:443");
  });

  it("normalizes network endpoint keys", () => {
    expect(normalizeEndpointKey("1.2.3.4", 8443, "TCP")).toBe("1.2.3.4:8443:tcp");
  });
});

describe("buildSurfaceHierarchy", () => {
  it("groups target ports, origins, JS, params, and directory entries without backend models", () => {
    const vm = buildSurfaceHierarchy({
      rootTarget: target({
        ports: [
          port({ port: 443, protocol: "tcp", service: "https" }),
          port({ port: 8443, protocol: "tcp", service: "https-alt" }),
        ],
      }),
      relatedDomains: [target({ id: "domain-1", type: "domain", value: "a.com", real_ip: "1.2.3.4" })],
      assets: [asset({ id: "asset-443", port: 443, protocol: "tcp", service: "https" })],
      apiEndpoints: [
        endpoint({
          id: "api-1",
          url: "https://a.com/api/login",
          path: "/api/login",
          method: "POST",
          params: ["username", "password"],
          capturePath: ".golish/captures/a.com/443/api/login.json",
        }),
      ],
      jsResults: [
        jsResult({
          id: "js-1",
          url: "https://a.com/",
          filename: "assets/app.js",
          filePath: ".golish/captures/a.com/443/js/assets/app.js",
        }),
      ],
      directoryEntries: [
        directoryEntry({
          id: "dir-1",
          url: "https://a.com/admin",
          status_code: 401,
        }),
      ],
    });

    expect(vm.mode).toBe("ip");
    expect(vm.endpoints.map((item) => item.id)).toContain("1.2.3.4:443:tcp");
    expect(vm.webOrigins.map((item) => item.id)).toEqual(["https://a.com:443"]);
    expect(vm.webOrigins[0].endpointIds).toContain("1.2.3.4:443:tcp");
    expect(vm.webOrigins[0].counts).toMatchObject({
      urls: 3,
      apis: 1,
      js: 1,
      params: 2,
      directoryEntries: 1,
    });
    expect(vm.relatedDomains[0].webOriginIds).toEqual(["https://a.com:443"]);
  });

  it("keeps relative or incomplete web data unassigned", () => {
    const vm = buildSurfaceHierarchy({
      rootTarget: target({ ports: [port({ port: 443, protocol: "tcp", service: "https" })] }),
      apiEndpoints: [endpoint({ id: "api-relative", url: "", path: "/api/relative", params: ["q"] })],
      jsResults: [jsResult({ id: "js-relative", url: "", filename: "assets/app.js" })],
      directoryEntries: [directoryEntry({ id: "dir-relative", url: "/admin" })],
    });

    expect(vm.webOrigins).toEqual([]);
    expect(vm.unassignedWebData.apis).toHaveLength(1);
    expect(vm.unassignedWebData.js).toHaveLength(1);
    expect(vm.unassignedWebData.urls).toHaveLength(1);
    expect(vm.unassignedWebData.params).toHaveLength(1);
  });
});
