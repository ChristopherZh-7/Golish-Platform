import { describe, expect, it } from "vitest";
import { extractToolAiTraceSections } from "./ToolAiTraceSummary";

describe("extractToolAiTraceSections", () => {
  it("does not surface collector hints without concrete result fields", () => {
    const sections = extractToolAiTraceSections({
      ai_assist: {
        recommended: true,
        reasons: ["js_saved_but_no_runtime_api_requests"],
        next_step: "Inspect context and call browser_collect_js_api again with a bounded recipe.",
      },
    });

    expect(sections).toHaveLength(0);
  });

  it("summarizes JS extraction as key findings", () => {
    const sections = extractToolAiTraceSections({
      files_scanned: 12,
      endpoints_total: 4,
      endpoints_unique: 3,
      persisted_endpoint_rows: 2,
      duplicate_endpoint_rows: 1,
      secrets_total: 1,
      param_endpoints: 2,
      jsapi_outcome: "found",
      param_outcome: "found",
      endpoints: [
        {
          method: "GET",
          path: "/api/users",
          source: "regex",
          source_file: "static/app.js",
          line: 42,
        },
      ],
    });

    expect(sections).toHaveLength(1);
    expect(sections[0].title).toBe("Key Findings");
    expect(sections[0].chips).toEqual(
      expect.arrayContaining(["JS API 4", "unique 3", "JS landed 2", "params 2", "secrets 1"])
    );
    expect(sections[0].reasons).toEqual(
      expect.arrayContaining([
        "Static JS extraction found 4 API endpoint(s) across 12 JS file(s); 2 landed and 1 already existed.",
        "Parameter evidence landed for 2 endpoint(s).",
        "JSAPI outcome: found.",
        "PARAM outcome: found.",
      ])
    );
    expect(sections[0].fileRows[0]).toMatchObject({
      source: "GET /api/users",
      meta: expect.arrayContaining(["source regex", "line 42", "file static/app.js"]),
    });
  });

  it("hides noisy JS framework and rule signals from the default summary", () => {
    const sections = extractToolAiTraceSections({
      endpoints_total: 1,
      frameworks_total: 4,
      rule_matches_total: 2,
      frameworks: [
        {
          name: "Webpack",
          source_file: "_next/static/chunks/a.js",
          confidence: 0.82,
          evidence: "matched webpackChunk",
        },
      ],
      rule_matches: [{ group: "Other", kind: "route", source_file: "app.js", confidence: 0.54 }],
    });

    expect(sections).toHaveLength(1);
    expect(sections[0].title).toBe("Key Findings");
    expect(sections[0].chips).not.toContain("frameworks 4");
    expect(sections[0].chips).not.toContain("rules 2");
    expect(sections[0].fileRows.some((row) => row.source.includes("Webpack"))).toBe(false);
  });

  it("summarizes route probe results as found-or-empty", () => {
    const sections = extractToolAiTraceSections({
      outcome: "empty",
      queue_completed: true,
      requests_sent: 1200,
      candidate_requests_sent: 1150,
      baseline_requests_sent: 50,
      matches: [],
      rejected_candidates: [{ url: "https://example.test/admin", reason: "uniform_response" }],
      errors: [{ url: "https://example.test/debug", error: "timeout" }],
      persisted_directory_entries: 0,
      seed_paths: { total_after_dedupe: 3 },
      wordlist: { entries_loaded: 1882 },
    });

    expect(sections).toHaveLength(1);
    expect(sections[0].title).toBe("Key Findings");
    expect(sections[0].chips).toEqual(
      expect.arrayContaining(["paths 0", "checked 1200", "outcome empty", "queue complete"])
    );
    expect(sections[0].reasons).toEqual(
      expect.arrayContaining([
        "Route probe checked 1200 request(s) and found no verified path.",
        "1 soft/uniform candidate(s) were rejected and hidden from findings.",
        "1 request error(s) occurred; raw output keeps the details.",
      ])
    );
  });

  it("shows verified route paths when present", () => {
    const sections = extractToolAiTraceSections({
      outcome: "found",
      requests_sent: 50,
      matches: [{ url: "https://example.test/admin", status: 200, verdict: "verified" }],
      persisted_directory_entries: 1,
      seed_paths: { total_after_dedupe: 3 },
    });

    expect(sections[0].chips).toContain("paths 1");
    expect(sections[0].reasons).toContain("Route probe found 1 verified path(s).");
    expect(sections[0].fileRows[0]).toMatchObject({
      source: "https://example.test/admin",
      meta: expect.arrayContaining(["status 200", "verdict verified"]),
    });
  });

  it("summarizes browser collection without exposing AI request/response samples", () => {
    const sections = extractToolAiTraceSections({
      scripts_saved: 6,
      api_requests_total: 4,
      persisted_api_rows: 2,
      duplicate_api_rows: 1,
      ai_recipe_rounds: 2,
      persistable_api_requests: [{ method: "POST", path: "/api/login", status: 200 }],
      ai_dialogue: [
        {
          stage: "recipe",
          request: "{}",
          response: '{"needs_second_pass":false}',
        },
      ],
    });

    expect(sections).toHaveLength(1);
    expect(sections[0].title).toBe("Key Findings");
    expect(sections[0].chips).toEqual(
      expect.arrayContaining(["runtime API 4", "saved JS 6", "runtime landed 2", "AI recipe 2"])
    );
    expect(sections[0].fileRows[0]).toMatchObject({
      source: "POST /api/login",
      meta: expect.arrayContaining(["status 200"]),
    });
    expect(JSON.stringify(sections)).not.toContain("needs_second_pass");
  });

  it("folds in-tool AI into the key finding instead of rendering a separate AI Pass section", () => {
    const sections = extractToolAiTraceSections({
      endpoints_total: 7,
      persisted_endpoint_rows: 4,
      hae_route_candidates_total: 12,
      hae_ai_promoted: 2,
      summary: { ai_used: true, ai_endpoints_added: 3 },
    });

    expect(sections).toHaveLength(1);
    expect(sections[0].title).toBe("Key Findings");
    expect(sections[0].chips).toEqual(
      expect.arrayContaining(["HAE candidates 12", "HAE promoted 2", "AI +3"])
    );
    expect(sections[0].reasons).toContain(
      "HaE-style regex produced 12 route/path candidate(s); 2 were AI-promoted into the API set."
    );
    expect(sections[0].reasons).toContain("AI review ran and added 3 endpoint candidate(s).");
  });

  it("renders nothing for unrelated JSON", () => {
    const sections = extractToolAiTraceSections({
      status: "ok",
      message: "done",
    });

    expect(sections).toHaveLength(0);
  });
});
