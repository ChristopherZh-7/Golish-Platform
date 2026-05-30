const NON_EXEC_NAMES = new Set([
  "license",
  "licence",
  "readme",
  "readme.md",
  "readme.txt",
  "changelog",
  "changelog.md",
  "contributing",
  "contributing.md",
  "authors",
  "notice",
  "code_of_conduct.md",
  "security.md",
  "makefile",
  "dockerfile",
  "docker-compose.yml",
  "cargo.toml",
  "cargo.lock",
  "package.json",
  "package-lock.json",
  "go.mod",
  "go.sum",
  "gemfile",
  "gemfile.lock",
  "requirements.txt",
  "setup.py",
  "setup.cfg",
  "pyproject.toml",
]);

const NON_EXEC_EXTS = new Set([
  "md",
  "txt",
  "rst",
  "html",
  "css",
  "json",
  "yaml",
  "yml",
  "toml",
  "xml",
  "csv",
  "log",
  "lock",
  "cfg",
  "ini",
  "conf",
  "png",
  "jpg",
  "jpeg",
  "gif",
  "svg",
  "ico",
  "pdf",
  "doc",
  "zip",
  "tar",
  "gz",
]);

/**
 * Filter a flat file list down to plausible tool executables by dropping
 * well-known docs / manifests / data files. Used as a fallback when
 * `findToolExecutables` returns nothing for an extracted release.
 */
export function filterCandidateExecutables(allFiles: string[]): string[] {
  return allFiles.filter((f) => {
    const base = f.split("/").pop()?.toLowerCase() || "";
    if (NON_EXEC_NAMES.has(base)) return false;
    const ext = base.includes(".") ? base.split(".").pop() || "" : "";
    return !(ext && NON_EXEC_EXTS.has(ext));
  });
}
