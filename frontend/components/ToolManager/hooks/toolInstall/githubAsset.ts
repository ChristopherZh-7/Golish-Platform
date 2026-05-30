interface GitHubAsset {
  name: string;
  browser_download_url: string;
}

/**
 * Pick the best release asset for the current platform/arch from a
 * GitHub release's asset list. Pure given `assets` plus the navigator
 * UA/platform; returns `null` when nothing suitable is found so the
 * caller can fall back to a git clone.
 */
export function selectGitHubAsset(assets: GitHubAsset[]): GitHubAsset | null {
  const ua = (navigator.userAgent || "").toLowerCase();
  const platformStr = (navigator.platform || "").toLowerCase();
  const isMac = platformStr.includes("mac") || platformStr.includes("darwin") || ua.includes("mac");
  const isWin = !isMac && (platformStr.includes("win") || ua.includes("windows"));
  const arch = ua.includes("arm64") || ua.includes("aarch64") ? "arm64" : "x64";
  const SKIP_EXTS = [".txt", ".md", ".sha256", ".sha512", ".asc", ".sig", ".pem"];
  const isSkippable = (name: string) =>
    SKIP_EXTS.some((e) => name.toLowerCase().endsWith(e)) ||
    /checksums?/i.test(name) ||
    /\.sbom\b/i.test(name);
  const archiveExts = [".zip", ".tar.gz", ".tgz", ".jar"];
  const winInstallerExts = [".exe", ".msi"];
  const isArchive = (name: string) => archiveExts.some((e) => name.toLowerCase().endsWith(e));
  const isWinInstaller = (name: string) =>
    winInstallerExts.some((e) => name.toLowerCase().endsWith(e));
  const archMatches = (n: string) => {
    if (arch === "arm64") {
      return n.includes("arm64") || n.includes("aarch64");
    }
    if (n.includes("arm64") || n.includes("aarch64")) return false;
    return n.includes("x86_64") || n.includes("x64") || n.includes("amd64") || n.includes("64");
  };
  const platformAssets = assets.filter((a) => {
    if (isSkippable(a.name)) return false;
    const n = a.name.toLowerCase();
    if (isMac)
      return n.includes("darwin") || n.includes("macos") || n.includes("mac") || n.includes("osx");
    if (isWin)
      return (
        n.includes("windows") ||
        n.includes("win64") ||
        n.includes("win32") ||
        n.includes("win-") ||
        n.endsWith(".exe") ||
        n.endsWith(".msi") ||
        /[-_.]win[-_.]/i.test(a.name)
      );
    return n.includes("linux");
  });
  const archScored = platformAssets.filter((a) => archMatches(a.name.toLowerCase()));
  const candidatePool = archScored.length > 0 ? archScored : platformAssets;
  return (
    (isWin && candidatePool.find((a) => isWinInstaller(a.name))) ||
    candidatePool.find((a) => isArchive(a.name)) ||
    candidatePool[0] ||
    platformAssets.find((a) => isArchive(a.name)) ||
    platformAssets[0] ||
    assets.find((a) => !isSkippable(a.name) && isArchive(a.name)) ||
    null
  );
}
