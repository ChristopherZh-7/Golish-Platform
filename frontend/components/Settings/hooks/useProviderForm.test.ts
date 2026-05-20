import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS, type AiSettings } from "@/lib/settings";
import { useProviderForm } from "./useProviderForm";

vi.mock("@/lib/model-registry", () => ({
  getProviders: vi.fn().mockResolvedValue([]),
}));

describe("useProviderForm", () => {
  it("creates a missing DeepSeek settings block when saving its API key", () => {
    const settings = {
      ...DEFAULT_SETTINGS.ai,
      deepseek: undefined,
    } as unknown as AiSettings;
    const onChange = vi.fn();

    const { result } = renderHook(() => useProviderForm(settings, onChange));

    act(() => {
      result.current.updateProvider("deepseek", "api_key", "sk-deepseek");
    });

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        deepseek: {
          api_key: "sk-deepseek",
          base_url: null,
          show_in_selector: true,
        },
      })
    );
  });
});
