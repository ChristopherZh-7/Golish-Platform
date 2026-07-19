import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WebOriginVM } from "../surfaceHierarchy";
import type { WhatWebAssessment } from "../whatWebAssessment";
import { WebOriginsTab } from "./WebOriginsTab";

const ORIGIN = "http://123.6.40.244:8000";

function origin(): WebOriginVM {
  return {
    id: ORIGIN,
    origin: ORIGIN,
    scheme: "http",
    host: "123.6.40.244",
    port: 8000,
    hostType: "ip",
    endpointIds: [],
    observationIds: [],
    urls: [],
    apiEndpoints: [],
    jsResources: [],
    params: [],
    directoryEntries: [],
    fingerprints: [],
    evidence: [],
    counts: {
      urls: 0,
      apis: 0,
      js: 0,
      params: 0,
      directoryEntries: 0,
      findings: 0,
      evidence: 0,
      passiveLogs: 0,
    },
    confidence: "confirmed",
    source: "backend_identity",
    contentCountSource: "backend_content_counts",
    contentRefs: [],
    crawlObservations: [],
  };
}

function assessment(): WhatWebAssessment {
  return {
    origin: ORIGIN,
    state: "producer_blocked",
    observedAt: "2026-07-17T00:03:00Z",
    attempt: 3,
    failureClass: "connection_reset",
  };
}

describe("WebOriginsTab assessment", () => {
  it("shows one evidence-backed producer status in the matching table column", () => {
    render(
      <WebOriginsTab
        webOrigins={[origin()]}
        endpoints={[]}
        loading={false}
        selectedOriginId={ORIGIN}
        onSelectOrigin={vi.fn()}
        projectPath={null}
        assessmentByOrigin={new Map([[ORIGIN, assessment()]])}
      />
    );

    expect(screen.getAllByText("WhatWeb stopped")).toHaveLength(1);
    const headers = screen.getAllByRole("columnheader").map((cell) => cell.textContent);
    expect(headers.slice(0, 3)).toEqual(["Origin", "WhatWeb", "Scheme"]);
    expect(screen.getAllByText(ORIGIN).length).toBeGreaterThan(0);
  });
});
