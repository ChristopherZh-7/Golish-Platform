import { describe, expect, it } from "vitest";
import type { GolishSettings } from "@/lib/settings";
import { buildProviderConfig } from "./providerConfig";

function vertexGeminiSettings(location: string | null): GolishSettings {
  return {
    ai: {
      model_overrides: {},
      vertex_gemini: {
        credentials_path: "/tmp/credentials.json",
        project_id: "project-id",
        location,
      },
    },
  } as GolishSettings;
}

describe("AIChatPanel provider config", () => {
  it("uses the shared Vertex Gemini region default while preserving an explicit region", () => {
    const fallback = buildProviderConfig(
      "vertex_gemini",
      "gemini-test",
      "/tmp/workspace",
      vertexGeminiSettings(null)
    );
    const explicit = buildProviderConfig(
      "vertex_gemini",
      "gemini-test",
      "/tmp/workspace",
      vertexGeminiSettings("asia-northeast1")
    );

    expect(fallback).toMatchObject({ location: "us-central1" });
    expect(explicit).toMatchObject({ location: "asia-northeast1" });
  });
});
