import { describe, expect, it } from "vitest";
import { getModelItemClassName, getVisibleProviderGroups } from "./ChatModelSelector";

describe("getVisibleProviderGroups", () => {
  it("moves the currently selected provider to the top of the chat model menu", () => {
    const groups = getVisibleProviderGroups(new Set(["nvidia", "deepseek"]), "deepseek");

    expect(groups.map((g) => g.provider)).toEqual(["deepseek", "nvidia"]);
  });

  it("uses high-contrast text for the selected model item", () => {
    const className = getModelItemClassName(true);

    expect(className).toContain("text-foreground");
    expect(className).toContain("bg-accent/20");
  });
});
