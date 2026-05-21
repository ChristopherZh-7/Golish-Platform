import { describe, expect, it } from "vitest";
import type { ResolvedIntegration } from "@/lib/api/integrations";
import { matchSearch } from "./CategoryNav";

function makeIntegration(partial: Partial<ResolvedIntegration> = {}): ResolvedIntegration {
  return {
    tool_id: partial.tool_id ?? "enscan-go",
    schema: {
      category: "enterprise-intel",
      display_name: "ENScan_GO",
      description: "Multi-source enterprise intelligence collector",
      storage: { type: "vault" },
      groups: [
        {
          id: "aqc",
          name: "爱企查 (AQC)",
          fields: [],
        },
      ],
      ...partial.schema,
    },
  };
}

describe("matchSearch", () => {
  it("passes everything when query is empty or whitespace", () => {
    const item = makeIntegration();
    expect(matchSearch(item, "")).toBe(true);
    expect(matchSearch(item, "   ")).toBe(true);
  });

  it("matches against display_name case-insensitively", () => {
    const item = makeIntegration();
    expect(matchSearch(item, "enscan")).toBe(true);
    expect(matchSearch(item, "ENSCAN")).toBe(true);
  });

  it("matches against tool_id", () => {
    const item = makeIntegration({ tool_id: "0.zone" });
    expect(matchSearch(item, "0.zone")).toBe(true);
  });

  it("matches against category", () => {
    const item = makeIntegration();
    expect(matchSearch(item, "enterprise")).toBe(true);
  });

  it("matches against group name (CJK support)", () => {
    const item = makeIntegration();
    expect(matchSearch(item, "爱企查")).toBe(true);
  });

  it("requires every whitespace-separated token to match (AND semantics)", () => {
    const item = makeIntegration();
    // "enscan" + "enterprise" both appear somewhere → match
    expect(matchSearch(item, "enscan enterprise")).toBe(true);
    // "enscan" appears but "fofa" doesn't → no match
    expect(matchSearch(item, "enscan fofa")).toBe(false);
  });

  it("returns false for queries that don't appear anywhere", () => {
    const item = makeIntegration();
    expect(matchSearch(item, "nonexistent-keyword")).toBe(false);
  });
});
