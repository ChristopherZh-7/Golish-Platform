import type { ComponentProps } from "react";

export function extToLang(name: string): string | null {
  const ext = name.split(".").pop()?.toLowerCase();
  const map: Record<string, string> = {
    py: "python", sh: "bash", bash: "bash", zsh: "bash", go: "go", rs: "rust",
    rb: "ruby", pl: "perl", js: "javascript", ts: "typescript", jsx: "jsx", tsx: "tsx",
    c: "c", cpp: "cpp", h: "c", hpp: "cpp", java: "java", cs: "csharp",
    swift: "swift", kt: "kotlin", lua: "lua", r: "r", ps1: "powershell",
    bat: "batch", php: "php", html: "html", css: "css", xml: "xml",
    json: "json", yaml: "yaml", yml: "yaml", toml: "toml", ini: "ini",
    sql: "sql", graphql: "graphql", proto: "protobuf", nse: "lua",
    dockerfile: "dockerfile", makefile: "makefile",
  };
  return ext ? (map[ext] || null) : null;
}

export function isMarkdown(name: string): boolean {
  return name.endsWith(".md");
}

export const CATEGORY_META: Record<string, { icon: string; label: string; color: string }> = {
  products: { icon: "📦", label: "Products", color: "text-blue-400" },
  techniques: { icon: "⚔️", label: "Techniques", color: "text-red-400" },
  pocs: { icon: "🔧", label: "PoCs", color: "text-amber-400" },
  experience: { icon: "📝", label: "Experience", color: "text-green-400" },
  analysis: { icon: "🔬", label: "Analysis", color: "text-purple-400" },
  uncategorized: { icon: "📄", label: "Other", color: "text-muted-foreground" },
};

export const STATUS_COLORS: Record<string, string> = {
  draft: "bg-yellow-500",
  partial: "bg-orange-500",
  complete: "bg-green-500",
  "needs-poc": "bg-blue-500",
  verified: "bg-emerald-500",
};
