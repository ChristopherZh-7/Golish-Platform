const AUTO_INSTALL_METHODS = new Set(["github", "homebrew", "homebrew-cask", "gem", "pip"]);

export function isAutoInstallMethod(method?: string | null) {
  return !!method && AUTO_INSTALL_METHODS.has(method);
}

type InstallSubBlock = { method: string; source?: string } | null | undefined;

export interface InstallLikeShape {
  method: string;
  source: string;
  macos?: InstallSubBlock;
  linux?: InstallSubBlock;
  windows?: InstallSubBlock;
}

export function detectInstallPlatform(): "windows" | "macos" | "linux" {
  const ua = (navigator.userAgent || "").toLowerCase();
  const platformStr = (navigator.platform || "").toLowerCase();
  if (platformStr.includes("win") || ua.includes("windows")) return "windows";
  if (platformStr.includes("mac") || platformStr.includes("darwin") || ua.includes("mac"))
    return "macos";
  return "linux";
}

/**
 * Resolve the (method, source) tuple to use on the current platform.
 *
 * Falls back to the top-level `install.method` / `install.source` if the
 * platform-specific sub-block is missing. Returns `null` when the tool's
 * default method cannot be auto-installed on this platform (e.g.
 * Homebrew on Windows without a `windows` block) so the caller can show
 * an actionable error.
 */
export function resolveInstallForPlatform(
  install: InstallLikeShape | null | undefined
): { method: string; source: string } | null {
  if (!install) return null;
  const platform = detectInstallPlatform();
  let block: InstallSubBlock;
  if (platform === "windows") block = install.windows;
  else if (platform === "macos") block = install.macos;
  else block = install.linux;
  if (block?.method) {
    return {
      method: block.method,
      source: block.source && block.source.length > 0 ? block.source : install.source,
    };
  }
  if (platform === "windows") {
    const homebrewMethods = new Set(["homebrew", "homebrew-cask", "brew", "brew-cask"]);
    if (homebrewMethods.has(install.method)) {
      return null;
    }
  }
  return { method: install.method, source: install.source };
}
