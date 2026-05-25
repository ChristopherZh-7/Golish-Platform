#!/usr/bin/env node
/**
 * generate-model-constants.mjs
 *
 * Reads `resources/llm-models/<provider>.json` (data source) and
 * `frontend/scripts/model-const-keys.json` (TS const KEY → model ID mapping)
 * and emits `frontend/lib/ai/models.generated.ts` containing one
 * `export const XXX_MODELS = { ... } as const` per provider.
 *
 * Design: see `docs/design/2026-05-25-llm-models-json-driven.md` (Phase 3).
 *
 * Safety checks before writing:
 *   1. every mapped ID must exist in the provider's JSON data file
 *   2. every JSON model ID *may* be unmapped (we just won't expose a constant
 *      for it; the runtime registry still serves it via Tauri commands)
 *
 * Failures abort with a non-zero exit so prebuild hooks bubble up.
 */

import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(here, "..", "..");
const RESOURCES_DIR = join(REPO_ROOT, "resources", "llm-models");
const KEYS_PATH = join(here, "model-const-keys.json");
const OUTPUT_PATH = join(REPO_ROOT, "frontend", "lib", "ai", "models.generated.ts");

/** Load and JSON-parse a file. */
async function loadJson(path) {
  const text = await readFile(path, "utf8");
  try {
    return JSON.parse(text);
  } catch (err) {
    throw new Error(`Failed to parse JSON at ${path}: ${err.message}`);
  }
}

/** Render one `XXX_MODELS = { KEY: "id" } as const` block. */
function renderConstBlock(constName, mapping) {
  const entries = Object.entries(mapping)
    .map(([key, id]) => `  ${key}: ${JSON.stringify(id)},`)
    .join("\n");
  return `export const ${constName} = {\n${entries}\n} as const;`;
}

async function main() {
  const keysFile = await loadJson(KEYS_PATH);
  const providers = keysFile.providers;
  if (!providers) {
    throw new Error(`${KEYS_PATH} must define a "providers" field`);
  }

  const blocks = [];
  const allProviders = Object.keys(providers).sort();

  for (const providerSlug of allProviders) {
    const constsForProvider = providers[providerSlug];
    if (!constsForProvider || typeof constsForProvider !== "object") {
      throw new Error(`provider "${providerSlug}" entry must be an object`);
    }

    // Load the matching `resources/llm-models/<slug>.json` to validate IDs.
    const dataPath = join(RESOURCES_DIR, `${providerSlug}.json`);
    let availableIds = new Set();
    try {
      const dataFile = await loadJson(dataPath);
      const models = Array.isArray(dataFile.models) ? dataFile.models : [];
      availableIds = new Set(models.map((m) => m.id));
    } catch (err) {
      throw new Error(
        `Cannot read provider data file ${dataPath} (referenced by ${KEYS_PATH}): ${err.message}`
      );
    }

    // Each provider may have multiple const blocks (currently one, but the
    // schema is flexible — e.g. NVIDIA + extra grouping in the future).
    for (const [constName, mapping] of Object.entries(constsForProvider)) {
      // Validate each mapped ID actually exists in the JSON data file.
      const missing = [];
      for (const [keyName, modelId] of Object.entries(mapping)) {
        if (!availableIds.has(modelId)) {
          missing.push(`  ${constName}.${keyName} → "${modelId}"`);
        }
      }
      if (missing.length > 0) {
        throw new Error(
          `\n${KEYS_PATH} references model IDs that do not exist in ${dataPath}:\n${missing.join(
            "\n"
          )}\n\nEither add the ID to the JSON data file or remove it from the const-key map.`
        );
      }

      blocks.push({ constName, code: renderConstBlock(constName, mapping) });
    }
  }

  // Sort generated const blocks by name for stable diffs.
  blocks.sort((a, b) => a.constName.localeCompare(b.constName));

  const header = `/**
 * AUTO-GENERATED FROM \`resources/llm-models/<provider>.json\` AND
 * \`frontend/scripts/model-const-keys.json\`. DO NOT EDIT BY HAND.
 *
 * Regenerate with:
 *   node frontend/scripts/generate-model-constants.mjs
 *
 * See \`docs/design/2026-05-25-llm-models-json-driven.md\` for the rationale.
 */
`;

  const body = blocks.map((b) => b.code).join("\n\n");
  const output = `${header}\n${body}\n`;

  await writeFile(OUTPUT_PATH, output, "utf8");
  // eslint-disable-next-line no-console
  console.log(
    `[generate-model-constants] wrote ${OUTPUT_PATH} (${blocks.length} constants)`
  );
}

main().catch((err) => {
  // eslint-disable-next-line no-console
  console.error(`[generate-model-constants] FAILED: ${err.message}`);
  process.exit(1);
});
