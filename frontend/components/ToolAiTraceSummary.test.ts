import { describe, expect, it } from "vitest";
import { extractToolAiTraceSections } from "./ToolAiTraceSummary";

describe("extractToolAiTraceSections", () => {
  it("no longer surfaces browser collector hints (removed for a simpler detail view)", () => {
    const sections = extractToolAiTraceSections({
      ai_assist: {
        recommended: true,
        reasons: ["js_saved_but_no_runtime_api_requests"],
        next_step: "Inspect context and call browser_collect_js_api again with a bounded recipe.",
        context: {
          signals: { closure_complete: true, scripts_saved: 3, api_requests_total: 0 },
          script_observations: [{ url: "https://example.test/app.js", size: 2048 }],
        },
      },
    });

    expect(sections).toHaveLength(0);
  });

  it("surfaces JS extraction static analysis handoff", () => {
    const sections = extractToolAiTraceSections({
      files_scanned: 12,
      endpoints_total: 4,
      secrets_total: 1,
      rule_matches_total: 9,
      ai_analysis: {
        api_base_path: "/api",
        candidate_files: [
          {
            source_file: "static/app.js",
            endpoints: 4,
            secrets: 1,
            configs: 0,
            rule_matches: 9,
            line_hints: [{ line_start: 42, line_end: 47 }],
          },
        ],
      },
    });

    expect(sections).toHaveLength(1);
    expect(sections[0].title).toBe("Static Analysis Hints");
    expect(sections[0].chips).toContain("files 12");
    expect(sections[0].chips).toContain("endpoints 4");
    expect(sections[0].chips).toContain("base /api");
    expect(sections[0].fileRows[0].source).toBe("static/app.js");
    expect(sections[0].fileRows[0].lines).toContain("L42-47");
  });
});
