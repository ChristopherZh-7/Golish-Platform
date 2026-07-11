#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "..");
const BROWSER_COLLECTOR = path.join(SCRIPT_DIR, "browser_collect_js_api.mjs");
const DEFAULT_URL = "https://life.pingan.com/";
const DEFAULT_MODEL = "deepseek-chat";
const DEFAULT_DEEPSEEK_BASE_URL = "https://api.deepseek.com";

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

function toInt(value, fallback, { min = 1, max = Number.MAX_SAFE_INTEGER } = {}) {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(max, Math.max(min, parsed));
}

function safeName(value) {
  return String(value).replace(/[^a-zA-Z0-9_.-]+/g, "_").replace(/^_+|_+$/g, "") || "target";
}

function defaultWorkspace(url) {
  const parsed = new URL(url);
  const port = parsed.port || (parsed.protocol === "http:" ? "80" : "443");
  return path.join(os.tmpdir(), `golish-jsapi-ai-recipe-${safeName(parsed.hostname)}-${port}`);
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
  return JSON.parse(result.stdout);
}

function runCollector(url, workspace, args, recipe = null) {
  const commandArgs = [
    BROWSER_COLLECTOR,
    "--url",
    url,
    "--workspace",
    workspace,
    "--crawl-mode",
    "standard",
    "--max-pages",
    String(toInt(args.max_pages, 12, { min: 1, max: 100 })),
    "--max-actions",
    String(toInt(args.max_actions, 12, { min: 0, max: 100 })),
    "--max-recursive-scripts",
    String(toInt(args.max_recursive_scripts, 0, { min: 0, max: 5000 })),
    "--timeout-ms",
    String(toInt(args.timeout_ms, 60000, { min: 5000, max: 300000 })),
    "--hard-timeout-ms",
    String(toInt(args.hard_timeout_ms, 0, { min: 0, max: 600000 })),
    "--same-origin",
    "true",
    "--block-noise",
    "true",
    "--ai-assist",
    "true",
  ];
  if (recipe) {
    commandArgs.push("--recipe-json", JSON.stringify(recipe));
  }
  return runJsonCommand(process.env.NODE || "node", commandArgs);
}

function compactCollection(result) {
  const scriptObservations = (result.ai_assist?.context?.script_observations ?? [])
    .slice(0, 8)
    .map((observation) => ({
      url: observation.url,
      size: observation.size,
      public_path_detected: observation.public_path_detected,
      refs_sample: (observation.refs_sample ?? []).slice(0, 10),
      ai_review_refs_sample: (observation.ai_review_refs_sample ?? []).slice(0, 10),
      chunk_urls_sample: (observation.chunk_urls_sample ?? []).slice(0, 10),
      runtime_chunk_urls_sample: (observation.runtime_chunk_urls_sample ?? []).slice(0, 10),
      vite_chunk_urls_sample: (observation.vite_chunk_urls_sample ?? []).slice(0, 10),
    }));
  return {
    status: result.status,
    target_url: result.target_url,
    scripts_saved: result.scripts_saved,
    scripts_total: result.scripts_total,
    scripts_recursive_downloaded: result.scripts_recursive_downloaded,
    recursive_queue_remaining: result.recursive_queue_remaining,
    closure_complete: result.closure_complete,
    closure_incomplete_reasons: result.closure_incomplete_reasons,
    recursive_errors_total: result.recursive_errors_total,
    recursive_errors_by_status: result.recursive_errors_by_status,
    recursive_errors_sample: (result.recursive_errors ?? []).slice(0, 20),
    api_requests_total: result.api_requests_total,
    api_requests_sample: (result.api_requests ?? []).slice(0, 20),
    ai_review_refs_total: result.ai_review_refs_total,
    ai_review_refs_sample: (result.ai_review_refs ?? []).slice(0, 20),
    scripts: (result.scripts ?? [])
      .filter((script) => script.path)
      .slice(0, 40)
      .map((script) => ({
        url: script.url,
        status: script.status,
        size: script.size,
        path: script.path,
      })),
    script_observations: scriptObservations,
  };
}

function shouldUseAi(result, args) {
  if (toBool(args.no_ai, false)) return false;
  if (toBool(args.force_ai, false)) return true;
  return Number(result.ai_review_refs_total ?? 0) > 0;
}

function extractJsonObject(text) {
  const trimmed = text.trim();
  if (trimmed.startsWith("{") && trimmed.endsWith("}")) return JSON.parse(trimmed);
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/i);
  if (fenced) return JSON.parse(fenced[1]);
  const start = trimmed.indexOf("{");
  const end = trimmed.lastIndexOf("}");
  if (start >= 0 && end > start) return JSON.parse(trimmed.slice(start, end + 1));
  throw new Error(`DeepSeek response did not contain JSON: ${trimmed.slice(0, 1000)}`);
}

function stringArray(value, limit) {
  if (!Array.isArray(value)) return [];
  return value
    .filter((item) => typeof item === "string")
    .map((item) => item.trim())
    .filter(Boolean)
    .slice(0, limit);
}

function sanitizeRecipe(recipe) {
  if (!recipe || typeof recipe !== "object") return {};
  const chunkPairs = Array.isArray(recipe.chunk_pairs)
    ? recipe.chunk_pairs
        .map((item) => ({
          id: String(item?.id ?? "").trim(),
          hash: String(item?.hash ?? "").trim(),
        }))
        .filter((item) => item.id && item.hash)
        .slice(0, 100)
    : [];
  return {
    manifest_paths: stringArray(recipe.manifest_paths, 20),
    script_urls: stringArray(recipe.script_urls, 50),
    routes: stringArray(recipe.routes, 20),
    click_texts: stringArray(recipe.click_texts, 20),
    public_path: typeof recipe.public_path === "string" ? recipe.public_path.trim() : undefined,
    chunk_pairs: chunkPairs,
  };
}

function recipeHasWork(recipe) {
  return (
    recipe.manifest_paths?.length > 0 ||
    recipe.script_urls?.length > 0 ||
    recipe.routes?.length > 0 ||
    recipe.click_texts?.length > 0 ||
    recipe.chunk_pairs?.length > 0
  );
}

async function askDeepSeekForRecipe(result, args) {
  const apiKey = process.env.DEEPSEEK_API_KEY || String(args.api_key || "");
  if (!apiKey) {
    return {
      status: "skipped",
      reason: "missing DEEPSEEK_API_KEY",
      recipe: {},
    };
  }
  const baseUrl = String(args.base_url || process.env.DEEPSEEK_BASE_URL || DEFAULT_DEEPSEEK_BASE_URL)
    .replace(/\/+$/, "");
  const model = String(args.model || process.env.DEEPSEEK_MODEL || DEFAULT_MODEL);
  const system = [
    "You are helping an authorized JavaScript/API collection tool decide a bounded second-pass recipe.",
    "The browser/helper already fetched and verified real scripts. Do not invent URLs.",
    "Relative refs like ./af.js or ../utils/request.js are often bundled module specifiers; prefer no fetch unless evidence suggests a deployed file.",
    "Do not suggest script_urls that already appear in scripts. Do not suggest URLs already shown in recursive_errors.",
    "If a second pass would only repeat already fetched scripts or already failed chunk candidates, set needs_second_pass=false.",
    "Return strict JSON with keys: status, needs_second_pass, recipe, discard_refs, rationale.",
    "recipe may contain only manifest_paths, script_urls, routes, click_texts, public_path, chunk_pairs.",
    "Choose the smallest recipe. If deterministic collection is enough, set needs_second_pass=false and empty recipe.",
  ].join("\n");
  const response = await fetch(`${baseUrl}/chat/completions`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model,
      temperature: 0,
      messages: [
        { role: "system", content: system },
        { role: "user", content: JSON.stringify(compactCollection(result)) },
      ],
    }),
  });
  const bodyText = await response.text();
  if (!response.ok) {
    return {
      status: "error",
      http_status: response.status,
      message: bodyText.slice(0, 2000),
      recipe: {},
    };
  }
  const body = JSON.parse(bodyText);
  const content = body?.choices?.[0]?.message?.content ?? "";
  const parsed = extractJsonObject(content);
  return {
    status: parsed.status || "ok",
    provider: "deepseek",
    model,
    needs_second_pass: Boolean(parsed.needs_second_pass),
    recipe: sanitizeRecipe(parsed.recipe),
    discard_refs: Array.isArray(parsed.discard_refs) ? parsed.discard_refs.slice(0, 100) : [],
    rationale: parsed.rationale || "",
  };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const url = String(args.url || DEFAULT_URL);
  const workspace = path.resolve(String(args.workspace || defaultWorkspace(url)));
  await fs.mkdir(workspace, { recursive: true });

  const initial = runCollector(url, workspace, args);
  const ai = shouldUseAi(initial, args)
    ? await askDeepSeekForRecipe(initial, args)
    : {
        status: "skipped",
        reason: "no ai_review_refs and --force-ai not set",
        recipe: {},
      };
  const recipe = sanitizeRecipe(ai.recipe);
  const second_pass =
    ai.needs_second_pass && recipeHasWork(recipe)
      ? runCollector(url, workspace, args, recipe)
      : null;

  const outputDir = path.join(
    workspace,
    ".golish",
    "js-ai-recipe-probe",
    new Date().toISOString().replace(/[:.]/g, "-"),
  );
  await fs.mkdir(outputDir, { recursive: true });
  const summary = {
    status: "ok",
    url,
    workspace,
    output_dir: outputDir,
    initial,
    ai,
    second_pass,
  };
  await fs.writeFile(path.join(outputDir, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);

  process.stdout.write(
    `${JSON.stringify(
      {
        status: "ok",
        url,
        workspace,
        output_dir: outputDir,
        initial: compactCollection(initial),
        ai: {
          status: ai.status,
          provider: ai.provider,
          model: ai.model,
          needs_second_pass: ai.needs_second_pass,
          recipe,
          rationale: ai.rationale,
          discard_refs_count: ai.discard_refs?.length ?? 0,
        },
        second_pass: second_pass ? compactCollection(second_pass) : null,
      },
      null,
      2,
    )}\n`,
  );
}

main().catch((error) => {
  console.error(error.stack || error.message);
  process.exit(1);
});
