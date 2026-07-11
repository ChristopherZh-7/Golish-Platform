#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "..");
const BACKEND_DIR = path.join(REPO_ROOT, "backend");
const BROWSER_COLLECTOR = path.join(SCRIPT_DIR, "browser_collect_js_api.mjs");
const DEFAULT_MODEL = "deepseek-v4-flash";
const DEFAULT_DEEPSEEK_BASE_URL = "https://api.deepseek.com";
const DEFAULT_FULL_CLOSURE_ROUNDS = 8;

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (!key.startsWith("--")) continue;
    const name = key.slice(2).replaceAll("-", "_");
    const next = argv[i + 1];
    if (!next || next.startsWith("--")) {
      args[name] = true;
      continue;
    }
    args[name] = next;
    i += 1;
  }
  return args;
}

function toBool(value, fallback = false) {
  if (value == null) return fallback;
  if (typeof value === "boolean") return value;
  return ["1", "true", "yes", "y", "on"].includes(String(value).toLowerCase());
}

function toInt(value, fallback, min, max) {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.max(min, Math.min(max, parsed));
}

function requireArg(args, name) {
  const value = args[name];
  if (!value || typeof value !== "string") {
    throw new Error(`--${name.replaceAll("_", "-")} is required`);
  }
  return value;
}

function safeName(value) {
  return String(value).replace(/[^a-zA-Z0-9_.-]+/g, "_").replace(/^_+|_+$/g, "") || "target";
}

function defaultWorkspaceFor(url) {
  const parsed = new URL(url);
  const port = parsed.port || (parsed.protocol === "https:" ? "443" : "80");
  return path.join(os.tmpdir(), `golish-jsapi-${safeName(parsed.hostname)}-${port}`);
}

function captureJsDir(workspace, url) {
  const parsed = new URL(url);
  const scheme = parsed.protocol.replace(/:$/, "");
  const port = parsed.port || (parsed.protocol === "https:" ? "443" : "80");
  return path.join(workspace, ".golish", "captures", parsed.hostname, port, scheme, "js");
}

function runJsonCommand(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd: options.cwd ?? REPO_ROOT,
    encoding: "utf8",
    maxBuffer: 128 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      [
        `${command} exited ${result.status}`,
        result.stderr?.trim(),
        result.stdout?.slice(0, 4000),
      ]
        .filter(Boolean)
        .join("\n"),
    );
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`failed to parse JSON from ${command}: ${error.message}\n${result.stdout}`);
  }
}

function runBrowserCollect(url, workspace, args) {
  const commandArgs = [
    BROWSER_COLLECTOR,
    "--url",
    url,
    "--workspace",
    workspace,
    "--crawl-mode",
    "standard",
    "--max-pages",
    String(toInt(args.max_pages, 12, 1, 100)),
    "--max-actions",
    String(toInt(args.max_actions, 12, 0, 100)),
    "--max-recursive-scripts",
    String(
      toInt(
        args.max_recursive_scripts,
        0,
        0,
        10_000,
      ),
    ),
    "--timeout-ms",
    String(toInt(args.timeout_ms, 60_000, 5_000, 300_000)),
    "--hard-timeout-ms",
    String(
      toInt(
        args.hard_timeout_ms,
        0,
        0,
        600_000,
      ),
    ),
    "--same-origin",
    "true",
    "--block-noise",
    "true",
    "--ai-assist",
    "true",
  ];
  return runJsonCommand(process.env.NODE || "node", commandArgs, { cwd: REPO_ROOT });
}

function shouldContinueFullClosure(result, args, round, startedAt) {
  if (String(args.closure || "bounded").toLowerCase() !== "full") return false;
  if (result?.closure_complete === true || result?.status === "ok") return false;
  const maxRounds = toInt(args.max_closure_rounds, DEFAULT_FULL_CLOSURE_ROUNDS, 1, 50);
  if (round >= maxRounds) return false;
  const maxTotalScripts = toInt(args.max_total_scripts, 12_000, 1, 100_000);
  if (Number(result?.scripts_saved ?? 0) >= maxTotalScripts) return false;
  const maxTotalMs = toInt(args.max_total_ms, 1_800_000, 30_000, 7_200_000);
  if (Date.now() - startedAt >= maxTotalMs) return false;
  const remaining = Number(result?.recursive_queue_remaining ?? 0);
  return remaining > 0 || result?.status === "closure_partial" || result?.status === "timeout_partial";
}

function runCollection(url, workspace, args) {
  if (toBool(args.skip_collection, false)) {
    return skippedCollection(url);
  }
  const startedAt = Date.now();
  const rounds = [];
  let round = 0;
  do {
    round += 1;
    const result = runBrowserCollect(url, workspace, args);
    rounds.push(result);
    if (!shouldContinueFullClosure(result, args, round, startedAt)) break;
  } while (true);
  const final = rounds.at(-1);
  return {
    rounds,
    final,
    closure_mode: String(args.closure || "bounded").toLowerCase(),
    elapsed_ms: Date.now() - startedAt,
  };
}

function skippedCollection(url) {
  const final = {
    status: "skipped",
    target_url: url,
    closure_complete: true,
    scripts_saved: null,
    scripts_cached_preloaded: null,
    scripts_duplicate_content_hits: null,
    scripts_recursive_downloaded: null,
    recursive_queue_remaining: 0,
    api_requests_total: null,
  };
  return {
    rounds: [final],
    final,
    closure_mode: "skipped",
    elapsed_ms: 0,
  };
}

function runStaticExtract(url, jsDir, args) {
  return runJsonCommand(
    "cargo",
    [
      "run",
      "-q",
      "-p",
      "golish-js-analyzer",
      "--bin",
      "js_api_extract",
      "--",
      "--js-dir",
      jsDir,
      "--target-url",
      url,
      "--endpoint-limit",
      String(toInt(args.endpoint_limit, 300, 1, 5000)),
      "--signal-limit",
      String(toInt(args.signal_limit, 160, 1, 5000)),
      "--context-limit",
      String(toInt(args.context_limit, 32, 0, 200)),
    ],
    { cwd: BACKEND_DIR },
  );
}

function loadGolishDeepSeekConfig() {
  const config = {
    apiKey: process.env.DEEPSEEK_API_KEY || "",
    baseUrl: process.env.DEEPSEEK_BASE_URL || DEFAULT_DEEPSEEK_BASE_URL,
    model: process.env.DEEPSEEK_MODEL || DEFAULT_MODEL,
  };
  const settingsPath = path.join(os.homedir(), ".golish", "settings.toml");
  return fs
    .readFile(settingsPath, "utf8")
    .then((text) => {
      let section = "";
      for (const rawLine of text.split(/\r?\n/)) {
        const line = rawLine.trim();
        if (!line || line.startsWith("#")) continue;
        const sectionMatch = line.match(/^\[([^\]]+)\]$/);
        if (sectionMatch) {
          section = sectionMatch[1];
          continue;
        }
        const kv = line.match(/^([A-Za-z0-9_.-]+)\s*=\s*"([^"]*)"\s*$/);
        if (!kv) continue;
        const [, key, value] = kv;
        if (!process.env.DEEPSEEK_MODEL && key === "default_model") {
          config.model = value || config.model;
        }
        if (section === "ai.deepseek") {
          if (!process.env.DEEPSEEK_API_KEY && key === "api_key") config.apiKey = value;
          if (!process.env.DEEPSEEK_BASE_URL && key === "base_url" && value) config.baseUrl = value;
        }
      }
      return config;
    })
    .catch(() => config);
}

function compactForAi(analysis, collection) {
  const endpoints = sampleForAi(analysis.endpoints, 120);
  const secretCandidates = sampleForAi(analysis.secret_candidates, 80);
  const configCandidates = sampleForAi(analysis.config_candidates, 40);
  const contextSnippets = sampleForAi(analysis.ai_review?.context_snippets, 30);
  const ruleMatches = sampleForAi(analysis.rule_matches, 80);
  return {
    collection: {
      final_status: collection.final?.status,
      rounds: collection.rounds?.length ?? 0,
      scripts_saved: collection.final?.scripts_saved,
      scripts_recursive_downloaded: collection.final?.scripts_recursive_downloaded,
      recursive_queue_remaining: collection.final?.recursive_queue_remaining,
      api_requests_total: collection.final?.api_requests_total,
    },
    analysis: {
      status: analysis.status,
      target_url: analysis.target_url,
      js_dir: analysis.js_dir,
      files_scanned: analysis.files_scanned,
      api_base_path: analysis.api_base_path,
      endpoints_total: analysis.endpoints_total,
      endpoints_unique: analysis.endpoints_unique,
      secret_candidates_total: analysis.secret_candidates_total,
      config_candidates_total: analysis.config_candidates_total,
      rule_matches_total: analysis.rule_matches_total,
      summary: analysis.summary,
      ai_review_reasons: analysis.ai_review?.reasons ?? [],
    },
    sampling: {
      endpoints: endpoints.meta,
      secret_candidates: secretCandidates.meta,
      config_candidates: configCandidates.meta,
      context_snippets: contextSnippets.meta,
      rule_matches: ruleMatches.meta,
      interpretation:
        "AI classifications apply only to included sample arrays. Deterministic totals in analysis remain the full extractor counts.",
    },
    endpoints: endpoints.items,
    secret_candidates: secretCandidates.items,
    config_candidates: configCandidates.items,
    context_snippets: contextSnippets.items,
    rule_matches_sample: ruleMatches.items,
  };
}

function sampleForAi(items, limit) {
  const source = Array.isArray(items) ? items : [];
  const sampled = source.slice(0, limit);
  return {
    items: sampled,
    meta: {
      total: source.length,
      included: sampled.length,
      limit,
      truncated: source.length > sampled.length,
    },
  };
}

function extractJsonObject(text) {
  const trimmed = text.trim();
  if (trimmed.startsWith("{") && trimmed.endsWith("}")) return JSON.parse(trimmed);
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/i);
  if (fenced) return JSON.parse(fenced[1]);
  const start = trimmed.indexOf("{");
  const end = trimmed.lastIndexOf("}");
  if (start >= 0 && end > start) return JSON.parse(trimmed.slice(start, end + 1));
  throw new Error(`AI response did not contain JSON: ${trimmed.slice(0, 1000)}`);
}

async function runAiFilter(analysis, collection, args) {
  const config = await loadGolishDeepSeekConfig();
  if (!config.apiKey) {
    return {
      status: "skipped",
      reason: "missing DEEPSEEK_API_KEY and no ai.deepseek.api_key in ~/.golish/settings.toml",
      provider: "deepseek",
      model: config.model,
    };
  }
  const model = String(args.ai_model || config.model || DEFAULT_MODEL);
  const payload = compactForAi(analysis, collection);
  const prompt = [
    "You are filtering JavaScript enumeration output for an authorized security test.",
    "Use only the supplied structured data and local source snippets.",
    "Classify API endpoints, secret candidates, and rule matches as real, test, noise, or needs_followup.",
    "Candidate arrays may be sampled; use the sampling metadata and never present sample classifications as full deterministic counts.",
    "Never output raw secrets. Use only source_file, line, kind, value_preview, value_sha256, and concise rationale for secret_triage.",
    "Do not claim verification of live vulnerability. Do not suggest exploitation.",
    "Return strict JSON with keys: status, provider, model, real_endpoints, test_or_placeholder, noise, needs_followup, secret_triage, rule_filter_summary, notes.",
    "Keep reasons concise and cite source_file/line when available.",
  ].join("\n");

  const response = await fetch(`${config.baseUrl.replace(/\/+$/, "")}/chat/completions`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${config.apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model,
      temperature: 0.1,
      messages: [
        { role: "system", content: prompt },
        { role: "user", content: JSON.stringify(payload) },
      ],
    }),
  });

  const text = await response.text();
  if (!response.ok) {
    return {
      status: "error",
      provider: "deepseek",
      model,
      http_status: response.status,
      message: text.slice(0, 2000),
    };
  }
  const body = JSON.parse(text);
  const content = body?.choices?.[0]?.message?.content ?? "";
  const parsed = extractJsonObject(content);
  return {
    ...parsed,
    provider: "deepseek",
    model,
    input_sampling: payload.sampling,
  };
}

function triageCount(value) {
  if (Array.isArray(value)) return value.length;
  if (!value || typeof value !== "object") return undefined;
  return ["real", "test_or_placeholder", "noise", "needs_followup"].reduce((sum, key) => {
    const bucket = value[key];
    return sum + (Array.isArray(bucket) ? bucket.length : 0);
  }, 0);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const url = requireArg(args, "url");
  const workspace = path.resolve(String(args.workspace || defaultWorkspaceFor(url)));
  const closureMode = String(args.closure || "bounded").toLowerCase();
  if (!["bounded", "full"].includes(closureMode)) {
    throw new Error("--closure must be bounded or full");
  }
  const aiFilter = toBool(args.ai_filter, false);
  await fs.mkdir(workspace, { recursive: true });

  const collection = runCollection(url, workspace, args);
  const finalCollection = collection.final;
  const jsDir = args.js_dir ? path.resolve(String(args.js_dir)) : captureJsDir(workspace, url);
  const analysis = runStaticExtract(url, jsDir, args);
  const ai_filter = aiFilter
    ? await runAiFilter(analysis, collection, args)
    : { status: "skipped", reason: "run with --ai-filter true" };

  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
  const outputDir = path.join(workspace, ".golish", "js-api-test-results", timestamp);
  await fs.mkdir(outputDir, { recursive: true });
  const summary = {
    status: "ok",
    target_url: url,
    workspace,
    js_dir: jsDir,
    output_dir: outputDir,
    collection,
    analysis,
    ai_filter,
  };
  await fs.writeFile(path.join(outputDir, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);

  const compact = {
    status: "ok",
    target_url: url,
    workspace,
    output_dir: outputDir,
    collection: {
      closure_mode: collection.closure_mode,
      rounds: collection.rounds.length,
      final_status: finalCollection.status,
      closure_complete: finalCollection.closure_complete,
      scripts_saved: finalCollection.scripts_saved,
      scripts_cached_preloaded: finalCollection.scripts_cached_preloaded,
      scripts_duplicate_content_hits: finalCollection.scripts_duplicate_content_hits,
      scripts_recursive_downloaded: finalCollection.scripts_recursive_downloaded,
      recursive_queue_remaining: finalCollection.recursive_queue_remaining,
    },
    analysis: {
      files_scanned: analysis.files_scanned,
      api_base_path: analysis.api_base_path,
      endpoints_total: analysis.endpoints_total,
      endpoints_unique: analysis.endpoints_unique,
      secret_candidates_total: analysis.secret_candidates_total,
      rule_matches_total: analysis.rule_matches_total,
      summary: analysis.summary,
    },
    ai_filter: {
      status: ai_filter.status,
      provider: ai_filter.provider,
      model: ai_filter.model,
      real_endpoints_count: Array.isArray(ai_filter.real_endpoints)
        ? ai_filter.real_endpoints.length
        : undefined,
      needs_followup_count: Array.isArray(ai_filter.needs_followup)
        ? ai_filter.needs_followup.length
        : undefined,
      secret_triage_count: triageCount(ai_filter.secret_triage),
    },
  };
  process.stdout.write(`${JSON.stringify(compact, null, 2)}\n`);
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exit(1);
});
