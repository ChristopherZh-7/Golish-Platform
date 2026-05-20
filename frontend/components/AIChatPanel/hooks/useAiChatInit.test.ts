import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getSettings } from "@/lib/settings";
import { useAiChatInit } from "./useAiChatInit";

vi.mock("@/lib/pentest/api", () => ({
  scanTools: vi.fn().mockResolvedValue({ success: true, tools: [] }),
}));

vi.mock("@/lib/settings", () => ({
  getSettings: vi.fn(),
}));

vi.mock("@/lib/terminal-restore", () => ({
  restoreBatchTerminals: vi.fn(),
}));

const getSettingsMock = vi.mocked(getSettings);

function settingsWithAi(aiOverrides: Record<string, unknown>) {
  return {
    ai: {
      anthropic: { api_key: null, show_in_selector: true },
      openai: { api_key: null, show_in_selector: true },
      openrouter: { api_key: null, show_in_selector: true },
      gemini: { api_key: null, show_in_selector: true },
      groq: { api_key: null, show_in_selector: true },
      xai: { api_key: null, show_in_selector: true },
      deepseek: { api_key: null, base_url: null, show_in_selector: true },
      zai_sdk: { api_key: null, base_url: null, show_in_selector: true },
      nvidia: { api_key: null, base_url: null, show_in_selector: true },
      vertex_ai: {
        credentials_path: null,
        project_id: null,
        location: null,
        show_in_selector: true,
      },
      vertex_gemini: {
        credentials_path: null,
        project_id: null,
        location: null,
        show_in_selector: true,
      },
      ollama: { base_url: "http://localhost:11434", show_in_selector: true },
      ...aiOverrides,
    },
  };
}

describe("useAiChatInit", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("includes DeepSeek in chat model picker providers when its API key is configured", async () => {
    getSettingsMock.mockResolvedValue(
      settingsWithAi({
        deepseek: {
          api_key: "sk-deepseek",
          base_url: null,
          show_in_selector: true,
        },
      }) as Awaited<ReturnType<typeof getSettings>>
    );

    const { result } = renderHook(() => useAiChatInit(vi.fn()));

    await waitFor(() => {
      expect(result.current.configuredProviders.has("deepseek")).toBe(true);
    });
  });

  it("hides DeepSeek when show_in_selector is disabled", async () => {
    getSettingsMock.mockResolvedValue(
      settingsWithAi({
        deepseek: {
          api_key: "sk-deepseek",
          base_url: null,
          show_in_selector: false,
        },
      }) as Awaited<ReturnType<typeof getSettings>>
    );

    const { result } = renderHook(() => useAiChatInit(vi.fn()));

    await waitFor(() => {
      expect(getSettingsMock).toHaveBeenCalled();
    });
    expect(result.current.configuredProviders.has("deepseek")).toBe(false);
  });
});
