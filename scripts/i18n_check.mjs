#!/usr/bin/env node
// i18n consistency guard for the frontend.
//
// Validates three independent invariants and exits non-zero on any breach:
//   1. Every literal `t("key.path")` / `t('key.path', ...)` / `<Trans i18nKey="key.path">`
//      reference resolves to an existing leaf in BOTH en.json and zh-CN.json.
//   2. Every leaf in en.json has a matching leaf in zh-CN.json (and vice versa),
//      i.e. translation tables are key-aligned.
//   3. No `t("...", "<contains CJK>")` calls — using a Chinese literal as the
//      i18next default value defeats the whole point of having an `en` table:
//      English users would see Chinese when the key is missing.
//
// CI-friendly. Report mode (`--report`) prints offenders without failing,
// useful for incremental clean-up.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const FRONTEND = join(REPO_ROOT, "frontend");
const EN_PATH = join(FRONTEND, "lib/i18n/en.json");
const ZH_PATH = join(FRONTEND, "lib/i18n/zh-CN.json");

const argv = process.argv.slice(2);
const REPORT_ONLY = argv.includes("--report");

function flatten(obj, prefix = "") {
  const out = [];
  for (const [k, v] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      out.push(...flatten(v, path));
    } else {
      out.push(path);
    }
  }
  return out;
}

function loadJson(p) {
  return JSON.parse(readFileSync(p, "utf8"));
}

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (entry === "node_modules" || entry === "dist" || entry === "test") continue;
      yield* walk(p);
    } else if (/\.(ts|tsx)$/.test(entry) && !/\.test\.(ts|tsx)$/.test(entry)) {
      yield p;
    }
  }
}

const CJK = /[\u3400-\u9fff]/;

const T_LITERAL = /\bt\(\s*(["'`])([A-Za-z0-9_.-]+)\1/g;
const T_WITH_DEFAULT = /\bt\(\s*(["'`])([A-Za-z0-9_.-]+)\1\s*,\s*(["'`])([^"'`]*?)\3/g;
const TRANS_KEY = /<Trans[^>]*\bi18nKey\s*=\s*(["'`])([A-Za-z0-9_.-]+)\1/g;

function scanFrontend() {
  const usedKeys = new Map();
  const cjkDefaults = [];
  for (const file of walk(FRONTEND)) {
    const src = readFileSync(file, "utf8");
    const rel = file.slice(REPO_ROOT.length + 1);
    const lineStarts = [0];
    for (let i = 0; i < src.length; i++) if (src[i] === "\n") lineStarts.push(i + 1);
    const locOf = (idx) => {
      let lo = 0;
      let hi = lineStarts.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if (lineStarts[mid] <= idx) lo = mid;
        else hi = mid - 1;
      }
      return lo + 1;
    };
    let m;
    T_LITERAL.lastIndex = 0;
    while ((m = T_LITERAL.exec(src)) !== null) {
      const key = m[2];
      if (!usedKeys.has(key)) usedKeys.set(key, []);
      usedKeys.get(key).push(`${rel}:${locOf(m.index)}`);
    }
    TRANS_KEY.lastIndex = 0;
    while ((m = TRANS_KEY.exec(src)) !== null) {
      const key = m[2];
      if (!usedKeys.has(key)) usedKeys.set(key, []);
      usedKeys.get(key).push(`${rel}:${locOf(m.index)}`);
    }
    T_WITH_DEFAULT.lastIndex = 0;
    while ((m = T_WITH_DEFAULT.exec(src)) !== null) {
      const fallback = m[4];
      if (CJK.test(fallback)) {
        cjkDefaults.push({ file: rel, line: locOf(m.index), key: m[2], fallback });
      }
    }
  }
  return { usedKeys, cjkDefaults };
}

function main() {
  const en = loadJson(EN_PATH);
  const zh = loadJson(ZH_PATH);
  const enKeys = new Set(flatten(en));
  const zhKeys = new Set(flatten(zh));

  const onlyEn = [...enKeys].filter((k) => !zhKeys.has(k));
  const onlyZh = [...zhKeys].filter((k) => !enKeys.has(k));

  const { usedKeys, cjkDefaults } = scanFrontend();
  const missingInEn = [];
  const missingInZh = [];
  for (const key of usedKeys.keys()) {
    if (!enKeys.has(key)) missingInEn.push(key);
    if (!zhKeys.has(key)) missingInZh.push(key);
  }

  let problems = 0;

  if (onlyEn.length || onlyZh.length) {
    problems++;
    console.error(`[i18n-check] ✗ key tables drift between en.json and zh-CN.json`);
    if (onlyEn.length) console.error(`  - only in en.json (${onlyEn.length}):`, onlyEn.slice(0, 20));
    if (onlyZh.length)
      console.error(`  - only in zh-CN.json (${onlyZh.length}):`, onlyZh.slice(0, 20));
  }

  if (missingInEn.length) {
    problems++;
    console.error(
      `[i18n-check] ✗ ${missingInEn.length} t() / <Trans> key(s) referenced in code but missing in en.json:`
    );
    for (const k of missingInEn.slice(0, 50)) {
      console.error(`  - ${k}  (used at ${usedKeys.get(k).slice(0, 3).join(", ")})`);
    }
    if (missingInEn.length > 50) console.error(`  ... and ${missingInEn.length - 50} more`);
  }

  if (missingInZh.length) {
    problems++;
    console.error(
      `[i18n-check] ✗ ${missingInZh.length} t() / <Trans> key(s) referenced in code but missing in zh-CN.json:`
    );
    for (const k of missingInZh.slice(0, 50)) {
      console.error(`  - ${k}  (used at ${usedKeys.get(k).slice(0, 3).join(", ")})`);
    }
    if (missingInZh.length > 50) console.error(`  ... and ${missingInZh.length - 50} more`);
  }

  if (cjkDefaults.length) {
    problems++;
    console.error(
      `[i18n-check] ✗ ${cjkDefaults.length} t(key, "<Chinese fallback>") call(s) — Chinese literal as default leaks to English users when the key is missing:`
    );
    for (const c of cjkDefaults.slice(0, 50)) {
      console.error(`  - ${c.file}:${c.line}  t("${c.key}", "${c.fallback}")`);
    }
    if (cjkDefaults.length > 50) console.error(`  ... and ${cjkDefaults.length - 50} more`);
  }

  if (problems === 0) {
    console.log(
      `[i18n-check] ✓ ${enKeys.size} keys aligned · ${usedKeys.size} t()/<Trans> references all resolved · no Chinese fallback literals`
    );
    process.exit(0);
  }

  if (REPORT_ONLY) {
    console.error(`[i18n-check] (report mode) ${problems} category problems found, exiting 0`);
    process.exit(0);
  }
  process.exit(1);
}

main();
