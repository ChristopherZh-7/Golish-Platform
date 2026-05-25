/**
 * Unit tests for the JSON-driven model constants pipeline.
 *
 * These act as a safety net for the build-time generator
 * (`frontend/scripts/generate-model-constants.mjs`) plus the
 * `frontend/scripts/model-const-keys.json` mapping. If any of these tests
 * fail, the generated `frontend/lib/ai/models.generated.ts` is out of sync
 * with the upstream data.
 *
 * See `docs/design/2026-05-25-llm-models-json-driven.md`.
 */

import { describe, expect, it } from "vitest";
import keysFile from "../../scripts/model-const-keys.json";
import * as models from "@/lib/ai/models";

interface ProviderModelsFile {
  provider: string;
  models: { id: string; display_name?: string }[];
}

// `import.meta.glob` is provided by Vite; the `eager: true` option loads
// every JSON file at module-evaluation time so the tests are synchronous.
const providerFilesByPath = import.meta.glob<ProviderModelsFile>(
  "../../../resources/llm-models/*.json",
  { eager: true, import: "default" }
);

// Map "../../../resources/llm-models/nvidia.json" -> "nvidia".
function slugFromPath(path: string): string {
  const match = path.match(/\/([^/]+)\.json$/);
  if (!match) {
    throw new Error(`Cannot extract provider slug from path: ${path}`);
  }
  return match[1];
}

const providerData: Record<string, ProviderModelsFile> = {};
for (const [path, file] of Object.entries(providerFilesByPath)) {
  providerData[slugFromPath(path)] = file;
}

const keysByProvider = (keysFile as {
  providers: Record<string, Record<string, Record<string, string>>>;
}).providers;

describe("models.generated", () => {
  it("exports one constant per provider entry in model-const-keys.json", () => {
    const exported = new Set(
      Object.keys(models as Record<string, unknown>).filter((name) =>
        name.endsWith("_MODELS")
      )
    );
    const expected = new Set<string>();
    for (const providerEntry of Object.values(keysByProvider)) {
      for (const constName of Object.keys(providerEntry)) {
        expected.add(constName);
      }
    }
    expect(exported).toEqual(expected);
  });

  it.each(Object.keys(keysByProvider))(
    "%s: every mapped ID exists in the matching JSON data file",
    (providerSlug) => {
      const data = providerData[providerSlug];
      expect(data, `missing resources/llm-models/${providerSlug}.json`).toBeDefined();
      const availableIds = data.models.map((m) => m.id);

      for (const mapping of Object.values(keysByProvider[providerSlug])) {
        for (const [keyName, modelId] of Object.entries(mapping)) {
          expect(
            availableIds,
            `${providerSlug}: const-key ${keyName} → "${modelId}" not in ${providerSlug}.json`
          ).toContain(modelId);
        }
      }
    }
  );

  it.each(Object.entries(keysByProvider))(
    "%s: generated constant matches the const-key map exactly",
    (providerSlug, providerEntry) => {
      for (const [constName, mapping] of Object.entries(providerEntry)) {
        const exported = (models as Record<string, Record<string, string>>)[
          constName
        ];
        expect(exported, `missing export ${constName}`).toBeDefined();
        expect(
          { ...exported },
          `${providerSlug} ${constName} differs from const-key map`
        ).toEqual({ ...mapping });
      }
    }
  );
});
