#!/usr/bin/env node
import { chromium } from "@playwright/test";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const NO_LIMIT = Number.POSITIVE_INFINITY;
const DEFAULT_TIMEOUT_MS = 60_000;
const DEFAULT_HARD_TIMEOUT_MS = 120_000;
const DEFAULT_MAX_PAGES = 100;
const DEFAULT_MAX_ACTIONS = 0;
const DEFAULT_MAX_SCRIPT_BYTES = NO_LIMIT;
const DEFAULT_MAX_RECURSIVE_SCRIPTS = NO_LIMIT;
const DEFAULT_FETCH_TIMEOUT_MS = NO_LIMIT;
const DEFAULT_BODY_TIMEOUT_MS = NO_LIMIT;
const MAX_API_CAPTURE_BYTES = 512_000;
const DEFAULT_CONTEXT_CLOSE_TIMEOUT_MS = 2_000;
const DEFAULT_CLOSE_TIMEOUT_MS = 3_000;
const MAX_RECOVERY_FAILURES = 2;
const TIMEOUT = Symbol("timeout");

function progress(label, fields = {}) {
  const suffix = Object.entries(fields)
    .filter(([, value]) => value !== undefined && value !== null && value !== "")
    .map(([key, value]) => `${key}=${String(value).replace(/\s+/g, " ").slice(0, 180)}`)
    .join(" ");
  process.stderr.write(`[browser_collect_js_api] ${label}${suffix ? ` ${suffix}` : ""}\n`);
}

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

function toInt(value, fallback, min, max) {
  const n = Number.parseInt(String(value ?? ""), 10);
  if (!Number.isFinite(n)) return fallback;
  return Math.max(min, Math.min(max, n));
}

function toLimit(value, fallback, min, max) {
  if (value == null || value === true || value === "") return fallback;
  const n = Number.parseInt(String(value), 10);
  if (!Number.isFinite(n) || n <= 0) return NO_LIMIT;
  return Math.max(min, Math.min(max, n));
}

export function boundedHardTimeoutMs(value) {
  const n = Number.parseInt(String(value ?? ""), 10);
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_HARD_TIMEOUT_MS;
  return Math.max(10_000, Math.min(600_000, n));
}

function limitForJson(value) {
  return Number.isFinite(value) ? value : null;
}

function limitLabel(value) {
  return Number.isFinite(value) ? value : "unlimited";
}

function toBool(value, fallback) {
  if (value == null) return fallback;
  if (typeof value === "boolean") return value;
  return ["1", "true", "yes", "y"].includes(String(value).toLowerCase());
}

export function isExactOriginUrl(value, exactOrigin) {
  try {
    return new URL(value).origin === exactOrigin;
  } catch {
    return false;
  }
}

export function isExactOriginWebSocketUrl(value, targetUrl) {
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") return false;
    parsed.protocol = parsed.protocol === "wss:" ? "https:" : "http:";
    return parsed.origin === targetUrl.origin;
  } catch {
    return false;
  }
}

export function captureDirectoryFor(workspace, targetUrl, kind) {
  const parsed = targetUrl instanceof URL ? targetUrl : new URL(targetUrl);
  const scheme = parsed.protocol.replace(/:$/, "");
  if (scheme !== "http" && scheme !== "https") {
    throw new Error(`unsupported capture scheme: ${scheme}`);
  }
  const port = parsed.port || (scheme === "http" ? "80" : "443");
  return path.join(
    workspace,
    ".golish",
    "captures",
    parsed.hostname || "unknown",
    String(port),
    scheme,
    kind,
  );
}

function count(value) {
  const parsed = Number(value ?? 0);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

export function classifyCollectionCompletion(input = {}) {
  const navigationAttempts = count(input.navigation_attempts);
  const successfulPages = count(input.successful_pages);
  const navigationErrors = count(input.navigation_errors);
  const pageQueueRemaining = count(input.page_queue_remaining);
  const pageCandidatesDropped = count(input.page_candidates_dropped);
  const recursiveQueueRemaining = count(input.recursive_queue_remaining);
  const scriptByteLimitSkips = count(input.script_byte_limit_skips);
  const scriptCaptureErrors = count(input.script_capture_errors);
  const scopeViolations = count(input.scope_violations);
  const pendingBodyTimeouts = count(input.pending_body_timeouts);
  const recoveryPending = count(input.recovery_pending);
  const recoveryExhausted = count(input.recovery_exhausted);
  const reasons = [];

  if (navigationAttempts > 0 && successfulPages === 0) {
    reasons.push("all_navigation_failed");
  } else if (navigationErrors > 0) {
    reasons.push("navigation_errors");
  }
  if (pageQueueRemaining > 0) reasons.push("page_queue_remaining");
  if (pageCandidatesDropped > 0) reasons.push("page_budget_truncated");
  if (recursiveQueueRemaining > 0) reasons.push("recursive_queue_remaining");
  if (input.recursive_limit_hit) reasons.push("max_recursive_scripts_hit");
  if (input.recursive_deadline_hit) reasons.push("recursive_deadline_hit");
  if (input.hard_deadline_hit) reasons.push("hard_deadline_hit");
  if (input.pending_wait_timed_out) reasons.push("pending_wait_timed_out");
  if (pendingBodyTimeouts > 0) reasons.push("pending_body_timeouts");
  if (scriptByteLimitSkips > 0) reasons.push("max_script_bytes_hit");
  if (scriptCaptureErrors > 0) reasons.push("script_capture_errors");
  if (scopeViolations > 0) reasons.push("exact_origin_scope_violations");
  if (recoveryPending > recoveryExhausted) reasons.push("recovery_pending");
  if (recoveryExhausted > 0) reasons.push("recovery_exhausted");

  if (
    reasons.includes("all_navigation_failed") ||
    reasons.includes("recovery_exhausted")
  ) {
    return {
      status: "error",
      completion_state: "error",
      closure_complete: false,
      reasons,
    };
  }
  if (reasons.length > 0) {
    const timedOut =
      Boolean(input.hard_deadline_hit) ||
      Boolean(input.pending_wait_timed_out) ||
      pendingBodyTimeouts > 0;
    return {
      status: timedOut ? "timeout_partial" : "closure_partial",
      completion_state: "partial",
      closure_complete: false,
      reasons,
    };
  }
  return {
    status: "ok",
    completion_state: "complete",
    closure_complete: true,
    reasons: [],
  };
}

function parseCrawlMode(_value) {
  return "standard";
}

function parseRecipe(value) {
  if (!value) return {};
  try {
    const parsed = JSON.parse(String(value));
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed
      : {};
  } catch {
    return {};
  }
}

function safeStringArray(value, limit, maxLength = 300) {
  if (!Array.isArray(value)) return [];
  return value
    .filter((item) => typeof item === "string")
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item) => item.slice(0, maxLength))
    .slice(0, limit);
}

function safeChunkPairs(value, limit) {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => ({
      id: String(item?.id ?? "").trim(),
      hash: String(item?.hash ?? "").trim(),
    }))
    .filter((item) => /^\d{1,8}$/.test(item.id) && /^[a-f0-9]{4,64}$/i.test(item.hash))
    .slice(0, limit);
}

function recipeSchema() {
  return {
    manifest_paths: ["same-origin manifest path such as /asset-manifest.json"],
    script_urls: ["same-origin or already-observed-origin JS URL/path to fetch"],
    routes: ["same-origin route/path/hash route to visit in the browser"],
    click_texts: [
      "disabled in Enumeration until a separate explicit high-risk interaction authorization exists",
    ],
    public_path: "optional public path override for chunk_pairs",
    chunk_pairs: [{ id: "123", hash: "abcdef1234" }],
  };
}

async function fetchWithTimeout(url, options = {}, timeoutMs = DEFAULT_FETCH_TIMEOUT_MS) {
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    return await fetch(url, options);
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, {
      ...options,
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timer);
  }
}

export async function fetchExactOrigin(
  url,
  exactOrigin,
  options = {},
  timeoutMs = DEFAULT_FETCH_TIMEOUT_MS,
) {
  let current = new URL(url);
  for (let hop = 0; hop <= 5; hop += 1) {
    if (current.origin !== exactOrigin) {
      throw new Error(`exact-origin request blocked: ${current.href}`);
    }
    if (isDangerousNavigationUrl(current)) {
      throw new Error(`read-only fetch blocked: ${current.href}`);
    }
    const response = await fetchWithTimeout(
      current.href,
      { ...options, redirect: "manual" },
      timeoutMs,
    );
    const location = response.headers.get("location");
    const isRedirect = [301, 302, 303, 307, 308].includes(response.status);
    if (!isRedirect || !location) {
      if (!isExactOriginUrl(response.url || current.href, exactOrigin)) {
        throw new Error(`exact-origin final URL blocked: ${response.url}`);
      }
      return response;
    }
    const next = new URL(location, current);
    if (next.origin !== exactOrigin) {
      throw new Error(`exact-origin redirect blocked: ${current.href} -> ${next.href}`);
    }
    if (isDangerousNavigationUrl(next)) {
      throw new Error(`read-only redirect blocked: ${current.href} -> ${next.href}`);
    }
    if (hop === 5) {
      throw new Error(`exact-origin redirect limit exceeded: ${url}`);
    }
    current = next;
  }
  throw new Error(`exact-origin fetch failed: ${url}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function withTimeout(promise, timeoutMs) {
  const guarded = Promise.resolve(promise);
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    return await guarded;
  }
  let timer;
  try {
    return await Promise.race([
      guarded,
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(TIMEOUT), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function timeLeft(deadlineMs, capMs) {
  const remaining = Number.isFinite(deadlineMs)
    ? Math.max(0, deadlineMs - Date.now())
    : NO_LIMIT;
  if (capMs == null || !Number.isFinite(capMs) || capMs <= 0) return remaining;
  return Math.min(capMs, remaining);
}

function redirectDepth(request) {
  let depth = 0;
  let previous = request.redirectedFrom();
  while (previous) {
    depth += 1;
    previous = previous.redirectedFrom();
  }
  return depth;
}

function isNoiseUrl(url) {
  return /(?:googletagmanager|google-analytics|doubleclick|cdn\.usefathom|plausible|hotjar|segment|clarity\.ms|sentry\.io)/i.test(
    url,
  );
}

function isBlockedResourceType(type) {
  return type === "image" || type === "media" || type === "font" || type === "stylesheet";
}

async function closeBrowserHard(browser) {
  const result = await withTimeout(browser.close(), DEFAULT_CLOSE_TIMEOUT_MS);
  if (result !== TIMEOUT) return false;
  try {
    browser.process?.()?.kill("SIGKILL");
  } catch {
    // Best effort for channel-launched browsers where process() may be absent.
  }
  return true;
}

async function closeContextHard(context) {
  const result = await withTimeout(context.close(), DEFAULT_CONTEXT_CLOSE_TIMEOUT_MS);
  return result === TIMEOUT;
}

async function fulfillSafeNavigationOnce(route, exactOrigin, timeoutMs) {
  const current = new URL(route.request().url());
  if (current.origin !== exactOrigin) {
    return { allowed: false, blocked_url: current.href };
  }
  const response = await route.fetch({
    maxRedirects: 0,
    timeout: Number.isFinite(timeoutMs) ? Math.max(1, timeoutMs) : 0,
  });
  const status = response.status();
  const location = response.headers().location;
  if ([301, 302, 303, 307, 308].includes(status) && location) {
    const next = new URL(location, current);
    if (next.origin !== exactOrigin) {
      const terminalRedirect = {
        from: current.href,
        to: next.href,
        status,
      };
      try {
        // The authorized origin was contacted exactly once and answered with a
        // terminal redirect outside the approved origin. Preserve that fact as
        // evidence, but replace the browser-visible response with a local empty
        // document so Playwright never schedules the foreign hop. This is a
        // completed scope decision, not a DNS/TLS/navigation failure.
        await route.fulfill({
          status: 200,
          contentType: "text/html; charset=utf-8",
          body: "<!doctype html><title>Golish exact-origin redirect boundary</title>",
        });
      } finally {
        await response.dispose();
      }
      return {
        allowed: true,
        final_url: current.href,
        terminal_redirect: terminalRedirect,
      };
    }
  }
  try {
    // `route.fetch()` already performed the one authorized request. Fulfill
    // from that response instead of `route.continue()` (which would send it a
    // second time). Redirect hops are routed and checked independently.
    await route.fulfill({ response });
  } finally {
    await response.dispose();
  }
  return { allowed: true, final_url: current.href };
}

async function writeJsonAndExit(value) {
  const text = JSON.stringify(value, null, 2);
  await new Promise((resolve, reject) => {
    process.stdout.write(text, (error) => {
      if (error) reject(error);
      else resolve();
    });
  });
  process.exit(0);
}

function sanitizeFilename(name) {
  return name.replace(/[\/\\:*?"<>|\0]/g, "_") || "unknown.js";
}

function sha256Prefix(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex").slice(0, 8);
}

function sha256Hex(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function canonicalScriptUrl(urlString) {
  try {
    const url = new URL(urlString);
    url.hash = "";
    url.searchParams.sort();
    return url.href;
  } catch {
    return String(urlString || "");
  }
}

function safeParentFromUrlPath(urlPath) {
  const clean = urlPath.replace(/^\/+/, "");
  const parent = path.posix.dirname(clean);
  if (!parent || parent === ".") return "";
  return parent.replace(/\.\./g, "_").replace(/:/g, "_");
}

function isJavaScriptResponse(urlString, headers) {
  const contentType = headers["content-type"] ?? "";
  if (/javascript|ecmascript|text\/js/i.test(contentType)) return true;
  if (/text\/html|application\/xhtml\+xml/i.test(contentType)) return false;
  try {
    const pathname = new URL(urlString).pathname.toLowerCase();
    return pathname.endsWith(".js") || pathname.endsWith(".mjs");
  } catch {
    return false;
  }
}

function looksLikeJsRef(value) {
  if (!value || /\s/.test(value)) return false;
  return (
    value.includes("/") ||
    value.endsWith(".js") ||
    value.endsWith(".mjs") ||
    value.endsWith(".cjs")
  );
}

function isBundledModuleSpecifier(ref) {
  return ref.startsWith("./") || ref.startsWith("../");
}

function scanJsForReferenceCandidates(text, options = {}) {
  const allowRelativeModules = Boolean(options.allowRelativeModules);
  const patterns = [
    /import\s*\(\s*["']([^"']+\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']\s*\)/g,
    /["']((?:\.{0,2}\/)[^"']*?\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
    /["']((?:assets|static|js|dist|build|chunks?|vendor|_next\/static)\/[^"']*?\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
    /\b[A-Za-z_$][A-Za-z0-9_$]*\s*\+\s*["']([^"']*\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
    /["']([^"']*\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']\s*\+\s*[A-Za-z_$][A-Za-z0-9_$]*/g,
    /\b(?:src|file|path|chunk|url)["']?\s*:\s*["']([^"']+\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
  ];

  const autoRefs = [];
  const aiReviewRefs = [];
  const seen = new Set();
  const reviewSeen = new Set();
  for (const [patternIndex, pattern] of patterns.entries()) {
    for (const match of text.matchAll(pattern)) {
      const ref = match[1];
      if (
        patternIndex !== 0 &&
        !allowRelativeModules &&
        isBundledModuleSpecifier(ref)
      ) {
        if (!reviewSeen.has(ref)) {
          reviewSeen.add(ref);
          aiReviewRefs.push(ref);
        }
        continue;
      }
      if (looksLikeJsRef(ref) && !seen.has(ref)) {
        seen.add(ref);
        autoRefs.push(ref);
      }
    }
  }
  return { auto_refs: autoRefs, ai_review_refs: aiReviewRefs };
}

function scanJsForReferences(text, options = {}) {
  return scanJsForReferenceCandidates(text, options).auto_refs;
}

function extractInterestingSnippets(text) {
  const needles = [
    "import(",
    "__webpack_require__",
    ".p=",
    ".u=",
    "chunk",
    ".js",
    "fetch(",
    "axios",
    "graphql",
    "/api/",
  ];
  const snippets = [];
  const lower = text.toLowerCase();
  for (const needle of needles) {
    const idx = lower.indexOf(needle.toLowerCase());
    if (idx === -1) continue;
    const start = Math.max(0, idx - 160);
    const end = Math.min(text.length, idx + 260);
    snippets.push(text.slice(start, end).replace(/\s+/g, " ").slice(0, 500));
    if (snippets.length >= 5) break;
  }
  return snippets;
}

function extractPublicPath(text) {
  const patterns = [
    /__webpack_require__\s*\.\s*p\s*=\s*["']([^"']*)["']/,
    /\b[a-zA-Z_$][a-zA-Z0-9_$]*\s*\.\s*p\s*=\s*["']([^"']*)["']\s*[,;]/,
    /["']publicPath["']\s*:\s*["']([^"']+)["']/,
  ];
  for (const pattern of patterns) {
    const match = pattern.exec(text);
    const value = match?.[1]?.trim();
    if (value) return value;
  }
  return null;
}

function ensureTrailingSlash(value) {
  return value.endsWith("/") ? value : `${value}/`;
}

function resolvePublicPath(publicPath, baseUrl) {
  const base = new URL(baseUrl);
  if (!publicPath) return ensureTrailingSlash(base.href);
  if (/^https?:\/\//i.test(publicPath)) {
    return ensureTrailingSlash(publicPath);
  }
  if (publicPath.startsWith("/")) {
    return ensureTrailingSlash(`${base.origin}${publicPath}`);
  }
  return ensureTrailingSlash(new URL(publicPath, baseUrl).href);
}

function expandWebpackChunkMap(text, publicPath, baseUrl) {
  const pairPattern = /["']?(\d+)["']?\s*:\s*["']([a-f0-9]{4,40})["']/g;
  const maps = [];
  let current = [];
  let lastEnd = null;
  for (const match of text.matchAll(pairPattern)) {
    const [whole, id, hash] = match;
    const start = match.index ?? 0;
    if (lastEnd != null) {
      const gap = text.slice(lastEnd, start);
      if (
        gap.length > 80 ||
        gap.includes(";") ||
        gap.includes("\n") ||
        gap.includes("function")
      ) {
        if (current.length >= 2) maps.push(current);
        current = [];
      }
    }
    current.push([id, hash]);
    lastEnd = start + whole.length;
  }
  if (current.length >= 2) maps.push(current);

  const base = resolvePublicPath(publicPath, baseUrl);
  const urls = [];
  for (const map of maps) {
    for (const [id, hash] of map) {
      urls.push(`${base}${id}.${hash}.js`);
    }
  }
  return urls;
}

function extractQuotedJsReferences(text) {
  const refs = [];
  const seen = new Set();
  for (const match of text.matchAll(/["'`]([^"'`]+?\.(?:js|mjs|cjs)(?:\?[^"'`]*)?)["'`]/g)) {
    const ref = match[1];
    if (looksLikeJsRef(ref) && !seen.has(ref)) {
      seen.add(ref);
      refs.push(ref);
    }
  }
  return refs;
}

function parseNumericStringEntries(objectText) {
  const entries = [];
  for (const match of objectText.matchAll(/["']?(\d+)["']?\s*:\s*["']([^"']+)["']/g)) {
    entries.push([match[1], match[2]]);
  }
  return entries;
}

function extractNumericStringMaps(segment) {
  const maps = [];
  const mapPattern = /\{([^{}]{12,6000})\}/g;
  for (const match of segment.matchAll(mapPattern)) {
    const entries = parseNumericStringEntries(match[1]);
    if (entries.length >= 2) {
      maps.push({
        index: match.index ?? 0,
        entries,
        map: new Map(entries),
      });
    }
  }
  return maps;
}

function isHashLike(value) {
  return /^[a-f0-9]{4,80}$/i.test(value);
}

function extractRuntimePrefix(segment, firstMapIndex) {
  const beforeMap = segment.slice(0, firstMapIndex);
  const strings = [...beforeMap.matchAll(/["'`]([^"'`]*)["'`]/g)].map(
    (match) => match[1],
  );
  return strings.at(-1) ?? "";
}

function expandRuntimeChunkUrls(text, publicPath, baseUrl) {
  const urls = [];
  const seen = new Set();
  const runtimePattern = /\b[A-Za-z_$][A-Za-z0-9_$]*\s*\.\s*u\s*=\s*[A-Za-z_$][A-Za-z0-9_$]*\s*=>/g;
  for (const match of text.matchAll(runtimePattern)) {
    const start = match.index ?? 0;
    const window = text.slice(start, start + 14_000);
    const jsIndex = window.indexOf(".js");
    if (jsIndex === -1) continue;
    const segment = window.slice(0, Math.min(window.length, jsIndex + 1_000));
    const maps = extractNumericStringMaps(segment);
    if (maps.length === 0) continue;

    const hashMap =
      [...maps].reverse().find((map) =>
        map.entries.some(([, value]) => isHashLike(value)),
      ) ?? maps.at(-1);
    if (!hashMap) continue;
    const hashMapIndex = maps.indexOf(hashMap);
    const nameMap =
      maps.length >= 2 && hashMapIndex > 0 ? maps[hashMapIndex - 1]?.map : null;
    const prefix = extractRuntimePrefix(segment, maps[0].index);
    const base = resolvePublicPath(publicPath, baseUrl);

    for (const [id, hash] of hashMap.entries) {
      if (!isHashLike(hash)) continue;
      const name = nameMap?.get(id) || id;
      const url = resolveScriptReference(`${prefix}${name}.${hash}.js`, base);
      if (url && !seen.has(url)) {
        seen.add(url);
        urls.push(url);
      }
    }
  }
  return urls;
}

function expandViteMapDeps(text, baseUrl) {
  const urls = [];
  const seen = new Set();
  const pattern = /__vite__mapDeps[\s\S]{0,3000}?m\.f\s*\|\|\s*\(\s*m\.f\s*=\s*\[([\s\S]*?)\]\s*\)/g;
  for (const match of text.matchAll(pattern)) {
    for (const ref of extractQuotedJsReferences(match[1])) {
      const url = resolveScriptReference(ref, baseUrl);
      if (url && !seen.has(url)) {
        seen.add(url);
        urls.push(url);
      }
    }
  }
  return urls;
}

function collectJsStringsFromJson(value, refs = []) {
  if (typeof value === "string") {
    if (looksLikeJsRef(value) && /\.(?:js|mjs|cjs)(?:$|\?)/i.test(value)) {
      refs.push(value);
    }
    return refs;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      collectJsStringsFromJson(item, refs);
    }
    return refs;
  }
  if (value && typeof value === "object") {
    for (const item of Object.values(value)) {
      collectJsStringsFromJson(item, refs);
    }
  }
  return refs;
}

function extractJsonManifestReferences(body, manifestUrl) {
  try {
    const parsed = JSON.parse(body);
    const refs = collectJsStringsFromJson(parsed);
    const manifest = new URL(manifestUrl);
    if (manifest.pathname === "/_nuxt/builds/latest.json") {
      const id = typeof parsed?.id === "string" ? parsed.id.trim() : "";
      if (/^[A-Za-z0-9_.-]{4,160}$/.test(id)) {
        refs.push(`/_nuxt/builds/${id}.json`);
      }
    }
    return refs;
  } catch {
    return [];
  }
}

function resolveManifestReference(ref, manifestUrl) {
  try {
    if (ref.startsWith("//")) {
      return `${new URL(manifestUrl).protocol}${ref}`;
    }
    const manifest = new URL(manifestUrl);
    if (/^https?:\/\//i.test(ref)) {
      return new URL(ref).href;
    }
    if (ref.startsWith("/")) {
      return new URL(ref, manifest.origin).href;
    }
    if (manifest.pathname.includes("/_next/static/") && ref.startsWith("static/")) {
      return new URL(`/_next/${ref}`, manifest.origin).href;
    }
    if (manifest.pathname.startsWith("/_nuxt/builds/") && /\.(?:js|mjs|cjs)(?:$|\?)/i.test(ref)) {
      return new URL(`/_nuxt/${ref}`, manifest.origin).href;
    }
    return new URL(ref, manifestUrl).href;
  } catch {
    return null;
  }
}

function resolveScriptReference(ref, sourceUrl) {
  try {
    if (ref.startsWith("//")) {
      return `${new URL(sourceUrl).protocol}${ref}`;
    }
    return new URL(ref, sourceUrl).href;
  } catch {
    return null;
  }
}

function outputFilenameForScript(scriptUrl, body) {
  let basename = "unknown.js";
  try {
    const url = new URL(scriptUrl);
    basename = path.posix.basename(url.pathname) || basename;
  } catch {
    basename = scriptUrl.split("/").pop() || basename;
  }
  if (!/\.(mjs|js)$/i.test(basename)) {
    basename = `${basename}.js`;
  }
  return `${sha256Prefix(body)}_${sanitizeFilename(basename)}`;
}

function outputFilenameForApiCapture(method, urlString) {
  let basename = "api";
  try {
    const url = new URL(urlString);
    basename = path.posix.basename(url.pathname) || basename;
  } catch {
    basename = String(urlString || basename).split("/").pop() || basename;
  }
  const hash = crypto.createHash("sha256").update(`${method} ${urlString}`).digest("hex").slice(0, 12);
  return `${method.toLowerCase()}_${hash}_${sanitizeFilename(basename)}.json`;
}

function textualBodySample(contentType, body) {
  if (!body || body.length === 0) return "";
  if (!/(?:text|json|javascript|xml|x-www-form-urlencoded|graphql)/i.test(contentType || "")) {
    return "";
  }
  return body.toString("utf8").slice(0, 64_000);
}

async function saveApiResponseCapture(
  workspace,
  targetHost,
  targetPort,
  targetScheme,
  entry,
  headers,
  body,
  extra = {},
) {
  let urlPath = "/api";
  try {
    urlPath = new URL(entry.url).pathname || urlPath;
  } catch {
    // keep fallback
  }
  const parent = safeParentFromUrlPath(urlPath);
  const relativeDir = path.join(
    ".golish",
    "captures",
    targetHost,
    String(targetPort),
    targetScheme,
    "api",
    parent,
  );
  const dir = path.join(workspace, relativeDir);
  await fs.mkdir(dir, { recursive: true });
  const fullPath = path.join(dir, outputFilenameForApiCapture(entry.method, entry.url));
  const contentType = headers["content-type"] ?? "";
  const bodyBuffer = Buffer.isBuffer(body) ? body : null;
  const requestBody = typeof entry.request_body === "string" ? entry.request_body : null;
  const payload = {
    version: 2,
    captured_at: new Date().toISOString(),
    request: {
      method: entry.method,
      url: entry.url,
      resource_type: entry.resource_type,
      headers: entry.request_headers ?? {},
      body: requestBody && requestBody.length <= 64_000 ? requestBody : null,
      body_truncated: Boolean(requestBody && requestBody.length > 64_000),
    },
    response: {
      status: entry.status,
      headers,
      content_type: contentType,
      body_len: bodyBuffer?.length ?? 0,
      body_sha256: bodyBuffer ? sha256Hex(bodyBuffer) : null,
      body_text_sample: textualBodySample(contentType, bodyBuffer),
      body_base64: bodyBuffer ? bodyBuffer.toString("base64") : null,
      ...extra,
    },
  };
  await fs.writeFile(fullPath, `${JSON.stringify(payload, null, 2)}\n`);
  return path.relative(workspace, fullPath);
}

async function saveScriptCapture(
  workspace,
  targetHost,
  targetPort,
  targetScheme,
  scriptUrl,
  body,
) {
  let urlPath = "/unknown.js";
  try {
    urlPath = new URL(scriptUrl).pathname || urlPath;
  } catch {
    // keep fallback
  }
  const parent = safeParentFromUrlPath(urlPath);
  const relativeDir = path.join(
    ".golish",
    "captures",
    targetHost,
    String(targetPort),
    targetScheme,
    "js",
    parent,
  );
  const dir = path.join(workspace, relativeDir);
  await fs.mkdir(dir, { recursive: true });
  const filename = outputFilenameForScript(scriptUrl, body);
  const fullPath = path.join(dir, filename);
  await fs.writeFile(fullPath, body);
  return path.relative(workspace, fullPath);
}

function scriptManifestPath(workspace, targetHost, targetPort, targetScheme) {
  return path.join(
    workspace,
    ".golish",
    "captures",
    targetHost,
    String(targetPort),
    targetScheme,
    "js",
    "manifest.json",
  );
}

async function loadScriptManifest(
  workspace,
  targetHost,
  targetPort,
  targetScheme,
  maxScriptBytes,
) {
  const manifestPath = scriptManifestPath(workspace, targetHost, targetPort, targetScheme);
  try {
    const text = await fs.readFile(manifestPath, "utf8");
    const parsed = JSON.parse(text);
    const scripts = Array.isArray(parsed?.scripts)
      ? parsed.scripts
      : Array.isArray(parsed)
        ? parsed
        : [];
    const entries = [];
    let stale = 0;
    let sizeLimited = 0;
    for (const item of scripts) {
      if (!item || typeof item !== "object") continue;
      const url = typeof item.url === "string" ? item.url : "";
      const diskPath = typeof item.path === "string" ? item.path : "";
      if (!url || !diskPath) continue;
      const fullPath = path.resolve(workspace, diskPath);
      let stat;
      try {
        stat = await fs.stat(fullPath);
      } catch {
        stale += 1;
        continue;
      }
      if (!stat.isFile()) {
        stale += 1;
        continue;
      }
      if (Number.isFinite(maxScriptBytes) && stat.size > maxScriptBytes) {
        stale += 1;
        sizeLimited += 1;
        continue;
      }
      entries.push({
        url,
        key: canonicalScriptUrl(url),
        path: diskPath,
        size: stat.size,
        status: item.status ?? 200,
        content_type: item.content_type ?? "",
        sha256: typeof item.sha256 === "string" ? item.sha256 : null,
        cached: true,
        full_path: fullPath,
      });
    }
    return {
      path: manifestPath,
      entries,
      raw_entries: scripts.length,
      stale,
      size_limited: sizeLimited,
      producer_run_id:
        typeof parsed?.producer_run_id === "string" ? parsed.producer_run_id : null,
      producer_session_id:
        typeof parsed?.producer_session_id === "string" ? parsed.producer_session_id : null,
      producer_operation_id:
        typeof parsed?.producer_operation_id === "string"
          ? parsed.producer_operation_id
          : null,
      producer_stage_started_at:
        typeof parsed?.producer_stage_started_at === "string"
          ? parsed.producer_stage_started_at
          : null,
      captured_at: typeof parsed?.captured_at === "string" ? parsed.captured_at : null,
      completion_state:
        typeof parsed?.completion_state === "string" ? parsed.completion_state : null,
      closure_complete: parsed?.closure_complete === true,
      closure_incomplete_reasons: Array.isArray(parsed?.closure_incomplete_reasons)
        ? parsed.closure_incomplete_reasons.filter((reason) => typeof reason === "string")
        : [],
      visited_pages: Array.isArray(parsed?.visited_pages)
        ? parsed.visited_pages.filter((url) => typeof url === "string")
        : [],
      pending_pages: Array.isArray(parsed?.pending_pages)
        ? parsed.pending_pages.filter((url) => typeof url === "string")
        : [],
      pending_recursive_scripts: Array.isArray(parsed?.pending_recursive_scripts)
        ? parsed.pending_recursive_scripts.filter(
            (item) =>
              typeof item === "string" ||
              (item && typeof item === "object" && !Array.isArray(item)),
          )
        : [],
      api_requests: Array.isArray(parsed?.api_requests)
        ? parsed.api_requests.filter(
            (request) => request && typeof request === "object" && !Array.isArray(request),
          )
        : [],
      page_resume_count: Number.isInteger(parsed?.page_resume_count)
        ? Math.max(0, parsed.page_resume_count)
        : 0,
      checkpoint_resume_count: Number.isInteger(parsed?.checkpoint_resume_count)
        ? Math.max(0, parsed.checkpoint_resume_count)
        : Number.isInteger(parsed?.page_resume_count)
          ? Math.max(0, parsed.page_resume_count)
          : 0,
      recovery_failures: Array.isArray(parsed?.recovery_failures)
        ? parsed.recovery_failures.filter(
            (item) => item && typeof item === "object" && !Array.isArray(item),
          )
        : [],
      recovery_exhausted: parsed?.recovery_exhausted === true,
      automatic_retry_allowed: parsed?.automatic_retry_allowed !== false,
    };
  } catch {
    return {
      path: manifestPath,
      entries: [],
      raw_entries: 0,
      stale: 0,
      size_limited: 0,
      producer_run_id: null,
      producer_session_id: null,
      producer_operation_id: null,
      producer_stage_started_at: null,
      captured_at: null,
      completion_state: null,
      closure_complete: false,
      closure_incomplete_reasons: [],
      visited_pages: [],
      pending_pages: [],
      pending_recursive_scripts: [],
      api_requests: [],
      page_resume_count: 0,
      checkpoint_resume_count: 0,
      recovery_failures: [],
      recovery_exhausted: false,
      automatic_retry_allowed: true,
    };
  }
}

const RECOVERABLE_CHECKPOINT_REASONS = new Set([
  "page_queue_remaining",
  "recursive_queue_remaining",
  "max_recursive_scripts_hit",
  "recursive_deadline_hit",
  "all_navigation_failed",
  "navigation_errors",
  "hard_deadline_hit",
  "pending_wait_timed_out",
  "pending_body_timeouts",
  "script_capture_errors",
  "recovery_pending",
  "recovery_exhausted",
]);
const RECOVERY_KINDS = new Set([
  "navigation",
  "page_inspection",
  "script_body",
  // Kept as a recognized legacy checkpoint kind so an existing v2 manifest
  // can be sanitized instead of rejected wholesale. API response-body capture
  // is diagnostic, though: URL/method/status metadata already proves the
  // observation and a missing body must not keep JS closure pending.
  "api_body",
  "manifest_body",
  "recursive_body",
  "recursive_fetch",
  "pending_wait",
]);

export function recoveryKindBlocksClosure(kind) {
  return RECOVERY_KINDS.has(kind) && kind !== "api_body";
}

function canonicalRecoveryUrl(value) {
  const parsed = new URL(value);
  parsed.hash = "";
  parsed.searchParams.sort();
  return parsed.href;
}

function recoverySignature(kind, url) {
  return crypto
    .createHash("sha256")
    .update(`${kind}\0${canonicalRecoveryUrl(url)}`)
    .digest("hex");
}

function resumableCollectionCheckpoint(
  manifest,
  targetUrl,
  producerRunId,
  producerSessionId,
  producerOperationId,
  producerStageStartedAt,
) {
  if (
    !producerRunId ||
    !producerSessionId ||
    !producerOperationId ||
    !producerStageStartedAt
  ) {
    return null;
  }
  const activeStageStartedAt = Date.parse(producerStageStartedAt);
  const manifestStageStartedAt = Date.parse(manifest.producer_stage_started_at ?? "");
  const manifestCapturedAt = Date.parse(manifest.captured_at ?? "");
  if (
    !Number.isFinite(activeStageStartedAt) ||
    !Number.isFinite(manifestStageStartedAt) ||
    !Number.isFinite(manifestCapturedAt) ||
    manifest.producer_stage_started_at !== producerStageStartedAt ||
    manifestCapturedAt < activeStageStartedAt
  ) {
    return null;
  }
  if (
    manifest.producer_run_id !== producerRunId ||
    manifest.producer_session_id !== producerSessionId ||
    manifest.producer_operation_id !== producerOperationId ||
    manifest.closure_complete ||
    !["partial", "error"].includes(manifest.completion_state) ||
    manifest.stale > 0 ||
    manifest.size_limited > 0 ||
    manifest.entries.length !== manifest.raw_entries
  ) {
    return null;
  }
  const reasons = manifest.closure_incomplete_reasons;
  if (
    reasons.length === 0 ||
    !reasons.every((reason) => RECOVERABLE_CHECKPOINT_REASONS.has(reason))
  ) {
    return null;
  }
  const sanitizePages = (pages) => {
    const out = [];
    const seen = new Set();
    for (const value of pages) {
      let parsed;
      try {
        parsed = new URL(value);
      } catch {
        return null;
      }
      if (
        parsed.origin !== targetUrl.origin ||
        isDangerousNavigationUrl(parsed) ||
        seen.has(parsed.href)
      ) {
        if (seen.has(parsed.href)) continue;
        return null;
      }
      seen.add(parsed.href);
      out.push(parsed.href);
    }
    return out;
  };
  const visitedPages = sanitizePages(manifest.visited_pages);
  const pendingPages = sanitizePages(manifest.pending_pages);
  if (!visitedPages || !pendingPages) return null;
  const visited = new Set(visitedPages);
  if (pendingPages.some((url) => visited.has(url))) return null;
  const pendingRecursiveScripts = [];
  const recursiveKeys = new Set();
  const cachedScriptKeys = new Set(manifest.entries.map((entry) => entry.key));
  for (const item of manifest.pending_recursive_scripts) {
    const rawUrl = typeof item === "string" ? item : item.url;
    if (typeof rawUrl !== "string") return null;
    let parsed;
    try {
      parsed = new URL(rawUrl);
    } catch {
      return null;
    }
    if (parsed.origin !== targetUrl.origin || isDangerousNavigationUrl(parsed)) {
      return null;
    }
    const key = canonicalScriptUrl(parsed.href);
    if (!key || cachedScriptKeys.has(key) || recursiveKeys.has(key)) continue;
    let source = typeof item === "object" ? item.source : null;
    if (typeof source !== "string" || !isExactOriginUrl(source, targetUrl.origin)) {
      source = targetUrl.href;
    }
    recursiveKeys.add(key);
    pendingRecursiveScripts.push({ url: parsed.href, key, source });
  }
  const apiRequests = [];
  const apiKeys = new Set();
  for (const request of manifest.api_requests) {
    if (
      typeof request.url !== "string" ||
      !isExactOriginUrl(request.url, targetUrl.origin) ||
      typeof request.method !== "string" ||
      !request.method.trim()
    ) {
      return null;
    }
    const key = `${request.method} ${request.url}`;
    if (apiKeys.has(key)) continue;
    apiKeys.add(key);
    apiRequests.push(request);
  }
  const recoveryFailures = [];
  const recoverySignatures = new Set();
  for (const failure of manifest.recovery_failures) {
    const kind = typeof failure.kind === "string" ? failure.kind : "";
    const url = typeof failure.url === "string" ? failure.url : "";
    const count = Number.isInteger(failure.count) ? failure.count : 0;
    if (
      !RECOVERY_KINDS.has(kind) ||
      !isExactOriginUrl(url, targetUrl.origin) ||
      isDangerousNavigationUrl(url) ||
      count < 1 ||
      count > MAX_RECOVERY_FAILURES
    ) {
      return null;
    }
    const signature = recoverySignature(kind, url);
    if (
      (typeof failure.signature === "string" && failure.signature !== signature) ||
      recoverySignatures.has(signature)
    ) {
      return null;
    }
    if (!recoveryKindBlocksClosure(kind)) {
      // Drop legacy API-body failures from the resumable closure state. The
      // per-request capture_error remains in api_requests for diagnostics.
      continue;
    }
    recoverySignatures.add(signature);
    recoveryFailures.push({
      signature,
      kind,
      url: canonicalRecoveryUrl(url),
      count,
      reason:
        typeof failure.reason === "string" ? failure.reason.slice(0, 300) : "retryable failure",
    });
  }
  const recoveryExhausted = recoveryFailures.some(
    (failure) => failure.count >= MAX_RECOVERY_FAILURES,
  );
  if (manifest.recovery_exhausted && !recoveryExhausted) return null;
  if (
    pendingPages.length === 0 &&
    pendingRecursiveScripts.length === 0 &&
    recoveryFailures.length === 0
  ) {
    return null;
  }
  return {
    visited_pages: visitedPages,
    pending_pages: pendingPages,
    pending_recursive_scripts: pendingRecursiveScripts,
    api_requests: apiRequests,
    page_resume_count: manifest.page_resume_count + 1,
    checkpoint_resume_count: manifest.checkpoint_resume_count + 1,
    recovery_failures: recoveryFailures,
    recovery_exhausted: recoveryExhausted,
    automatic_retry_allowed: !recoveryExhausted,
  };
}

async function writeScriptManifest(
  workspace,
  targetHost,
  targetPort,
  targetScheme,
  scripts,
  completion,
  provenance,
  checkpoint,
) {
  const manifestPath = scriptManifestPath(workspace, targetHost, targetPort, targetScheme);
  await fs.mkdir(path.dirname(manifestPath), { recursive: true });
  const rows = scripts
    .filter((script) => script.path && script.url)
    .map((script) => ({
      url: script.url,
      canonical_url: canonicalScriptUrl(script.url),
      path: script.path,
      size: script.size ?? null,
      status: script.status ?? null,
      content_type: script.content_type ?? "",
      sha256: script.sha256 ?? null,
      discovered_by: script.discovered_by ?? null,
      duplicate_of: script.duplicate_of ?? null,
    }))
    .sort((a, b) => a.canonical_url.localeCompare(b.canonical_url));
  const capturedAt = new Date().toISOString();
  const payload = {
    version: 2,
    updated_at: capturedAt,
    captured_at: capturedAt,
    producer_run_id: provenance.run_id || null,
    producer_session_id: provenance.session_id || null,
    producer_operation_id: provenance.operation_id || null,
    producer_stage_started_at: provenance.stage_started_at || null,
    visited_pages: checkpoint.visited_pages,
    pending_pages: checkpoint.pending_pages,
    pending_recursive_scripts: checkpoint.pending_recursive_scripts,
    api_requests: checkpoint.api_requests,
    page_resume_count: checkpoint.page_resume_count,
    checkpoint_resume_count: checkpoint.checkpoint_resume_count,
    recovery_failures: checkpoint.recovery_failures,
    recovery_exhausted: checkpoint.recovery_exhausted,
    automatic_retry_allowed: checkpoint.automatic_retry_allowed,
    recovery_instruction: checkpoint.recovery_exhausted
      ? "Start a new trusted producer operation or stage attempt after changing the failing transport/timeout condition; this provenance will not auto-retry again."
      : null,
    collection_status: completion.status,
    completion_state: completion.completion_state,
    closure_complete: completion.closure_complete,
    closure_incomplete_reasons: completion.reasons,
    scripts: rows,
  };
  const tmpPath = `${manifestPath}.tmp`;
  await fs.writeFile(tmpPath, `${JSON.stringify(payload, null, 2)}\n`);
  await fs.rename(tmpPath, manifestPath);
  return manifestPath;
}

function sameOrigin(urlString, origin) {
  try {
    return new URL(urlString).origin === origin;
  } catch {
    return false;
  }
}

function summarizeRecursiveErrors(errors, sampleLimit = 20) {
  const byStatus = new Map();
  for (const error of errors) {
    const status = error?.status == null ? "network" : String(error.status);
    byStatus.set(status, (byStatus.get(status) ?? 0) + 1);
  }
  return {
    sample: errors.slice(0, sampleLimit),
    by_status: [...byStatus.entries()]
      .map(([status, count]) => ({ status, count }))
      .sort((a, b) => b.count - a.count || a.status.localeCompare(b.status)),
  };
}

function resolveSameOriginUrl(value, targetUrl) {
  try {
    const resolved = new URL(value, targetUrl.href);
    if (resolved.origin !== targetUrl.origin) return null;
    return resolved.href;
  } catch {
    return null;
  }
}

function resolveSameOriginPath(value, targetUrl) {
  try {
    const resolved = new URL(value, targetUrl.href);
    if (resolved.origin !== targetUrl.origin) return null;
    return `${resolved.pathname}${resolved.search}`;
  } catch {
    return null;
  }
}

function resolveAllowedScriptUrl(value, targetUrl, allowedOrigins) {
  try {
    const resolved = new URL(value, targetUrl.href);
    if (!allowedOrigins.has(resolved.origin)) return null;
    return resolved.href;
  } catch {
    return null;
  }
}

function shouldSkipClick(text) {
  return /(logout|log out|delete|remove|checkout|purchase|shutdown|restart|stop|cancel|disable|deactivate|archive|unsubscribe|approve|reject|activate|enable|refund|buy|pay|submit|sign out|退出|删除|支付|购买|提交)/i.test(
    text,
  );
}

const DANGEROUS_NAVIGATION_PATTERN =
  /(?:logout|log[-_ ]?out|sign[-_ ]?out|delete|remove|destroy|payment|checkout|purchase|confirm|reset|clear|terminate|revoke|shutdown|restart|stop|cancel|disable|deactivate|archive|unsubscribe|approve|reject|activate|enable|refund)/i;

function decodePercentOnce(value) {
  return value.replace(/%([0-9a-f]{2})/gi, (_match, hex) =>
    String.fromCharCode(Number.parseInt(hex, 16)),
  );
}

export function isDangerousNavigationUrl(value) {
  try {
    const parsed = value instanceof URL ? value : new URL(value);
    let route = `${parsed.pathname}${parsed.search}`;
    for (let pass = 0; pass < 2; pass += 1) {
      const decoded = decodePercentOnce(route);
      if (decoded === route) break;
      route = decoded;
    }
    // Two decoding passes are the supported interpretation boundary. A valid
    // escape that remains is deeper/mixed encoding, so fail closed instead of
    // guessing how many downstream layers may decode it.
    if (/%[0-9a-f]{2}/i.test(route)) return true;
    return DANGEROUS_NAVIGATION_PATTERN.test(route);
  } catch {
    return true;
  }
}

async function collectSameOriginLinks(page, origin) {
  const result = await page.evaluate(
    ({ origin }) =>
      (() => {
        const links = Array.from(document.querySelectorAll("a[href]"))
        .map((a) => a.href)
        .filter((href) => {
          try {
            const url = new URL(href);
            return (
              url.origin === origin &&
              !url.hash &&
              !url.href.startsWith("javascript:") &&
              !url.href.startsWith("mailto:")
            );
          } catch {
            return false;
          }
        });
        const unique = [...new Set(links)];
        return {
          // `max_pages` is an execution slice, not a discovery limit. Keep
          // every safe exact-origin candidate in the durable pending queue so
          // a wide page can make deterministic progress across invocations.
          links: unique,
          dropped: 0,
        };
      })(),
    { origin },
  );
  return result;
}

async function collectDomScriptCandidates(page, origin, limit) {
  const urls = await page.evaluate(
    ({ origin, limit }) => {
      const candidates = [];
      const push = (value) => {
        if (!value) return;
        try {
          const url = new URL(value, location.href);
          if (url.origin === origin && /\.(?:js|mjs|cjs)(?:$|\?)/i.test(url.pathname + url.search)) {
            candidates.push(url.href);
          }
        } catch {
          // Ignore invalid DOM URLs.
        }
      };
      for (const script of document.querySelectorAll("script[src]")) {
        push(script.getAttribute("src"));
      }
      for (const link of document.querySelectorAll("link[href]")) {
        const rel = (link.getAttribute("rel") || "").toLowerCase();
        const as = (link.getAttribute("as") || "").toLowerCase();
        if (
          rel.includes("modulepreload") ||
          rel.includes("preload") ||
          rel.includes("prefetch") ||
          as === "script"
        ) {
          push(link.getAttribute("href"));
        }
      }
      return [...new Set(candidates)].slice(0, limit);
    },
    { origin, limit },
  );
  return urls;
}

function inferFrameworkManifestUrls(scriptUrls, targetUrl) {
  const urls = [];
  const seen = new Set();
  const add = (value) => {
    try {
      const url = new URL(value, targetUrl.href);
      if (url.origin !== targetUrl.origin || seen.has(url.href)) return;
      seen.add(url.href);
      urls.push(url.href);
    } catch {
      // Ignore invalid inferred manifests.
    }
  };

  for (const scriptUrl of scriptUrls) {
    let parsed;
    try {
      parsed = new URL(scriptUrl);
    } catch {
      continue;
    }
    if (parsed.origin !== targetUrl.origin) continue;
    const nextMatch = parsed.pathname.match(/\/_next\/static\/([^/]+)\//);
    if (nextMatch) {
      add(`/_next/static/${nextMatch[1]}/_buildManifest.js`);
      add(`/_next/static/${nextMatch[1]}/_ssgManifest.js`);
      add(`/_next/static/${nextMatch[1]}/app-build-manifest.json`);
      add(`/_next/static/${nextMatch[1]}/build-manifest.json`);
      add(`/_next/static/${nextMatch[1]}/react-loadable-manifest.json`);
    }
    if (parsed.pathname.startsWith("/_nuxt/")) {
      add("/_nuxt/builds/latest.json");
      add("/_nuxt/manifest.json");
    }
    if (parsed.pathname.startsWith("/static/js/")) {
      add("/asset-manifest.json");
    }
  }

  return urls;
}

async function waitWithinDeadline(ms, deadlineMs) {
  const waitMs = Math.min(ms, timeLeft(deadlineMs));
  if (waitMs > 0) {
    await sleep(waitMs);
  }
}

async function exercisePage(page, maxActions, deadlineMs) {
  for (let i = 0; i < 4; i += 1) {
    if (timeLeft(deadlineMs) <= 0) break;
    await page.mouse.wheel(0, 900).catch(() => {});
    await waitWithinDeadline(250, deadlineMs);
  }

  if (timeLeft(deadlineMs) <= 0) return { clicked: 0 };

  const candidates = await page
    .locator(
      "button:not([type=submit]), [role=button], summary, [aria-expanded=false]",
    )
    .evaluateAll((nodes, maxActions) =>
      nodes
        .map((node, index) => ({
          index,
          text: (node.innerText || node.textContent || "").trim().slice(0, 80),
        }))
        .slice(0, maxActions),
      maxActions,
    )
    .catch(() => []);

  let clicked = 0;
  for (const item of candidates) {
    if (clicked >= maxActions || timeLeft(deadlineMs) <= 0) break;
    if (shouldSkipClick(item.text)) continue;
    const locator = page
      .locator(
        "button:not([type=submit]), [role=button], summary, [aria-expanded=false]",
      )
      .nth(item.index);
    try {
      const visibleTimeout = Math.max(1, Math.min(500, timeLeft(deadlineMs)));
      if (await locator.isVisible({ timeout: visibleTimeout })) {
        const clickTimeout = Math.max(1, Math.min(1_000, timeLeft(deadlineMs)));
        await locator.click({ timeout: clickTimeout });
        clicked += 1;
        await waitWithinDeadline(500, deadlineMs);
      }
    } catch {
      // Dynamic pages often detach nodes while we interact. Best-effort only.
    }
  }

  return { clicked };
}

async function applyRecipeClicks(page, clickTexts, maxClicks, deadlineMs) {
  let clicked = 0;
  for (const text of clickTexts) {
    if (clicked >= maxClicks || timeLeft(deadlineMs) <= 0) break;
    if (shouldSkipClick(text)) continue;
    try {
      const locator = page.getByText(text, { exact: false }).first();
      const visibleTimeout = Math.max(1, Math.min(750, timeLeft(deadlineMs)));
      if (await locator.isVisible({ timeout: visibleTimeout })) {
        const clickTimeout = Math.max(1, Math.min(1_500, timeLeft(deadlineMs)));
        await locator.click({ timeout: clickTimeout });
        clicked += 1;
        await waitWithinDeadline(500, deadlineMs);
      }
    } catch {
      // Text hints are best-effort only.
    }
  }
  return clicked;
}

async function launchBrowser() {
  try {
    return await chromium.launch({ headless: true });
  } catch (error) {
    const message = String(error?.message ?? error);
    if (!/Executable doesn't exist|browserType\.launch/i.test(message)) {
      throw error;
    }
    return chromium.launch({ channel: "chrome", headless: true });
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args.url || !args.workspace) {
    throw new Error("--url and --workspace are required");
  }

  const targetUrl = new URL(args.url);
  const workspace = path.resolve(String(args.workspace));
  const targetHost = targetUrl.hostname || "unknown";
  const targetPort =
    targetUrl.port || (targetUrl.protocol === "http:" ? "80" : "443");
  const targetScheme = targetUrl.protocol.replace(/:$/, "");
  const crawlMode = parseCrawlMode(args.crawl_mode);
  const timeoutMs = toInt(
    args.timeout_ms,
    DEFAULT_TIMEOUT_MS,
    5_000,
    300_000,
  );
  // A finite per-origin wall clock is mandatory. Page count alone cannot bound
  // a response body, recursive fetch, or stuck browser-protocol operation.
  // Explicit zero is legacy input and is normalized to the same safe default.
  const hardTimeoutMs = boundedHardTimeoutMs(args.hard_timeout_ms);
  const hardDeadline = Date.now() + hardTimeoutMs;
  const maxPages = toInt(
    args.max_pages,
    DEFAULT_MAX_PAGES,
    1,
    100,
  );
  const requestedMaxActions = toInt(
    args.max_actions,
    DEFAULT_MAX_ACTIONS,
    0,
    100,
  );
  // Enumeration has no authorization contract for state-changing UI
  // interactions. Keep scrolling/lazy loading, but clicks remain disabled even
  // if an old caller or AI recipe asks for them.
  const maxActions = 0;
  const maxScriptBytes = toLimit(
    args.max_script_bytes,
    DEFAULT_MAX_SCRIPT_BYTES,
    100_000,
    Number.MAX_SAFE_INTEGER,
  );
  const maxRecursiveScripts = toLimit(
    args.max_recursive_scripts,
    DEFAULT_MAX_RECURSIVE_SCRIPTS,
    1,
    Number.MAX_SAFE_INTEGER,
  );
  const requestedSameOrigin = toBool(args.same_origin, true);
  const restrictApisToSameOrigin = true;
  const includeAiAssist = toBool(args.ai_assist, true);
  const blockNoise = toBool(args.block_noise, true);
  const producerRunId =
    typeof args.run_id === "string" && args.run_id.trim() ? args.run_id.trim() : null;
  const producerSessionId =
    typeof args.session_id === "string" && args.session_id.trim()
      ? args.session_id.trim()
      : null;
  const producerOperationId =
    typeof args.operation_id === "string" && args.operation_id.trim()
      ? args.operation_id.trim()
      : null;
  const producerStageStartedAt =
    typeof args.stage_started_at === "string" && args.stage_started_at.trim()
      ? args.stage_started_at.trim()
      : null;
  const recipe = parseRecipe(args.recipe_json);
  const blockedReadOnlyRouteUrls = new Set();
  if (isDangerousNavigationUrl(targetUrl)) {
    throw new Error(`read-only Enumeration navigation blocked: ${targetUrl.href}`);
  }
  const recipeRoutes = [];
  for (const route of safeStringArray(recipe.routes, 20)) {
    const resolved = resolveSameOriginUrl(route, targetUrl);
    if (!resolved) continue;
    if (isDangerousNavigationUrl(resolved)) {
      blockedReadOnlyRouteUrls.add(resolved);
      continue;
    }
    recipeRoutes.push(resolved);
  }
  const recipeManifestPaths = [];
  let disabledRecipeManifestPaths = 0;
  for (const manifestPath of safeStringArray(recipe.manifest_paths, 20)) {
    const resolved = resolveSameOriginPath(manifestPath, targetUrl);
    if (!resolved) continue;
    const absolute = new URL(resolved, targetUrl.href);
    if (isDangerousNavigationUrl(absolute)) {
      blockedReadOnlyRouteUrls.add(absolute.href);
      disabledRecipeManifestPaths += 1;
      continue;
    }
    recipeManifestPaths.push(resolved);
  }
  const recipeScriptUrlHints = safeStringArray(recipe.script_urls, maxRecursiveScripts);
  let disabledRecipeScriptUrls = 0;
  const requestedRecipeClickTexts = safeStringArray(recipe.click_texts, 20, 120);
  const recipeClickTexts = [];
  const recipePublicPath =
    typeof recipe.public_path === "string" ? recipe.public_path.trim().slice(0, 300) : null;
  const recipeChunkPairs = safeChunkPairs(recipe.chunk_pairs, maxRecursiveScripts);
  const recipeSummary = {
    crawl_mode: crawlMode,
    routes: recipeRoutes.length,
    manifest_paths: recipeManifestPaths.length,
    manifest_paths_disabled: disabledRecipeManifestPaths,
    script_urls: recipeScriptUrlHints.length,
    script_urls_disabled: disabledRecipeScriptUrls,
    click_texts: recipeClickTexts.length,
    click_texts_disabled: requestedRecipeClickTexts.length,
    public_path: recipePublicPath || null,
    chunk_pairs: recipeChunkPairs.length,
  };

  progress("start", {
    target: targetUrl.href,
    timeout_ms: timeoutMs,
    hard_timeout_ms: limitLabel(hardTimeoutMs),
    max_pages: maxPages,
    max_actions: maxActions,
    requested_max_actions: requestedMaxActions,
    max_script_bytes: limitLabel(maxScriptBytes),
    max_recursive_scripts: limitLabel(maxRecursiveScripts),
  });
  progress("launch_browser");
  const browser = await withTimeout(launchBrowser(), timeLeft(hardDeadline, 15_000));
  if (browser === TIMEOUT) {
    throw new Error("browser launch timed out");
  }
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    serviceWorkers: "block",
    userAgent:
      "Mozilla/5.0 (compatible; GolishBrowserCollect/1.0; +https://golish.local)",
  });
  let blockedResourceRequests = 0;
  const blockedNavigationUrls = new Set();
  const blockedSubresourceUrls = new Set();
  const blockedWebSocketUrls = new Set();
  const blockedReadOnlyRequests = new Set();
  const terminalCrossOriginRedirects = new Map();
  const apiRequestsByKey = new Map();
  let apiTraceCount = 0;

  const observeApiRequest = (request, blockedReadOnly = false, force = false) => {
    const type = request.resourceType();
    if (!force && type !== "xhr" && type !== "fetch") return null;
    const url = request.url();
    if (!isExactOriginUrl(url, targetUrl.origin)) return null;
    const key = `${request.method()} ${url}`;
    if (!apiRequestsByKey.has(key)) {
      apiRequestsByKey.set(key, {
        url,
        method: request.method(),
        resource_type: type,
        status: null,
        request_headers: request.headers(),
        request_body: request.postData() ?? null,
        read_only_blocked: blockedReadOnly,
      });
      if (apiTraceCount < 80) {
        progress(blockedReadOnly ? "api_observed_blocked_read_only" : "api_observed", {
          method: request.method(),
          url,
        });
        apiTraceCount += 1;
      }
    } else if (blockedReadOnly) {
      apiRequestsByKey.get(key).read_only_blocked = true;
    }
    return apiRequestsByKey.get(key);
  };

  await context.route("**/*", async (route) => {
    const request = route.request();
    const type = request.resourceType();
    const url = request.url();
    let parsedRequestUrl = null;
    try {
      parsedRequestUrl = new URL(url);
    } catch {
      // Browser-internal schemes are not network requests; leave them to the
      // existing resource/noise policy below.
    }
    if (
      parsedRequestUrl &&
      (parsedRequestUrl.protocol === "http:" || parsedRequestUrl.protocol === "https:") &&
      parsedRequestUrl.origin !== targetUrl.origin
    ) {
      if (request.isNavigationRequest()) blockedNavigationUrls.add(url);
      else blockedSubresourceUrls.add(url);
      await route.abort("blockedbyclient").catch(() => {});
      return;
    }
    const unsafeMethod = !matchesSafeNavigationMethod(request.method());
    const dangerousRoute = parsedRequestUrl && isDangerousNavigationUrl(parsedRequestUrl);
    const observedApi = observeApiRequest(
      request,
      unsafeMethod || dangerousRoute,
      unsafeMethod,
    );
    if (unsafeMethod || dangerousRoute) {
      const key = `${request.method()} ${url}`;
      blockedReadOnlyRequests.add(key);
      if (dangerousRoute) blockedReadOnlyRouteUrls.add(url);
      if (observedApi) observedApi.read_only_block_reason = unsafeMethod
        ? "method_not_read_only"
        : "dangerous_route";
      await route.abort("blockedbyclient").catch(() => {});
      return;
    }
    if (request.isNavigationRequest()) {
      if (redirectDepth(request) > 5) {
        blockedNavigationUrls.add(url);
        await route.abort("blockedbyclient").catch(() => {});
        return;
      }
      try {
        const inspection = await fulfillSafeNavigationOnce(
          route,
          targetUrl.origin,
          timeLeft(hardDeadline, Math.min(timeoutMs, 10_000)),
        );
        if (!inspection.allowed) {
          blockedNavigationUrls.add(inspection.blocked_url);
          await route.abort("blockedbyclient").catch(() => {});
          return;
        }
        if (inspection.terminal_redirect) {
          const redirect = inspection.terminal_redirect;
          terminalCrossOriginRedirects.set(
            `${redirect.status} ${redirect.from} -> ${redirect.to}`,
            redirect,
          );
        }
      } catch {
        await route.abort("failed").catch(() => {});
        return;
      }
      return;
    }
    if (isBlockedResourceType(type) || (blockNoise && isNoiseUrl(url))) {
      blockedResourceRequests += 1;
      await route.abort().catch(() => {});
      return;
    }
    await route.continue().catch(() => {});
  });
  await context.routeWebSocket(/.*/, async (webSocketRoute) => {
    const url = webSocketRoute.url();
    blockedWebSocketUrls.add(url);
    const reason = isExactOriginWebSocketUrl(url, targetUrl)
      ? "Golish read-only Enumeration"
      : "Golish exact-origin scope";
    // Even a same-origin WebSocket can send state-changing messages. There is
    // no read-only handshake contract here, so Enumeration never connects it.
    await webSocketRoute.close({ code: 1008, reason }).catch(() => {});
  });
  const page = await context.newPage();
  progress("browser_ready", {
    block_noise: blockNoise,
    same_origin: restrictApisToSameOrigin,
    requested_same_origin: requestedSameOrigin,
  });

  const scriptsByUrl = new Map();
  const scriptInsightsByUrl = new Map();
  const aiReviewRefsByKey = new Map();
  const recursiveQueue = [];
  const queuedRecursiveUrls = new Set();
  const scriptPathByHash = new Map();
  const scannedScriptHashes = new Set();
  const pending = new Set();
  const navigationErrors = [];
  const consoleErrors = [];
  const recursiveErrors = [];
  let publicPathHint = null;
  let recursiveScriptsDownloaded = 0;
  let scriptByteLimitSkips = 0;
  let scriptCaptureErrors = 0;
  let hardDeadlineHit = false;
  let pendingBodyTimeouts = 0;
  let apiBodyCaptureErrors = 0;
  let pendingWaitTimedOut = false;
  let contextCloseTimedOut = false;
  let browserCloseTimedOut = false;
  let duplicateContentHits = 0;
  let scriptTraceCount = 0;
  let recursiveTraceCount = 0;

  const recordRecursiveError = (url, status, reason) => {
    if (recursiveErrors.length >= 100) return;
    recursiveErrors.push({
      url,
      status,
      reason,
    });
  };

  const recordAiReviewRef = (ref, source) => {
    const key = `${source} ${ref}`;
    if (aiReviewRefsByKey.has(key) || aiReviewRefsByKey.size >= 500) return;
    aiReviewRefsByKey.set(key, {
      ref,
      source_url: source,
      resolved_candidate: resolveScriptReference(ref, source),
      reason: "relative_js_module_specifier_not_auto_fetched",
    });
  };

  const enqueueScriptUrl = (url, source) => {
    if (!url) return;
    const key = canonicalScriptUrl(url);
    if (!key || scriptsByUrl.has(key) || queuedRecursiveUrls.has(key)) return;
    try {
      const parsed = new URL(url);
      if (parsed.origin !== targetUrl.origin) return;
      if (isDangerousNavigationUrl(parsed)) {
        blockedReadOnlyRouteUrls.add(parsed.href);
        return;
      }
      queuedRecursiveUrls.add(key);
      recursiveQueue.push({ url: parsed.href, key, source });
    } catch {
      // Ignore invalid chunk references.
    }
  };

  const enqueueRefsFromScript = (scriptUrl, body) => {
    const hash = sha256Hex(body);
    if (scannedScriptHashes.has(hash)) return;
    scannedScriptHashes.add(hash);
    const text = body.toString("utf8");
    const detectedPublicPath = extractPublicPath(text);
    publicPathHint ||= recipePublicPath || detectedPublicPath;
    const refCandidates = scanJsForReferenceCandidates(text);
    const refs = refCandidates.auto_refs;
    for (const ref of refCandidates.ai_review_refs) {
      recordAiReviewRef(ref, scriptUrl);
    }
    const runtimeChunkUrls = expandRuntimeChunkUrls(
      text,
      recipePublicPath || detectedPublicPath || publicPathHint,
      new URL("./", scriptUrl).href,
    );
    const viteChunkUrls = expandViteMapDeps(text, scriptUrl);
    const rawChunkUrls =
      runtimeChunkUrls.length === 0 && viteChunkUrls.length === 0
        ? expandWebpackChunkMap(
            text,
            recipePublicPath || detectedPublicPath || publicPathHint,
            new URL("./", scriptUrl).href,
          )
        : [];
    const chunkUrls = [
      ...new Set([...runtimeChunkUrls, ...viteChunkUrls, ...rawChunkUrls]),
    ];
    scriptInsightsByUrl.set(scriptUrl, {
      url: scriptUrl,
      size: body.length,
      public_path_detected: detectedPublicPath,
      refs_sample: refs.slice(0, 20),
      ai_review_refs_sample: refCandidates.ai_review_refs.slice(0, 20),
      chunk_urls_sample: chunkUrls.slice(0, 20),
      runtime_chunk_urls_sample: runtimeChunkUrls.slice(0, 20),
      vite_chunk_urls_sample: viteChunkUrls.slice(0, 20),
      snippets: extractInterestingSnippets(text),
    });
    for (const ref of refs) {
      enqueueScriptUrl(resolveScriptReference(ref, scriptUrl), scriptUrl);
    }
    for (const chunkUrl of chunkUrls) {
      enqueueScriptUrl(resolveScriptReference(chunkUrl, scriptUrl), scriptUrl);
    }
  };

  const manifestCache = await loadScriptManifest(
    workspace,
    targetHost,
    targetPort,
    targetScheme,
    maxScriptBytes,
  );
  const checkpointResume = resumableCollectionCheckpoint(
    manifestCache,
    targetUrl,
    producerRunId,
    producerSessionId,
    producerOperationId,
    producerStageStartedAt,
  );
  let cachedScriptsPreloaded = 0;
  if (checkpointResume) {
    // Only the same run/session/operation/stage attempt may carry verified
    // observations and pending cursors forward. The manifest is revalidated
    // for exact-origin scope, safe routes, and intact script files first.
    for (const entry of manifestCache.entries) {
      if (!entry.key || scriptsByUrl.has(entry.key)) continue;
      scriptsByUrl.set(entry.key, {
        url: entry.url,
        path: entry.path,
        size: entry.size,
        status: entry.status,
        content_type: entry.content_type,
        sha256: entry.sha256,
        cached: true,
        resumed_from_checkpoint: true,
      });
      if (entry.sha256) scriptPathByHash.set(entry.sha256, entry.path);
      cachedScriptsPreloaded += 1;
    }
    for (const request of checkpointResume.api_requests) {
      apiRequestsByKey.set(`${request.method} ${request.url}`, request);
    }
    progress("resume_collection_checkpoint", {
      visited_pages: checkpointResume.visited_pages.length,
      pending_pages: checkpointResume.pending_pages.length,
      pending_recursive_scripts: checkpointResume.pending_recursive_scripts.length,
      scripts: cachedScriptsPreloaded,
      api_requests: checkpointResume.api_requests.length,
      resume_count: checkpointResume.checkpoint_resume_count,
      recovery_exhausted: checkpointResume.recovery_exhausted,
    });
  } else {
    // Cross-run/session/stage or structurally unsafe manifests are historical
    // evidence, never current observations. Do not seed discovery/dedupe or
    // completion counters from them.
    progress("ignore_previous_script_manifest", {
      previous_entries: manifestCache.entries.length,
      stale_entries: manifestCache.stale,
      size_limited_entries: manifestCache.size_limited,
    });
  }

  const recoveryFailures = new Map(
    (checkpointResume?.recovery_failures ?? []).map((failure) => [
      failure.signature,
      failure,
    ]),
  );
  const pendingRecoveryPages = new Set();
  const recoveryFailureList = () =>
    [...recoveryFailures.values()].sort(
      (left, right) => left.signature.localeCompare(right.signature),
    );
  const noteRecoveryFailure = (kind, rawUrl, reason) => {
    if (!recoveryKindBlocksClosure(kind)) {
      return { exhausted: false, count: 0, failure: null };
    }
    let url;
    try {
      url = canonicalRecoveryUrl(rawUrl);
    } catch {
      return { exhausted: true, count: MAX_RECOVERY_FAILURES };
    }
    if (
      !RECOVERY_KINDS.has(kind) ||
      !isExactOriginUrl(url, targetUrl.origin) ||
      isDangerousNavigationUrl(url)
    ) {
      return { exhausted: true, count: MAX_RECOVERY_FAILURES };
    }
    const signature = recoverySignature(kind, url);
    const count = Math.min(
      MAX_RECOVERY_FAILURES,
      (recoveryFailures.get(signature)?.count ?? 0) + 1,
    );
    const failure = {
      signature,
      kind,
      url,
      count,
      reason: String(reason || "retryable failure").slice(0, 300),
    };
    recoveryFailures.set(signature, failure);
    return { exhausted: count >= MAX_RECOVERY_FAILURES, count, failure };
  };
  const clearRecoveryFailure = (kind, rawUrl) => {
    try {
      recoveryFailures.delete(recoverySignature(kind, rawUrl));
    } catch {
      // Invalid current observations cannot match a validated checkpoint row.
    }
  };
  const retryPageForRequest = (request) => {
    try {
      const frameUrl = request.frame().url();
      if (
        isExactOriginUrl(frameUrl, targetUrl.origin) &&
        !isDangerousNavigationUrl(frameUrl)
      ) {
        return canonicalRecoveryUrl(frameUrl);
      }
    } catch {
      // Fall back to the authorized seed below.
    }
    return targetUrl.href;
  };
  const automaticRetryBlocked = () =>
    recoveryFailureList().some((failure) => failure.count >= MAX_RECOVERY_FAILURES);

  const allowedRecipeScriptOrigins = new Set([targetUrl.origin]);
  const recipeScriptUrls = [];
  for (const scriptUrl of recipeScriptUrlHints) {
    const resolved = resolveAllowedScriptUrl(
      scriptUrl,
      targetUrl,
      allowedRecipeScriptOrigins,
    );
    if (!resolved) continue;
    if (isDangerousNavigationUrl(resolved)) {
      blockedReadOnlyRouteUrls.add(resolved);
      disabledRecipeScriptUrls += 1;
      continue;
    }
    recipeScriptUrls.push(resolved);
  }
  recipeSummary.script_urls = recipeScriptUrls.length;
  recipeSummary.script_urls_disabled = disabledRecipeScriptUrls;

  if (!automaticRetryBlocked()) {
    for (const pendingScript of checkpointResume?.pending_recursive_scripts ?? []) {
      enqueueScriptUrl(pendingScript.url, pendingScript.source);
    }
    for (const scriptUrl of recipeScriptUrls) {
      enqueueScriptUrl(scriptUrl, targetUrl.href);
    }
  }
  if (
    !automaticRetryBlocked() &&
    recipePublicPath &&
    recipeChunkPairs.length > 0
  ) {
    const base = resolvePublicPath(recipePublicPath, targetUrl.href);
    for (const pair of recipeChunkPairs) {
      enqueueScriptUrl(`${base}${pair.id}.${pair.hash}.js`, targetUrl.href);
    }
  }

  async function fetchManifests() {
    const manifestUrls = [
      ...recipeManifestPaths.map((manifestPath) =>
        new URL(manifestPath, targetUrl.href).href,
      ),
      ...inferFrameworkManifestUrls(
        [...scriptsByUrl.keys(), ...queuedRecursiveUrls],
        targetUrl,
      ),
    ];
    const uniqueManifestUrls = [...new Set(manifestUrls)];
    let fetched = 0;
    let failed = 0;
    let inferred = Math.max(0, uniqueManifestUrls.length - recipeManifestPaths.length);
    for (const manifestUrl of uniqueManifestUrls) {
      const fetchTimeout = timeLeft(hardDeadline, DEFAULT_FETCH_TIMEOUT_MS);
      if (fetchTimeout <= 0) {
        hardDeadlineHit = true;
        break;
      }
      try {
        const response = await fetchExactOrigin(
          manifestUrl,
          targetUrl.origin,
          {
            headers: {
              "user-agent":
                "Mozilla/5.0 (compatible; GolishBrowserCollect/1.0; +https://golish.local)",
            },
          },
          fetchTimeout,
        );
        if (!response.ok) {
          failed += 1;
          recordRecursiveError(manifestUrl, response.status, `manifest HTTP ${response.status}`);
          continue;
        }
        const body = await withTimeout(
          response.text(),
          timeLeft(hardDeadline, DEFAULT_BODY_TIMEOUT_MS),
        );
        if (body === TIMEOUT) {
          failed += 1;
          pendingBodyTimeouts += 1;
          pendingRecoveryPages.add(targetUrl.href);
          noteRecoveryFailure("manifest_body", manifestUrl, "manifest body-timeout");
          recordRecursiveError(manifestUrl, response.status, "manifest body-timeout");
          continue;
        }
        clearRecoveryFailure("manifest_body", manifestUrl);
        clearRecoveryFailure("recursive_fetch", manifestUrl);
        fetched += 1;
        for (const ref of [
          ...scanJsForReferences(body, { allowRelativeModules: true }),
          ...extractJsonManifestReferences(body, manifestUrl),
        ]) {
          enqueueScriptUrl(resolveManifestReference(ref, manifestUrl), manifestUrl);
        }
        const manifestPublicPath = recipePublicPath || extractPublicPath(body) || publicPathHint;
        for (const chunkUrl of expandWebpackChunkMap(
          body,
          manifestPublicPath,
          new URL("./", manifestUrl).href,
        )) {
          enqueueScriptUrl(resolveManifestReference(chunkUrl, manifestUrl), manifestUrl);
        }
      } catch (error) {
        failed += 1;
        pendingRecoveryPages.add(targetUrl.href);
        noteRecoveryFailure(
          "recursive_fetch",
          manifestUrl,
          `manifest ${String(error?.message ?? error).slice(0, 300)}`,
        );
        recordRecursiveError(
          manifestUrl,
          null,
          `manifest ${String(error?.message ?? error).slice(0, 300)}`,
        );
      }
    }
    recipeSummary.manifests_fetched = fetched;
    recipeSummary.manifests_failed = failed;
    recipeSummary.manifests_inferred = inferred;
  }

  page.on("console", (msg) => {
    if (msg.type() === "error") {
      consoleErrors.push(msg.text().slice(0, 500));
    }
  });

  page.on("request", (request) => {
    observeApiRequest(request);
  });

  page.on("response", (response) => {
    const request = response.request();
    const url = response.url();
    const headers = response.headers();
    const type = request.resourceType();

    if ((type === "xhr" || type === "fetch") && apiRequestsByKey.has(`${request.method()} ${url}`)) {
      const entry = apiRequestsByKey.get(`${request.method()} ${url}`);
      entry.status = response.status();
      entry.headers = headers;
      entry.content_type = headers["content-type"] ?? "";
      // capture v2: persist the request side so the Inspector can show it.
      entry.request_headers = request.headers();
      entry.request_body = request.postData() ?? null;
      const contentLength = Number.parseInt(headers["content-length"] ?? "0", 10);
      const task = (async () => {
        try {
          if (Number.isFinite(contentLength) && contentLength > MAX_API_CAPTURE_BYTES) {
            entry.capture_path = await saveApiResponseCapture(
              workspace,
              targetHost,
              targetPort,
              targetScheme,
              entry,
              headers,
              null,
              {
                body_truncated: true,
                body_skipped_reason: `content-length>${MAX_API_CAPTURE_BYTES}`,
              },
            );
            return;
          }
          const body = await withTimeout(
            response.body(),
            timeLeft(hardDeadline, DEFAULT_BODY_TIMEOUT_MS),
          );
          if (body === TIMEOUT) {
            apiBodyCaptureErrors += 1;
            entry.capture_error = "body-timeout";
            return;
          }
          clearRecoveryFailure("api_body", url);
          entry.capture_path = await saveApiResponseCapture(
            workspace,
            targetHost,
            targetPort,
            targetScheme,
            entry,
            headers,
            body,
            { body_truncated: false },
          );
        } catch (error) {
          apiBodyCaptureErrors += 1;
          entry.capture_error = String(error?.message ?? error).slice(0, 300);
        }
      })();
      pending.add(task);
      task.finally(() => pending.delete(task));
    }

    const scriptKey = canonicalScriptUrl(url);
    if (
      !isExactOriginUrl(url, targetUrl.origin) ||
      !isJavaScriptResponse(url, headers) ||
      scriptsByUrl.has(scriptKey)
    ) {
      return;
    }
    const contentLength = Number.parseInt(headers["content-length"] ?? "0", 10);
    if (Number.isFinite(maxScriptBytes) && Number.isFinite(contentLength) && contentLength > maxScriptBytes) {
      scriptByteLimitSkips += 1;
      scriptsByUrl.set(scriptKey, {
        url,
        status: response.status(),
        skipped: true,
        reason: `content-length>${maxScriptBytes}`,
      });
      return;
    }

    const task = (async () => {
        const body = await withTimeout(
          response.body(),
          timeLeft(hardDeadline, DEFAULT_BODY_TIMEOUT_MS),
        );
        if (body === TIMEOUT) {
          pendingBodyTimeouts += 1;
          const retryPage = retryPageForRequest(request);
          pendingRecoveryPages.add(retryPage);
          noteRecoveryFailure("script_body", url, "script response body-timeout");
          scriptsByUrl.set(scriptKey, {
            url,
            status: response.status(),
            skipped: true,
            reason: "body-timeout",
          });
          return;
        }
        clearRecoveryFailure("script_body", url);
        if (Number.isFinite(maxScriptBytes) && body.length > maxScriptBytes) {
          scriptByteLimitSkips += 1;
          scriptsByUrl.set(scriptKey, {
            url,
            status: response.status(),
            skipped: true,
            reason: `body>${maxScriptBytes}`,
          });
          return;
        }
        const sha = sha256Hex(body);
        let pathOnDisk = scriptPathByHash.get(sha);
        let duplicateOf = null;
        if (pathOnDisk) {
          duplicateContentHits += 1;
          duplicateOf = pathOnDisk;
        } else {
          pathOnDisk = await saveScriptCapture(
            workspace,
            targetHost,
            targetPort,
            targetScheme,
            url,
            body,
          );
          scriptPathByHash.set(sha, pathOnDisk);
        }
        enqueueRefsFromScript(url, body);
        scriptsByUrl.set(scriptKey, {
          url,
          path: pathOnDisk,
          size: body.length,
          status: response.status(),
          content_type: headers["content-type"] ?? "",
          sha256: sha,
          duplicate_of: duplicateOf,
        });
        if (scriptTraceCount < 80) {
          progress(duplicateOf ? "script_seen_duplicate" : "script_saved", {
            status: response.status(),
            bytes: body.length,
            path: pathOnDisk,
            url,
          });
          scriptTraceCount += 1;
        }
      })()
      .catch((error) => {
        scriptCaptureErrors += 1;
        const retryPage = retryPageForRequest(request);
        pendingRecoveryPages.add(retryPage);
        noteRecoveryFailure(
          "script_body",
          url,
          String(error?.message ?? error).slice(0, 500),
        );
        scriptsByUrl.set(scriptKey, {
          url,
          status: response.status(),
          skipped: true,
          reason: String(error?.message ?? error).slice(0, 500),
        });
      })
      .finally(() => pending.delete(task));
    pending.add(task);
  });

  const recoveryWasExhausted = checkpointResume?.recovery_exhausted === true;
  const blockedCheckpointPages = recoveryWasExhausted
    ? checkpointResume.pending_pages
    : [];
  const initialQueue = checkpointResume
    ? recoveryWasExhausted
      ? []
      : checkpointResume.pending_pages
    : [targetUrl.href, ...recipeRoutes];
  const queue = [...new Set(initialQueue)];
  const queuedPages = new Set(queue);
  const seenPages = new Set(checkpointResume?.visited_pages ?? []);
  const pagesVisitedThisRun = [];
  let actionsClicked = 0;
  let successfulPages = 0;
  let pageCandidatesDropped = 0;

  while (
    queue.length > 0 &&
    pagesVisitedThisRun.length < maxPages &&
    timeLeft(hardDeadline) > 0
  ) {
    const nextUrl = queue.shift();
    queuedPages.delete(nextUrl);
    if (!nextUrl || seenPages.has(nextUrl)) continue;
    pagesVisitedThisRun.push(nextUrl);

    try {
      const navTimeout = timeLeft(hardDeadline, Math.min(timeoutMs, 10_000));
      if (navTimeout <= 0) {
        hardDeadlineHit = true;
        break;
      }
      progress("goto", {
        page: pagesVisitedThisRun.length,
        cumulative_page: seenPages.size + 1,
        queued: queue.length,
        url: nextUrl,
        timeout_ms: navTimeout,
      });
      await page.goto(nextUrl, {
        waitUntil: "commit",
        timeout: Math.max(1, navTimeout),
      });
      if (!isExactOriginUrl(page.url(), targetUrl.origin)) {
        blockedNavigationUrls.add(page.url());
        throw new Error(`exact-origin final URL blocked: ${page.url()}`);
      }
      clearRecoveryFailure("navigation", nextUrl);
      seenPages.add(nextUrl);
      successfulPages += 1;
      const domTimeout = timeLeft(hardDeadline, 5_000);
      if (domTimeout > 0) {
        await page
          .waitForLoadState("domcontentloaded", { timeout: Math.max(1, domTimeout) })
          .catch(() => {});
      }
    } catch (error) {
      const message = String(error?.message ?? error).slice(0, 500);
      navigationErrors.push({
        url: nextUrl,
        error: message,
      });
      if (!message.includes("exact-origin")) {
        noteRecoveryFailure("navigation", nextUrl, message);
        pendingRecoveryPages.add(nextUrl);
      }
      continue;
    }

    try {
      const firstIdleTimeout = timeLeft(hardDeadline, 3_000);
      if (firstIdleTimeout > 0) {
        await page
          .waitForLoadState("networkidle", { timeout: Math.max(1, firstIdleTimeout) })
          .catch(() => {});
      }
      await waitWithinDeadline(750, hardDeadline);
      const exercised = await exercisePage(page, maxActions, hardDeadline);
      actionsClicked += exercised.clicked;
      actionsClicked += await applyRecipeClicks(
        page,
        recipeClickTexts,
        Math.max(0, maxActions - exercised.clicked),
        hardDeadline,
      );
      const secondIdleTimeout = timeLeft(hardDeadline, 3_000);
      if (secondIdleTimeout > 0) {
        await page
          .waitForLoadState("networkidle", { timeout: Math.max(1, secondIdleTimeout) })
          .catch(() => {});
      }
      await waitWithinDeadline(750, hardDeadline);
      progress("page_exercised", {
        url: page.url(),
        clicked: exercised.clicked,
        total_actions: actionsClicked,
        scripts_seen: scriptsByUrl.size,
        api_seen: apiRequestsByKey.size,
        queued_recursive: recursiveQueue.length,
      });

      const domScriptUrls = await withTimeout(
        collectDomScriptCandidates(
          page,
          targetUrl.origin,
          Number.isFinite(maxRecursiveScripts)
            ? Math.max(maxRecursiveScripts, maxPages * 10, 50)
            : Number.MAX_SAFE_INTEGER,
        ),
        timeLeft(hardDeadline, 3_000),
      );
      if (domScriptUrls !== TIMEOUT) {
        for (const scriptUrl of domScriptUrls) {
          enqueueScriptUrl(scriptUrl, page.url());
        }
      }

      const linkResult = await withTimeout(
        collectSameOriginLinks(page, targetUrl.origin),
        timeLeft(hardDeadline, 3_000),
      );
      if (linkResult === TIMEOUT) {
        const retryPage = canonicalRecoveryUrl(page.url());
        navigationErrors.push({
          url: retryPage,
          error: "collect-links-timeout",
        });
        noteRecoveryFailure(
          "page_inspection",
          retryPage,
          "collect-links-timeout",
        );
        pendingRecoveryPages.add(retryPage);
        continue;
      }
      clearRecoveryFailure("page_inspection", page.url());
      pageCandidatesDropped += linkResult.dropped;
      for (const link of linkResult.links) {
        if (seenPages.has(link) || queuedPages.has(link)) continue;
        if (isDangerousNavigationUrl(link)) {
          blockedReadOnlyRouteUrls.add(link);
          continue;
        }
        queue.push(link);
        queuedPages.add(link);
      }
    } catch (error) {
      const retryPage = isExactOriginUrl(page.url(), targetUrl.origin)
        ? canonicalRecoveryUrl(page.url())
        : nextUrl;
      const message = String(error?.message ?? error).slice(0, 500);
      navigationErrors.push({
        url: retryPage,
        error: message,
      });
      noteRecoveryFailure("page_inspection", retryPage, message);
      pendingRecoveryPages.add(retryPage);
    }
  }

  if (timeLeft(hardDeadline) <= 0) {
    hardDeadlineHit = true;
  }

  const pendingResult = await withTimeout(
    Promise.allSettled([...pending]),
    timeLeft(hardDeadline, DEFAULT_BODY_TIMEOUT_MS),
  );
  if (pendingResult === TIMEOUT) {
    pendingWaitTimedOut = true;
    pendingRecoveryPages.add(targetUrl.href);
    noteRecoveryFailure(
      "pending_wait",
      targetUrl.href,
      "pending response wait timed out",
    );
    progress("pending_response_wait_timeout", { pending: pending.size });
  } else {
    clearRecoveryFailure("pending_wait", targetUrl.href);
  }

  if (!automaticRetryBlocked() && timeLeft(hardDeadline) > 0) {
    progress("fetch_manifests", { queued_recursive: recursiveQueue.length });
    await fetchManifests();
  } else if (timeLeft(hardDeadline) <= 0) {
    hardDeadlineHit = true;
  }

  const recursiveDeadline = hardDeadline;
  const recursiveRetryQueue = [];
  const blockedCheckpointRecursive = recoveryWasExhausted
    ? checkpointResume.pending_recursive_scripts
    : [];
  while (
    !automaticRetryBlocked() &&
    recursiveQueue.length > 0 &&
    recursiveScriptsDownloaded < maxRecursiveScripts &&
    Date.now() < recursiveDeadline &&
    timeLeft(hardDeadline) > 0
  ) {
    const queued = recursiveQueue.shift();
    if (!queued || scriptsByUrl.has(queued.key)) continue;

    try {
      const fetchTimeout = timeLeft(hardDeadline, DEFAULT_FETCH_TIMEOUT_MS);
      if (fetchTimeout <= 0) {
        hardDeadlineHit = true;
        break;
      }
      const response = await fetchExactOrigin(
        queued.url,
        targetUrl.origin,
        {
          headers: {
            "user-agent":
              "Mozilla/5.0 (compatible; GolishBrowserCollect/1.0; +https://golish.local)",
            },
        },
        fetchTimeout,
      );
      const headers = Object.fromEntries(response.headers.entries());
      const responseKey = canonicalScriptUrl(response.url);
      if (scriptsByUrl.has(responseKey)) {
        clearRecoveryFailure("recursive_fetch", queued.url);
        clearRecoveryFailure("recursive_body", queued.url);
        continue;
      }
      if (!response.ok || !isJavaScriptResponse(response.url, headers)) {
        clearRecoveryFailure("recursive_fetch", queued.url);
        clearRecoveryFailure("recursive_body", queued.url);
        recordRecursiveError(
          queued.url,
          response.status,
          response.ok ? "not-javascript" : `HTTP ${response.status}`,
        );
        continue;
      }
      const contentLength = Number.parseInt(headers["content-length"] ?? "0", 10);
      if (Number.isFinite(maxScriptBytes) && Number.isFinite(contentLength) && contentLength > maxScriptBytes) {
        scriptByteLimitSkips += 1;
        recordRecursiveError(
          response.url,
          response.status,
          `content-length>${maxScriptBytes}`,
        );
        continue;
      }
      const arrayBuffer = await withTimeout(
        response.arrayBuffer(),
        timeLeft(hardDeadline, DEFAULT_BODY_TIMEOUT_MS),
      );
      if (arrayBuffer === TIMEOUT) {
        pendingBodyTimeouts += 1;
        noteRecoveryFailure("recursive_body", queued.url, "recursive body-timeout");
        recursiveRetryQueue.push(queued);
        recordRecursiveError(response.url, response.status, "body-timeout");
        continue;
      }
      clearRecoveryFailure("recursive_fetch", queued.url);
      clearRecoveryFailure("recursive_body", queued.url);
      const body = Buffer.from(arrayBuffer);
      if (Number.isFinite(maxScriptBytes) && body.length > maxScriptBytes) {
        scriptByteLimitSkips += 1;
        recordRecursiveError(response.url, response.status, `body>${maxScriptBytes}`);
        continue;
      }
      const sha = sha256Hex(body);
      let pathOnDisk = scriptPathByHash.get(sha);
      let duplicateOf = null;
      if (pathOnDisk) {
        duplicateContentHits += 1;
        duplicateOf = pathOnDisk;
      } else {
        pathOnDisk = await saveScriptCapture(
          workspace,
          targetHost,
          targetPort,
          targetScheme,
          response.url,
          body,
        );
        scriptPathByHash.set(sha, pathOnDisk);
      }
      scriptsByUrl.set(responseKey, {
        url: response.url,
        path: pathOnDisk,
        size: body.length,
        status: response.status,
        content_type: headers["content-type"] ?? "",
        sha256: sha,
        discovered_by: "chunk_reference",
        duplicate_of: duplicateOf,
      });
      recursiveScriptsDownloaded += 1;
      if (recursiveTraceCount < 120) {
        progress(duplicateOf ? "recursive_script_duplicate" : "recursive_script_saved", {
          count: recursiveScriptsDownloaded,
          status: response.status,
          bytes: body.length,
          path: pathOnDisk,
          url: response.url,
        });
        recursiveTraceCount += 1;
      }
      enqueueRefsFromScript(response.url, body);
    } catch (error) {
      const reason = String(error?.message ?? error).slice(0, 500);
      if (!reason.includes("exact-origin") && !reason.includes("read-only")) {
        noteRecoveryFailure("recursive_fetch", queued.url, reason);
        recursiveRetryQueue.push(queued);
      }
      recordRecursiveError(
        queued.url,
        null,
        reason,
      );
    }
  }

  for (const queued of [...recursiveRetryQueue, ...blockedCheckpointRecursive]) {
    if (
      !queued ||
      scriptsByUrl.has(queued.key) ||
      recursiveQueue.some((item) => item.key === queued.key)
    ) {
      continue;
    }
    recursiveQueue.push(queued);
  }

  if (timeLeft(hardDeadline) <= 0) {
    hardDeadlineHit = true;
  }

  progress("close_browser", {
    scripts_seen: scriptsByUrl.size,
    recursive_downloaded: recursiveScriptsDownloaded,
    api_seen: apiRequestsByKey.size,
    deadline_hit: hardDeadlineHit,
  });
  contextCloseTimedOut = await closeContextHard(context);
  browserCloseTimedOut = await closeBrowserHard(browser);

  const scripts = [...scriptsByUrl.values()].sort((a, b) =>
    a.url.localeCompare(b.url),
  );
  const apiRequests = [...apiRequestsByKey.values()].sort((a, b) =>
    a.url.localeCompare(b.url),
  );
  const scriptsWithPath = scripts.filter((s) => s.path);
  const uniqueScriptPaths = new Set(scriptsWithPath.map((s) => s.path));
  const scriptsObserved = scripts.length;
  const scriptManifestEntries = scriptsWithPath.length;
  const scriptsSaved = uniqueScriptPaths.size;
  const scriptObservations = [...scriptInsightsByUrl.values()]
    .sort((a, b) => b.size - a.size)
    .slice(0, 8);
  const recursiveErrorSummary = summarizeRecursiveErrors(recursiveErrors);
  const aiReviewRefs = [...aiReviewRefsByKey.values()].slice(0, 100);
  const aiAssistReasons = [];
  for (const pendingPage of [...pendingRecoveryPages, ...blockedCheckpointPages]) {
    if (
      !isExactOriginUrl(pendingPage, targetUrl.origin) ||
      isDangerousNavigationUrl(pendingPage)
    ) {
      continue;
    }
    const canonical = canonicalRecoveryUrl(pendingPage);
    seenPages.delete(canonical);
    if (!queuedPages.has(canonical)) {
      queue.push(canonical);
      queuedPages.add(canonical);
    }
  }
  const checkpointRecoveryFailures = recoveryFailureList();
  const recoveryExhaustedCount = checkpointRecoveryFailures.filter(
    (failure) => failure.count >= MAX_RECOVERY_FAILURES,
  ).length;
  const recoveryExhausted = recoveryExhaustedCount > 0;
  const automaticRetryAllowed = !recoveryExhausted;
  const recoveryInstruction = recoveryExhausted
    ? "Start a new trusted producer operation or stage attempt after changing the failing transport/timeout condition; this provenance will not auto-retry again."
    : null;
  if (scriptsSaved === 0) {
    aiAssistReasons.push("no_js_saved");
  }
  if (scriptsSaved > 0 && apiRequests.length === 0) {
    aiAssistReasons.push("js_saved_but_no_runtime_api_requests");
  }
  if (recursiveErrors.length > Math.max(20, scriptsSaved * 2)) {
    aiAssistReasons.push("many_recursive_candidates_failed");
  }
  if (navigationErrors.length > 0) {
    aiAssistReasons.push("navigation_or_interaction_errors");
  }
  if (aiReviewRefs.length > 0) {
    aiAssistReasons.push("relative_js_refs_need_ai_review");
  }
  const recursiveLimitHit =
    Number.isFinite(maxRecursiveScripts) &&
    recursiveQueue.length > 0 &&
    recursiveScriptsDownloaded >= maxRecursiveScripts;
  const recursiveDeadlineHit =
    Number.isFinite(recursiveDeadline) &&
    recursiveQueue.length > 0 &&
    recursiveScriptsDownloaded < maxRecursiveScripts &&
    (Date.now() >= recursiveDeadline || hardDeadlineHit);
  const recursiveScopeViolations = recursiveErrors.filter((entry) =>
    String(entry?.reason ?? "").includes("exact-origin"),
  ).length;
  const scopeViolationCount =
    blockedNavigationUrls.size +
    recursiveScopeViolations;
  // Foreign subresources/WebSockets successfully aborted before request are a
  // completed scope decision, not unfinished in-origin work. Keep them visible
  // as exclusions without making every CDN/analytics-using page permanently
  // partial. Main-navigation/explicit recursive escape attempts remain real
  // closure violations.
  const scopeExclusionCount =
    blockedSubresourceUrls.size +
    blockedWebSocketUrls.size +
    blockedReadOnlyRequests.size +
    blockedReadOnlyRouteUrls.size +
    terminalCrossOriginRedirects.size;
  const completion = classifyCollectionCompletion({
    navigation_attempts: pagesVisitedThisRun.length,
    successful_pages: successfulPages,
    navigation_errors: navigationErrors.length,
    page_queue_remaining: queue.length,
    page_candidates_dropped: pageCandidatesDropped,
    recursive_queue_remaining: recursiveQueue.length,
    recursive_limit_hit: recursiveLimitHit,
    recursive_deadline_hit: recursiveDeadlineHit,
    hard_deadline_hit: hardDeadlineHit,
    pending_wait_timed_out: pendingWaitTimedOut,
    pending_body_timeouts: pendingBodyTimeouts,
    script_byte_limit_skips: scriptByteLimitSkips,
    script_capture_errors: scriptCaptureErrors,
    scope_violations: scopeViolationCount,
    recovery_pending: checkpointRecoveryFailures.length,
    recovery_exhausted: recoveryExhaustedCount,
  });
  const closureIncompleteReasons = completion.reasons;
  const closureComplete = completion.closure_complete;
  const status = completion.status;
  const manifestPath = await writeScriptManifest(
    workspace,
    targetHost,
    targetPort,
    targetScheme,
    scripts,
    completion,
    {
      run_id: producerRunId,
      session_id: producerSessionId,
      operation_id: producerOperationId,
      stage_started_at: producerStageStartedAt,
    },
    {
      visited_pages: [...seenPages],
      pending_pages: [...queue],
      pending_recursive_scripts: recursiveQueue.map(({ url, source }) => ({
        url,
        source,
      })),
      api_requests: apiRequests,
      page_resume_count: checkpointResume?.page_resume_count ?? 0,
      checkpoint_resume_count: checkpointResume?.checkpoint_resume_count ?? 0,
      recovery_failures: checkpointRecoveryFailures,
      recovery_exhausted: recoveryExhausted,
      automatic_retry_allowed: automaticRetryAllowed,
    },
  );
  if (!closureComplete) {
    aiAssistReasons.push("js_closure_incomplete");
  }
  if (
    hardDeadlineHit ||
    pendingWaitTimedOut ||
    pendingBodyTimeouts > 0 ||
    contextCloseTimedOut ||
    browserCloseTimedOut
  ) {
    aiAssistReasons.push("timeout_partial_collection");
  }
  if (recoveryExhausted) {
    aiAssistReasons.push("recovery_exhausted");
  }
  const aiAssist = includeAiAssist
    ? {
        recommended: !recoveryExhausted && aiAssistReasons.length > 0,
        reasons: aiAssistReasons,
        next_step: recoveryExhausted
          ? recoveryInstruction
          : "If recommended, inspect context and call browser_collect_js_api again with a bounded recipe. Do not report endpoints from inference alone; only persisted files/network observations count.",
        recipe_schema: recipeSchema(),
        recipe_applied: recipeSummary,
        context: {
          signals: {
            crawl_mode: crawlMode,
            scripts_saved: scriptsSaved,
            unique_scripts_saved: scriptsSaved,
            scripts_observed: scriptsObserved,
            script_manifest_entries: scriptManifestEntries,
            scripts_duplicate_content_hits: duplicateContentHits,
            api_requests_total: apiRequests.length,
            scripts_recursive_downloaded: recursiveScriptsDownloaded,
            recursive_queue_remaining: recursiveQueue.length,
            closure_complete: closureComplete,
            recursive_errors: recursiveErrors.length,
            recursive_errors_by_status: recursiveErrorSummary.by_status,
            ai_review_refs: aiReviewRefsByKey.size,
            pages_visited: seenPages.size,
            pages_visited_this_run: pagesVisitedThisRun.length,
            successful_pages: successfulPages,
            page_queue_remaining: queue.length,
            page_candidates_dropped: pageCandidatesDropped,
            script_byte_limit_skips: scriptByteLimitSkips,
            scope_violations: scopeViolationCount,
            scope_exclusions: scopeExclusionCount,
            recovery_exhausted: recoveryExhaustedCount,
            automatic_retry_allowed: automaticRetryAllowed,
          },
          recursive_errors_sample: recursiveErrorSummary.sample,
          script_observations: scriptObservations,
          api_requests_sample: apiRequests.slice(0, 20),
          ai_review_refs_sample: aiReviewRefs.slice(0, 30),
          console_errors_sample: consoleErrors.slice(0, 10),
          navigation_errors_sample: navigationErrors.slice(0, 10),
        },
      }
    : null;

  progress("summary", {
    status,
    scripts_saved: scriptsSaved,
    scripts_observed: scriptsObserved,
    script_manifest_entries: scriptManifestEntries,
    api_requests: apiRequests.length,
    recursive_errors: recursiveErrors.length,
    closure_complete: closureComplete,
    completion_state: completion.completion_state,
  });

  await writeJsonAndExit({
        status,
        completion_state: completion.completion_state,
        target_url: targetUrl.href,
        producer_run_id: producerRunId,
        producer_session_id: producerSessionId,
        producer_operation_id: producerOperationId,
        producer_stage_started_at: producerStageStartedAt,
        checkpoint_version: 2,
        checkpoint_resume_applied: Boolean(checkpointResume),
        checkpoint_resume_count: checkpointResume?.checkpoint_resume_count ?? 0,
        automatic_retry_allowed: automaticRetryAllowed,
        recovery_exhausted: recoveryExhausted,
        recovery_instruction: recoveryInstruction,
        recovery_failures: checkpointRecoveryFailures,
        crawl_mode: crawlMode,
        hard_timeout_ms: limitForJson(hardTimeoutMs),
        hard_deadline_hit: hardDeadlineHit,
        pending_body_timeouts: pendingBodyTimeouts,
        api_body_capture_errors: apiBodyCaptureErrors,
        pending_wait_timed_out: pendingWaitTimedOut,
        context_close_timed_out: contextCloseTimedOut,
        browser_close_timed_out: browserCloseTimedOut,
        block_noise: blockNoise,
        same_origin: restrictApisToSameOrigin,
        requested_same_origin: requestedSameOrigin,
        blocked_resource_requests: blockedResourceRequests,
        blocked_navigation_urls: [...blockedNavigationUrls].slice(0, 20),
        terminal_cross_origin_redirects: [...terminalCrossOriginRedirects.values()].slice(0, 20),
        blocked_subresource_urls: [...blockedSubresourceUrls].slice(0, 40),
        blocked_websocket_urls: [...blockedWebSocketUrls].slice(0, 20),
        service_workers: "block",
        read_only_enumeration: true,
        interactive_actions_authorized: false,
        requested_max_actions: requestedMaxActions,
        max_actions: maxActions,
        disabled_recipe_click_texts: requestedRecipeClickTexts.length,
        disabled_recipe_manifest_paths: disabledRecipeManifestPaths,
        disabled_recipe_script_urls: disabledRecipeScriptUrls,
        blocked_read_only_requests: [...blockedReadOnlyRequests].slice(0, 40),
        blocked_read_only_routes: [...blockedReadOnlyRouteUrls].slice(0, 40),
        scope_violations: scopeViolationCount,
        scope_exclusions: scopeExclusionCount,
        pages_visited: [...seenPages],
        pages_visited_this_run: pagesVisitedThisRun,
        page_resume_applied: Boolean(
          checkpointResume &&
            checkpointResume.pending_pages.length > 0 &&
            !recoveryWasExhausted,
        ),
        page_resume_count: checkpointResume?.page_resume_count ?? 0,
        page_resume_prior_visited: checkpointResume?.visited_pages.length ?? 0,
        successful_pages: successfulPages,
        page_queue_remaining: queue.length,
        page_candidates_dropped: pageCandidatesDropped,
        actions_clicked: actionsClicked,
        scripts_total: scriptsObserved,
        scripts_observed: scriptsObserved,
        script_manifest_entries: scriptManifestEntries,
        unique_scripts_saved: scriptsSaved,
        scripts_saved: scriptsSaved,
        scripts_cached_preloaded: cachedScriptsPreloaded,
        scripts_duplicate_content_hits: duplicateContentHits,
        script_manifest: path.relative(workspace, manifestPath),
        script_manifest_stale_entries: manifestCache.stale,
        script_byte_limit_skips: scriptByteLimitSkips,
        script_capture_errors: scriptCaptureErrors,
        scripts_recursive_downloaded: recursiveScriptsDownloaded,
        max_script_bytes: limitForJson(maxScriptBytes),
        max_recursive_scripts: limitForJson(maxRecursiveScripts),
        recursive_resume_applied: Boolean(
          checkpointResume &&
            checkpointResume.pending_recursive_scripts.length > 0 &&
            !recoveryWasExhausted,
        ),
        recursive_resume_prior_pending:
          checkpointResume?.pending_recursive_scripts.length ?? 0,
        recursive_queue_remaining: recursiveQueue.length,
        recursive_deadline_hit: recursiveDeadlineHit,
        recursive_limit_hit: recursiveLimitHit,
        closure_complete: closureComplete,
        closure_incomplete_reasons: closureIncompleteReasons,
        recursive_errors_total: recursiveErrors.length,
        recursive_errors_by_status: recursiveErrorSummary.by_status,
        recursive_errors_truncated: recursiveErrors.length > recursiveErrorSummary.sample.length,
        recursive_errors: recursiveErrorSummary.sample,
        ai_review_refs_total: aiReviewRefsByKey.size,
        ai_review_refs_truncated: aiReviewRefsByKey.size > aiReviewRefs.length,
        ai_review_refs: aiReviewRefs,
        recipe_applied: recipeSummary,
        ai_assist: aiAssist,
        scripts,
        api_requests_total: apiRequests.length,
        api_requests: apiRequests,
        console_errors: consoleErrors.slice(0, 20),
        navigation_errors: navigationErrors,
        output_dir: captureDirectoryFor(workspace, targetUrl, "js"),
      });
}

function matchesSafeNavigationMethod(method) {
  return method === "GET" || method === "HEAD";
}

const invokedDirectly =
  process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack ?? error)}\n`);
    process.exit(1);
  });
}
