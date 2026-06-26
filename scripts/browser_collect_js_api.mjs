#!/usr/bin/env node
import { chromium } from "@playwright/test";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_MAX_PAGES = 6;
const DEFAULT_MAX_ACTIONS = 8;
const DEFAULT_MAX_SCRIPT_BYTES = 5_000_000;
const DEFAULT_MAX_RECURSIVE_SCRIPTS = 200;
const DEFAULT_DEEP_TIMEOUT_MS = 60_000;
const DEFAULT_DEEP_MAX_PAGES = 12;
const DEFAULT_DEEP_MAX_ACTIONS = 12;
const DEFAULT_DEEP_MAX_RECURSIVE_SCRIPTS = 1_000;
const DEFAULT_FETCH_TIMEOUT_MS = 5_000;
const DEFAULT_BODY_TIMEOUT_MS = 3_000;
const DEFAULT_CONTEXT_CLOSE_TIMEOUT_MS = 2_000;
const DEFAULT_CLOSE_TIMEOUT_MS = 3_000;
const TIMEOUT = Symbol("timeout");

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

function toBool(value, fallback) {
  if (value == null) return fallback;
  if (typeof value === "boolean") return value;
  return ["1", "true", "yes", "y"].includes(String(value).toLowerCase());
}

function parseCrawlMode(value) {
  return String(value ?? "fast").toLowerCase() === "deep" ? "deep" : "fast";
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
    click_texts: ["visible non-destructive text to click, e.g. API or Settings"],
    public_path: "optional public path override for chunk_pairs",
    chunk_pairs: [{ id: "123", hash: "abcdef1234" }],
  };
}

async function fetchWithTimeout(url, options = {}, timeoutMs = DEFAULT_FETCH_TIMEOUT_MS) {
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

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function withTimeout(promise, timeoutMs) {
  const guarded = Promise.resolve(promise);
  if (timeoutMs <= 0) {
    guarded.catch(() => {});
    return TIMEOUT;
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
  const remaining = Math.max(0, deadlineMs - Date.now());
  return capMs == null ? remaining : Math.min(capMs, remaining);
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

function scanJsForReferences(text) {
  const patterns = [
    /import\s*\(\s*["']([^"']+\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']\s*\)/g,
    /["']((?:\.{0,2}\/)[^"']*?\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
    /["']((?:assets|static|js|dist|build|chunks?|vendor|_next\/static)\/[^"']*?\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
    /\b[A-Za-z_$][A-Za-z0-9_$]*\s*\+\s*["']([^"']*\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
    /["']([^"']*\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']\s*\+\s*[A-Za-z_$][A-Za-z0-9_$]*/g,
    /\b(?:src|file|path|chunk|url)["']?\s*:\s*["']([^"']+\.(?:js|mjs|cjs)(?:\?[^"']*)?)["']/g,
  ];

  const refs = [];
  const seen = new Set();
  for (const pattern of patterns) {
    for (const match of text.matchAll(pattern)) {
      const ref = match[1];
      if (looksLikeJsRef(ref) && !seen.has(ref)) {
        seen.add(ref);
        refs.push(ref);
      }
    }
  }
  return refs;
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

async function saveScriptCapture(workspace, targetHost, targetPort, scriptUrl, body) {
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

function scriptManifestPath(workspace, targetHost, targetPort) {
  return path.join(
    workspace,
    ".golish",
    "captures",
    targetHost,
    String(targetPort),
    "js",
    "manifest.json",
  );
}

async function loadScriptManifest(workspace, targetHost, targetPort, maxScriptBytes) {
  const manifestPath = scriptManifestPath(workspace, targetHost, targetPort);
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
      if (!stat.isFile() || stat.size > maxScriptBytes) {
        stale += 1;
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
    return { path: manifestPath, entries, stale };
  } catch {
    return { path: manifestPath, entries: [], stale: 0 };
  }
}

async function writeScriptManifest(workspace, targetHost, targetPort, scripts) {
  const manifestPath = scriptManifestPath(workspace, targetHost, targetPort);
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
  const payload = {
    version: 1,
    updated_at: new Date().toISOString(),
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

function shouldSkipClick(text) {
  return /(logout|log out|delete|remove|checkout|purchase|buy|pay|submit|sign out|退出|删除|支付|购买|提交)/i.test(
    text,
  );
}

async function collectSameOriginLinks(page, origin, limit) {
  const links = await page.evaluate(
    ({ origin, limit }) =>
      Array.from(document.querySelectorAll("a[href]"))
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
        })
        .slice(0, limit),
    { origin, limit },
  );
  return [...new Set(links)];
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
  const crawlMode = parseCrawlMode(args.crawl_mode);
  const deepMode = crawlMode === "deep";
  const timeoutMs = toInt(
    args.timeout_ms,
    deepMode ? DEFAULT_DEEP_TIMEOUT_MS : DEFAULT_TIMEOUT_MS,
    5_000,
    300_000,
  );
  const hardTimeoutMs = toInt(
    args.hard_timeout_ms,
    deepMode ? Math.max(timeoutMs + 90_000, 120_000) : timeoutMs + 20_000,
    10_000,
    600_000,
  );
  const hardDeadline = Date.now() + hardTimeoutMs;
  const maxPages = toInt(
    args.max_pages,
    deepMode ? DEFAULT_DEEP_MAX_PAGES : DEFAULT_MAX_PAGES,
    1,
    100,
  );
  const maxActions = toInt(
    args.max_actions,
    deepMode ? DEFAULT_DEEP_MAX_ACTIONS : DEFAULT_MAX_ACTIONS,
    0,
    100,
  );
  const maxScriptBytes = toInt(
    args.max_script_bytes,
    DEFAULT_MAX_SCRIPT_BYTES,
    100_000,
    50_000_000,
  );
  const maxRecursiveScripts = toInt(
    args.max_recursive_scripts,
    deepMode ? DEFAULT_DEEP_MAX_RECURSIVE_SCRIPTS : DEFAULT_MAX_RECURSIVE_SCRIPTS,
    0,
    10_000,
  );
  const restrictApisToSameOrigin = toBool(args.same_origin, true);
  const includeAiAssist = toBool(args.ai_assist, true);
  const blockNoise = toBool(args.block_noise, true);
  const recipe = parseRecipe(args.recipe_json);
  const recipeRoutes = safeStringArray(recipe.routes, 20)
    .map((route) => resolveSameOriginUrl(route, targetUrl))
    .filter(Boolean);
  const recipeManifestPaths = safeStringArray(recipe.manifest_paths, 20)
    .map((route) => resolveSameOriginPath(route, targetUrl))
    .filter(Boolean);
  const recipeScriptUrls = safeStringArray(recipe.script_urls, deepMode ? 500 : 100)
    .map((route) => resolveSameOriginUrl(route, targetUrl))
    .filter(Boolean);
  const recipeClickTexts = safeStringArray(recipe.click_texts, 20, 120);
  const recipePublicPath =
    typeof recipe.public_path === "string" ? recipe.public_path.trim().slice(0, 300) : null;
  const recipeChunkPairs = safeChunkPairs(recipe.chunk_pairs, deepMode ? 1_000 : 200);
  const recipeSummary = {
    crawl_mode: crawlMode,
    routes: recipeRoutes.length,
    manifest_paths: recipeManifestPaths.length,
    script_urls: recipeScriptUrls.length,
    click_texts: recipeClickTexts.length,
    public_path: recipePublicPath || null,
    chunk_pairs: recipeChunkPairs.length,
  };

  const browser = await withTimeout(launchBrowser(), timeLeft(hardDeadline, 15_000));
  if (browser === TIMEOUT) {
    throw new Error(`browser launch timed out after ${hardTimeoutMs} ms hard deadline`);
  }
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    userAgent:
      "Mozilla/5.0 (compatible; GolishBrowserCollect/1.0; +https://golish.local)",
  });
  let blockedResourceRequests = 0;
  await context.route("**/*", async (route) => {
    const request = route.request();
    const type = request.resourceType();
    const url = request.url();
    if (isBlockedResourceType(type) || (blockNoise && isNoiseUrl(url))) {
      blockedResourceRequests += 1;
      await route.abort().catch(() => {});
      return;
    }
    await route.continue().catch(() => {});
  });
  const page = await context.newPage();

  const scriptsByUrl = new Map();
  const scriptInsightsByUrl = new Map();
  const recursiveQueue = [];
  const queuedRecursiveUrls = new Set();
  const scriptPathByHash = new Map();
  const scannedScriptHashes = new Set();
  const apiRequestsByKey = new Map();
  const pending = new Set();
  const navigationErrors = [];
  const consoleErrors = [];
  const recursiveErrors = [];
  let publicPathHint = null;
  let recursiveScriptsDownloaded = 0;
  let hardDeadlineHit = false;
  let pendingBodyTimeouts = 0;
  let pendingWaitTimedOut = false;
  let contextCloseTimedOut = false;
  let browserCloseTimedOut = false;
  let duplicateContentHits = 0;

  const recordRecursiveError = (url, status, reason) => {
    if (recursiveErrors.length >= 100) return;
    recursiveErrors.push({
      url,
      status,
      reason,
    });
  };

  const enqueueScriptUrl = (url, source) => {
    if (!url) return;
    const key = canonicalScriptUrl(url);
    if (!key || scriptsByUrl.has(key) || queuedRecursiveUrls.has(key)) return;
    try {
      const parsed = new URL(url);
      const sourceOrigin = source ? new URL(source).origin : targetUrl.origin;
      if (parsed.origin !== targetUrl.origin && parsed.origin !== sourceOrigin) return;
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
    const refs = scanJsForReferences(text);
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
    maxScriptBytes,
  );
  let cachedScriptsPreloaded = 0;
  for (const entry of manifestCache.entries) {
    if (!entry.key || scriptsByUrl.has(entry.key)) continue;
    let body = null;
    try {
      body = await fs.readFile(entry.full_path);
    } catch {
      continue;
    }
    const sha = entry.sha256 || sha256Hex(body);
    const cached = {
      url: entry.url,
      path: entry.path,
      size: entry.size,
      status: entry.status,
      content_type: entry.content_type,
      sha256: sha,
      cached: true,
    };
    scriptsByUrl.set(entry.key, cached);
    scriptPathByHash.set(sha, entry.path);
    cachedScriptsPreloaded += 1;
    enqueueRefsFromScript(entry.url, body);
  }

  for (const scriptUrl of recipeScriptUrls) {
    enqueueScriptUrl(scriptUrl, targetUrl.href);
  }
  if (recipePublicPath && recipeChunkPairs.length > 0) {
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
        const response = await fetchWithTimeout(
          manifestUrl,
          {
            redirect: "follow",
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
          recordRecursiveError(manifestUrl, response.status, "manifest body-timeout");
          continue;
        }
        fetched += 1;
        for (const ref of [
          ...scanJsForReferences(body),
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
    const type = request.resourceType();
    if (type !== "xhr" && type !== "fetch") return;
    const url = request.url();
    if (restrictApisToSameOrigin && !sameOrigin(url, targetUrl.origin)) return;
    const key = `${request.method()} ${url}`;
    if (!apiRequestsByKey.has(key)) {
      apiRequestsByKey.set(key, {
        url,
        method: request.method(),
        resource_type: type,
        status: null,
      });
    }
  });

  page.on("response", (response) => {
    const request = response.request();
    const url = response.url();
    const headers = response.headers();
    const type = request.resourceType();

    if ((type === "xhr" || type === "fetch") && apiRequestsByKey.has(`${request.method()} ${url}`)) {
      apiRequestsByKey.get(`${request.method()} ${url}`).status = response.status();
    }

    const scriptKey = canonicalScriptUrl(url);
    if (!isJavaScriptResponse(url, headers) || scriptsByUrl.has(scriptKey)) return;
    const contentLength = Number.parseInt(headers["content-length"] ?? "0", 10);
    if (Number.isFinite(contentLength) && contentLength > maxScriptBytes) {
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
          scriptsByUrl.set(scriptKey, {
            url,
            status: response.status(),
            skipped: true,
            reason: "body-timeout",
          });
          return;
        }
        if (body.length > maxScriptBytes) {
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
      })()
      .catch((error) => {
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

  const queue = [targetUrl.href, ...recipeRoutes];
  const seenPages = new Set();
  let actionsClicked = 0;

  while (
    queue.length > 0 &&
    seenPages.size < maxPages &&
    timeLeft(hardDeadline) > 0
  ) {
    const nextUrl = queue.shift();
    if (!nextUrl || seenPages.has(nextUrl)) continue;
    seenPages.add(nextUrl);

    try {
      const navTimeout = timeLeft(hardDeadline, Math.min(timeoutMs, 10_000));
      if (navTimeout <= 0) {
        hardDeadlineHit = true;
        break;
      }
      await page.goto(nextUrl, {
        waitUntil: "commit",
        timeout: Math.max(1, navTimeout),
      });
      const domTimeout = timeLeft(hardDeadline, 5_000);
      if (domTimeout > 0) {
        await page
          .waitForLoadState("domcontentloaded", { timeout: Math.max(1, domTimeout) })
          .catch(() => {});
      }
    } catch (error) {
      navigationErrors.push({
        url: nextUrl,
        error: String(error?.message ?? error).slice(0, 500),
      });
      if (page.url() === "about:blank") {
        continue;
      }
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

      const domScriptUrls = await withTimeout(
        collectDomScriptCandidates(
          page,
          targetUrl.origin,
          Math.max(maxRecursiveScripts, maxPages * 10, 50),
        ),
        timeLeft(hardDeadline, 3_000),
      );
      if (domScriptUrls !== TIMEOUT) {
        for (const scriptUrl of domScriptUrls) {
          enqueueScriptUrl(scriptUrl, page.url());
        }
      }

      const links = await withTimeout(
        collectSameOriginLinks(page, targetUrl.origin, Math.max(maxPages * 3, 10)),
        timeLeft(hardDeadline, 3_000),
      );
      if (links === TIMEOUT) {
        navigationErrors.push({
          url: page.url(),
          error: "collect-links-timeout",
        });
        continue;
      }
      for (const link of links) {
        if (!seenPages.has(link) && queue.length + seenPages.size < maxPages) {
          queue.push(link);
        }
      }
    } catch (error) {
      navigationErrors.push({
        url: page.url(),
        error: String(error?.message ?? error).slice(0, 500),
      });
    }
  }

  if (timeLeft(hardDeadline) <= 0) {
    hardDeadlineHit = true;
  }

  const pendingResult = await withTimeout(
    Promise.allSettled([...pending]),
    timeLeft(hardDeadline, 3_000),
  );
  if (pendingResult === TIMEOUT) {
    pendingWaitTimedOut = true;
  }

  if (timeLeft(hardDeadline) > 0) {
    await fetchManifests();
  } else {
    hardDeadlineHit = true;
  }

  const recursiveDeadline = Math.min(
    hardDeadline,
    Date.now() + (deepMode ? Math.min(hardTimeoutMs, 180_000) : Math.min(timeoutMs, 30_000)),
  );
  while (
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
      const response = await fetchWithTimeout(
        queued.url,
        {
          redirect: "follow",
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
        continue;
      }
      if (!response.ok || !isJavaScriptResponse(response.url, headers)) {
        recordRecursiveError(
          queued.url,
          response.status,
          response.ok ? "not-javascript" : `HTTP ${response.status}`,
        );
        continue;
      }
      const contentLength = Number.parseInt(headers["content-length"] ?? "0", 10);
      if (Number.isFinite(contentLength) && contentLength > maxScriptBytes) {
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
        recordRecursiveError(response.url, response.status, "body-timeout");
        continue;
      }
      const body = Buffer.from(arrayBuffer);
      if (body.length > maxScriptBytes) {
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
      enqueueRefsFromScript(response.url, body);
    } catch (error) {
      recordRecursiveError(
        queued.url,
        null,
        String(error?.message ?? error).slice(0, 500),
      );
    }
  }

  if (timeLeft(hardDeadline) <= 0) {
    hardDeadlineHit = true;
  }

  contextCloseTimedOut = await closeContextHard(context);
  browserCloseTimedOut = await closeBrowserHard(browser);

  const scripts = [...scriptsByUrl.values()].sort((a, b) =>
    a.url.localeCompare(b.url),
  );
  const manifestPath = await writeScriptManifest(
    workspace,
    targetHost,
    targetPort,
    scripts,
  );
  const apiRequests = [...apiRequestsByKey.values()].sort((a, b) =>
    a.url.localeCompare(b.url),
  );
  const scriptsSaved = scripts.filter((s) => s.path).length;
  const scriptObservations = [...scriptInsightsByUrl.values()]
    .sort((a, b) => b.size - a.size)
    .slice(0, 8);
  const aiAssistReasons = [];
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
  const recursiveLimitHit =
    recursiveQueue.length > 0 && recursiveScriptsDownloaded >= maxRecursiveScripts;
  const recursiveDeadlineHit =
    recursiveQueue.length > 0 &&
    recursiveScriptsDownloaded < maxRecursiveScripts &&
    (Date.now() >= recursiveDeadline || hardDeadlineHit);
  const closureIncompleteReasons = [];
  if (recursiveQueue.length > 0) closureIncompleteReasons.push("recursive_queue_remaining");
  if (recursiveLimitHit) closureIncompleteReasons.push("max_recursive_scripts_hit");
  if (recursiveDeadlineHit) closureIncompleteReasons.push("recursive_deadline_hit");
  if (hardDeadlineHit) closureIncompleteReasons.push("hard_deadline_hit");
  if (pendingWaitTimedOut) closureIncompleteReasons.push("pending_wait_timed_out");
  if (pendingBodyTimeouts > 0) closureIncompleteReasons.push("pending_body_timeouts");
  const closureComplete = closureIncompleteReasons.length === 0;
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
  const aiAssist = includeAiAssist
    ? {
        recommended: aiAssistReasons.length > 0,
        reasons: aiAssistReasons,
        next_step:
          "If recommended, inspect context and call browser_collect_js_api again with a bounded recipe. Do not report endpoints from inference alone; only persisted files/network observations count.",
        recipe_schema: recipeSchema(),
        recipe_applied: recipeSummary,
        context: {
          signals: {
            crawl_mode: crawlMode,
            scripts_saved: scriptsSaved,
            api_requests_total: apiRequests.length,
            scripts_recursive_downloaded: recursiveScriptsDownloaded,
            recursive_queue_remaining: recursiveQueue.length,
            closure_complete: closureComplete,
            recursive_errors: recursiveErrors.length,
            pages_visited: seenPages.size,
          },
          recursive_errors_sample: recursiveErrors.slice(0, 20),
          script_observations: scriptObservations,
          api_requests_sample: apiRequests.slice(0, 20),
          console_errors_sample: consoleErrors.slice(0, 10),
          navigation_errors_sample: navigationErrors.slice(0, 10),
        },
      }
    : null;
  const status =
    hardDeadlineHit ||
    pendingWaitTimedOut ||
    pendingBodyTimeouts > 0
      ? "timeout_partial"
      : !closureComplete
        ? "closure_partial"
      : "ok";

  await writeJsonAndExit({
        status,
        target_url: targetUrl.href,
        crawl_mode: crawlMode,
        hard_timeout_ms: hardTimeoutMs,
        hard_deadline_hit: hardDeadlineHit,
        pending_body_timeouts: pendingBodyTimeouts,
        pending_wait_timed_out: pendingWaitTimedOut,
        context_close_timed_out: contextCloseTimedOut,
        browser_close_timed_out: browserCloseTimedOut,
        block_noise: blockNoise,
        blocked_resource_requests: blockedResourceRequests,
        pages_visited: [...seenPages],
        actions_clicked: actionsClicked,
        scripts_total: scripts.length,
        scripts_saved: scriptsSaved,
        scripts_cached_preloaded: cachedScriptsPreloaded,
        scripts_duplicate_content_hits: duplicateContentHits,
        script_manifest: path.relative(workspace, manifestPath),
        script_manifest_stale_entries: manifestCache.stale,
        scripts_recursive_downloaded: recursiveScriptsDownloaded,
        max_recursive_scripts: maxRecursiveScripts,
        recursive_queue_remaining: recursiveQueue.length,
        recursive_deadline_hit: recursiveDeadlineHit,
        recursive_limit_hit: recursiveLimitHit,
        closure_complete: closureComplete,
        closure_incomplete_reasons: closureIncompleteReasons,
        recursive_errors: recursiveErrors,
        recipe_applied: recipeSummary,
        ai_assist: aiAssist,
        scripts,
        api_requests_total: apiRequests.length,
        api_requests: apiRequests,
        console_errors: consoleErrors.slice(0, 20),
        navigation_errors: navigationErrors,
        output_dir: path.join(
          workspace,
          ".golish",
          "captures",
          targetHost,
          String(targetPort),
          "js",
        ),
      });
}

main().catch((error) => {
  process.stderr.write(`${String(error?.stack ?? error)}\n`);
  process.exit(1);
});
