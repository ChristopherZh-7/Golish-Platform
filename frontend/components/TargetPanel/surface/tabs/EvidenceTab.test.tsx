import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Target } from "@/lib/pentest/types";
import type { TimelineEntry } from "@/lib/security-analysis";
import type { SurfaceHierarchyVM } from "../surfaceHierarchy";
import { EvidenceTab } from "./EvidenceTab";

const target = {
  id: "target-1",
  name: "203.0.113.10",
  type: "ip",
  value: "203.0.113.10",
  tags: [],
  notes: "",
  scope: "in",
  status: "active",
  grp: "",
  owner: "",
  time_window_start: null,
  time_window_end: null,
  organization_id: "org-1",
  source: "asset_intel",
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
} as Target;

const exactOrigin = "https://example.com:443";

const hierarchy = {
  webOrigins: [
    {
      id: exactOrigin,
      origin: exactOrigin,
      scheme: "https",
      host: "example.com",
      port: 443,
    },
  ],
} as SurfaceHierarchyVM;

function timelineEntry(overrides: Partial<TimelineEntry> = {}): TimelineEntry {
  return {
    source: "audit_log",
    event: "whatweb_completed",
    category: "evidence",
    details: `[eas.fingerprint_web_stack] ${exactOrigin}`,
    toolName: "whatweb",
    status: "completed",
    detail: {},
    createdAt: "2026-07-12T00:00:00Z",
    ...overrides,
  };
}

function renderEvidence(entry: TimelineEntry) {
  return render(
    <EvidenceTab
      target={target}
      timeline={[entry]}
      logs={[]}
      loading={false}
      hierarchy={hierarchy}
    />
  );
}

describe("EvidenceTab WhatWeb transport evidence", () => {
  it("shows a retryable network error with attempt, failure class, outcome and exact origin", () => {
    renderEvidence(
      timelineEntry({
        detail: {
          kind: "eas.fingerprint_web_stack",
          subject: exactOrigin,
          raw_output: JSON.stringify({
            whatweb_line: "ERROR Connection reset by peer",
            failure_class: "connection_reset",
            attempt: 2,
            producer_outcome: "error",
            independently_confirmed: false,
          }),
        },
      })
    );

    expect(screen.getByText("WhatWeb network error")).toBeInTheDocument();
    expect(screen.getByText("Attempt 2/3")).toBeInTheDocument();
    expect(screen.getByText("connection_reset")).toBeInTheDocument();
    expect(screen.getByText("error")).toBeInTheDocument();
    expect(screen.getByText(`${exactOrigin} · confirmed`)).toBeInTheDocument();
    expect(screen.queryByText(/Excluded from Enumeration/)).not.toBeInTheDocument();
  });

  it("shows the terminal third failure and independently confirmed Enumeration exclusion", () => {
    renderEvidence(
      timelineEntry({
        detail: {
          kind: "eas.fingerprint_web_stack",
          subject: exactOrigin,
          raw_output: JSON.stringify({
            whatweb_line: "ERROR TLS handshake failed",
            failure_class: "tls_handshake",
            attempt: 3,
            producer_outcome: "blocked",
            independently_confirmed: true,
          }),
        },
      })
    );

    expect(screen.getByText("WhatWeb stopped after 3 network failures")).toBeInTheDocument();
    expect(screen.getByText("Attempt 3/3")).toBeInTheDocument();
    expect(screen.getByText("tls_handshake")).toBeInTheDocument();
    expect(screen.getByText("blocked")).toBeInTheDocument();
    expect(
      screen.getByText(`Excluded exact origin from Enumeration: ${exactOrigin}`)
    ).toBeInTheDocument();
  });

  it("keeps ordinary and malformed evidence rows unchanged", () => {
    const { rerender } = renderEvidence(
      timelineEntry({
        event: "httpx_completed",
        details: "ordinary evidence remains visible",
        toolName: "httpx",
        detail: { kind: "eas.probe_http_liveness", subject: exactOrigin },
      })
    );

    expect(screen.getByText("ordinary evidence remains visible")).toBeInTheDocument();
    expect(screen.queryByText(/WhatWeb network error/)).not.toBeInTheDocument();

    rerender(
      <EvidenceTab
        target={target}
        timeline={[
          timelineEntry({
            details: "malformed WhatWeb evidence remains visible",
            detail: {
              kind: "eas.fingerprint_web_stack",
              subject: exactOrigin,
              raw_output: "not-json",
            },
          }),
        ]}
        logs={[]}
        loading={false}
        hierarchy={hierarchy}
      />
    );

    expect(screen.getByText("malformed WhatWeb evidence remains visible")).toBeInTheDocument();
    expect(screen.queryByText("error")).not.toBeInTheDocument();
    expect(screen.queryByText("blocked")).not.toBeInTheDocument();
  });
});
