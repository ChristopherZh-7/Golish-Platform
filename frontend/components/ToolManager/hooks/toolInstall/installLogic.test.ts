import { describe, expect, it } from "vitest";
import { filterCandidateExecutables } from "./executableFilter";
import { selectGitHubAsset } from "./githubAsset";
import { isAutoInstallMethod } from "./installPlatform";

describe("isAutoInstallMethod", () => {
  it("accepts known auto-install methods", () => {
    expect(isAutoInstallMethod("github")).toBe(true);
    expect(isAutoInstallMethod("pip")).toBe(true);
    expect(isAutoInstallMethod("homebrew-cask")).toBe(true);
  });

  it("rejects unknown methods and nullish input", () => {
    expect(isAutoInstallMethod("apt")).toBe(false);
    expect(isAutoInstallMethod(null)).toBe(false);
    expect(isAutoInstallMethod(undefined)).toBe(false);
  });
});

describe("filterCandidateExecutables", () => {
  it("drops docs / manifests / data files but keeps real binaries", () => {
    const files = [
      "nuclei",
      "bin/scanner",
      "tool.bin",
      "README.md",
      "LICENSE",
      "package.json",
      "release.zip",
    ];
    expect(filterCandidateExecutables(files)).toEqual(["nuclei", "bin/scanner", "tool.bin"]);
  });
});

describe("selectGitHubAsset", () => {
  it("returns null for an empty or skippable-only asset list", () => {
    expect(selectGitHubAsset([])).toBeNull();
    expect(
      selectGitHubAsset([{ name: "NOTES.txt", browser_download_url: "u" }])
    ).toBeNull();
  });

  it("falls back to a non-skippable archive regardless of platform tag", () => {
    const picked = selectGitHubAsset([
      { name: "checksums.txt", browser_download_url: "u1" },
      { name: "tool.zip", browser_download_url: "u2" },
    ]);
    expect(picked?.name).toBe("tool.zip");
  });
});
