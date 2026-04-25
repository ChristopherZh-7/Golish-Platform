import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ChevronDown, ChevronRight, Code, Copy, ExternalLink, FileCode2,
  FileText, Loader2, Plus, Search, Trash2, Zap,
} from "lucide-react";
import { copyToClipboard } from "@/lib/clipboard";
import { cn } from "@/lib/utils";
import type { VulnLink, PocTemplate } from "./types";
import { CustomSelect } from "@/components/ui/custom-select";
import { useTranslation } from "react-i18next";

interface GithubPocResult {
  full_name: string;
  html_url: string;
  description: string | null;
  language: string | null;
  stars: number;
  updated_at: string;
  topics: string[];
}

interface NucleiTemplateResult {
  name: string;
  path: string;
  html_url: string;
  content: string | null;
  severity: string | null;
}

export function PocTab({ link, cveId, onUpdateLink }: { link: VulnLink; cveId: string; onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void }) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<PocTemplate | null>(null);
  const [formName, setFormName] = useState("");
  const [formType, setFormType] = useState<PocTemplate["type"]>("nuclei");
  const [formLang, setFormLang] = useState("yaml");
  const [formContent, setFormContent] = useState("");
  const [expandedPoc, setExpandedPoc] = useState<string | null>(null);
  const [ghResults, setGhResults] = useState<GithubPocResult[]>([]);
  const [ghSearching, setGhSearching] = useState(false);
  const [ghSearched, setGhSearched] = useState(false);
  const [ghError, setGhError] = useState<string | null>(null);

  const [nucleiResults, setNucleiResults] = useState<NucleiTemplateResult[]>([]);
  const [nucleiSearching, setNucleiSearching] = useState(false);
  const [nucleiSearched, setNucleiSearched] = useState(false);
  const [nucleiError, setNucleiError] = useState<string | null>(null);
  const [nucleiImporting, setNucleiImporting] = useState<string | null>(null);

  const searchGithubPoc = useCallback(async () => {
    setGhSearching(true);
    setGhError(null);
    try {
      const results = await invoke<GithubPocResult[]>("intel_search_github_poc", { cveId });
      setGhResults(results);
    } catch (e) {
      setGhError(String(e));
      setGhResults([]);
    }
    setGhSearching(false);
    setGhSearched(true);
  }, [cveId]);

  const searchNucleiTemplates = useCallback(async () => {
    setNucleiSearching(true);
    setNucleiError(null);
    try {
      const results = await invoke<NucleiTemplateResult[]>("intel_search_nuclei_templates", { cveId });
      setNucleiResults(results);
    } catch (e) {
      setNucleiError(String(e));
      setNucleiResults([]);
    }
    setNucleiSearching(false);
    setNucleiSearched(true);
  }, [cveId]);

  const importNucleiTemplate = useCallback(async (template: NucleiTemplateResult) => {
    if (!template.content) return;
    setNucleiImporting(template.name);
    try {
      const dbPoc = await invoke<PocTemplate>(
        "vuln_link_add_poc_full",
        {
          cveId, name: `[Nuclei] ${template.name}`, pocType: "nuclei", language: "yaml",
          content: template.content, source: "nuclei_template",
          sourceUrl: template.html_url, severity: template.severity ?? "unknown",
          description: "", tags: [],
        }
      );
      onUpdateLink((l) => ({
        ...l,
        pocTemplates: [...l.pocTemplates, {
          ...dbPoc,
          type: dbPoc.type as PocTemplate["type"],
        }],
      }));
    } catch (e) {
      console.error("Failed to import nuclei template:", e);
    }
    setNucleiImporting(null);
  }, [cveId, onUpdateLink]);

  const importAllNucleiTemplates = useCallback(async () => {
    const importable = nucleiResults.filter((t) => t.content);
    for (const template of importable) {
      await importNucleiTemplate(template);
    }
  }, [nucleiResults, importNucleiTemplate]);

  const generateTemplate = useCallback((type: PocTemplate["type"], lang = "python"): { content: string; language: string } => {
    const slug = cveId.toLowerCase().replace(/[^a-z0-9-]/g, "-");
    if (type === "nuclei") {
      return { language: "yaml", content: `id: ${slug}

info:
  name: ${cveId}
  author: golish
  severity: medium
  description: |
    Detection for ${cveId}

http:
  - method: GET
    path:
      - "{{BaseURL}}/"
    matchers:
      - type: status
        status:
          - 200
` };
    }
    if (type === "script") {
      const templates: Record<string, string> = {
        python: `#!/usr/bin/env python3
"""${cveId} PoC - Proof of Concept"""
import requests
import sys

def exploit(target: str):
    """Test target for ${cveId}"""
    url = f"{target.rstrip('/')}/"
    try:
        resp = requests.get(url, timeout=10, verify=False)
        if resp.status_code == 200:
            print(f"[+] {target} may be vulnerable to ${cveId}")
            return True
    except requests.RequestException as e:
        print(f"[-] Error: {e}")
    return False

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <target_url>")
        sys.exit(1)
    exploit(sys.argv[1])
`,
        bash: `#!/bin/bash
# ${cveId} PoC - Proof of Concept

TARGET="\${1:?Usage: $0 <target_url>}"

echo "[*] Testing $TARGET for ${cveId}..."

RESP=$(curl -sk -o /dev/null -w "%{http_code}" "$TARGET/")

if [ "$RESP" = "200" ]; then
    echo "[+] $TARGET may be vulnerable to ${cveId}"
else
    echo "[-] $TARGET does not appear vulnerable (HTTP $RESP)"
fi
`,
        go: `package main

import (
\t"fmt"
\t"net/http"
\t"os"
\t"time"
)

// ${cveId} PoC
func main() {
\tif len(os.Args) < 2 {
\t\tfmt.Fprintf(os.Stderr, "Usage: %s <target_url>\\n", os.Args[0])
\t\tos.Exit(1)
\t}
\ttarget := os.Args[1]
\tclient := &http.Client{Timeout: 10 * time.Second}
\tresp, err := client.Get(target + "/")
\tif err != nil {
\t\tfmt.Printf("[-] Error: %v\\n", err)
\t\tos.Exit(1)
\t}
\tdefer resp.Body.Close()
\tif resp.StatusCode == 200 {
\t\tfmt.Printf("[+] %s may be vulnerable to ${cveId}\\n", target)
\t} else {
\t\tfmt.Printf("[-] %s does not appear vulnerable (HTTP %d)\\n", target, resp.StatusCode)
\t}
}
`,
        javascript: `#!/usr/bin/env node
// ${cveId} PoC - Proof of Concept

const target = process.argv[2];
if (!target) {
  console.error(\`Usage: \${process.argv[1]} <target_url>\`);
  process.exit(1);
}

fetch(\`\${target.replace(/\\/$/, "")}/\`)
  .then((resp) => {
    if (resp.ok) {
      console.log(\`[+] \${target} may be vulnerable to ${cveId}\`);
    } else {
      console.log(\`[-] \${target} does not appear vulnerable (HTTP \${resp.status})\`);
    }
  })
  .catch((err) => console.error(\`[-] Error: \${err.message}\`));
`,
        c: `/* ${cveId} PoC - Proof of Concept */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <curl/curl.h>

static size_t discard_cb(void *ptr, size_t size, size_t nmemb, void *data) {
    (void)ptr; (void)data;
    return size * nmemb;
}

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <target_url>\\n", argv[0]);
        return 1;
    }

    CURL *curl = curl_easy_init();
    if (!curl) {
        fprintf(stderr, "[-] Failed to init curl\\n");
        return 1;
    }

    char url[2048];
    snprintf(url, sizeof(url), "%s/", argv[1]);

    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, discard_cb);
    curl_easy_setopt(curl, CURLOPT_TIMEOUT, 10L);
    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 0L);

    CURLcode res = curl_easy_perform(curl);
    if (res == CURLE_OK) {
        long code;
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &code);
        if (code == 200)
            printf("[+] %s may be vulnerable to ${cveId}\\n", argv[1]);
        else
            printf("[-] %s does not appear vulnerable (HTTP %ld)\\n", argv[1], code);
    } else {
        fprintf(stderr, "[-] Error: %s\\n", curl_easy_strerror(res));
    }

    curl_easy_cleanup(curl);
    return 0;
}
`,
      };
      return { language: lang, content: templates[lang] || templates.python };
    }
    return { language: "markdown", content: `# ${cveId} - Manual Testing\n\n## Steps to Reproduce\n\n1. Navigate to the target application\n2. ...\n\n## Expected Result\n\n...\n\n## Actual Result\n\n...\n\n## Impact\n\n...\n` };
  }, [cveId]);

  const handleNewPoc = useCallback(() => {
    const { content, language } = generateTemplate("nuclei");
    setFormName(`${cveId} PoC`);
    setFormType("nuclei");
    setFormLang(language);
    setFormContent(content);
    setEditing({ id: "", name: "", type: "nuclei", language, content: "", source: "manual", source_url: "", severity: "unknown", verified: false, description: "", tags: [], created: 0 });
  }, [cveId, generateTemplate]);

  const handleTypeChange = useCallback((newType: PocTemplate["type"]) => {
    setFormType(newType);
    const { content, language } = generateTemplate(newType, newType === "script" ? "python" : undefined);
    setFormLang(language);
    if (!editing?.id) {
      setFormContent(content);
    }
  }, [generateTemplate, editing]);

  const handleLangChange = useCallback((newLang: string) => {
    setFormLang(newLang);
    if (!editing?.id) {
      const { content } = generateTemplate("script", newLang);
      setFormContent(content);
    }
  }, [generateTemplate, editing]);

  const handleEditPoc = useCallback((poc: PocTemplate) => {
    setFormName(poc.name);
    setFormType(poc.type);
    setFormLang(poc.language);
    setFormContent(poc.content);
    setEditing(poc);
  }, []);

  const handleSavePoc = useCallback(() => {
    if (!formName.trim() || !formContent.trim()) return;
    const isNew = !editing;
    if (isNew) {
      invoke<PocTemplate>(
        "vuln_link_add_poc",
        { cveId, name: formName.trim(), pocType: formType, language: formLang, content: formContent }
      ).then((dbPoc) => {
        onUpdateLink((l) => ({
          ...l,
          pocTemplates: [...l.pocTemplates, {
            ...dbPoc,
            type: dbPoc.type as PocTemplate["type"],
          }],
        }));
      }).catch(console.error);
    } else {
      invoke("vuln_link_update_poc", { pocId: editing.id, name: formName.trim(), content: formContent }).catch(console.error);
      onUpdateLink((l) => ({
        ...l,
        pocTemplates: l.pocTemplates.map((p) =>
          p.id === editing.id ? { ...p, name: formName.trim(), content: formContent } : p
        ),
      }));
    }
    setEditing(null);
    setFormName("");
    setFormContent("");
  }, [editing, formName, formType, formLang, formContent, onUpdateLink, cveId]);

  const handleDeletePoc = useCallback((id: string) => {
    onUpdateLink((l) => ({ ...l, pocTemplates: l.pocTemplates.filter((p) => p.id !== id) }));
    invoke("vuln_link_remove_poc", { pocId: id }).catch(console.error);
  }, [onUpdateLink]);

  const handleCopyContent = useCallback((content: string) => {
    copyToClipboard(content);
  }, []);

  const typeIcon = (type: PocTemplate["type"]) => {
    if (type === "nuclei") return <Zap className="w-3 h-3 text-orange-400/60" />;
    if (type === "script") return <FileCode2 className="w-3 h-3 text-emerald-400/60" />;
    return <FileText className="w-3 h-3 text-blue-400/60" />;
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">
          {t("vulnIntel.pocTemplates", "PoC Templates")}
        </span>
        <button onClick={handleNewPoc}
          className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors">
          <Plus className="w-2.5 h-2.5" /> {t("vulnIntel.addPoc", "Add PoC")}
        </button>
      </div>

      {editing && (
        <div className="space-y-2 p-2.5 border border-border/15 rounded bg-[var(--bg-hover)]/20">
          <div className="flex items-center gap-2">
            <input
              value={formName}
              onChange={(e) => setFormName(e.target.value)}
              placeholder={t("vulnIntel.pocName", "PoC name...")}
              className="flex-1 h-6 px-2 text-[10px] bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <CustomSelect value={formType} onChange={(v) => handleTypeChange(v as PocTemplate["type"])}
              options={[
                { value: "nuclei", label: "Nuclei YAML" },
                { value: "script", label: "Script" },
                { value: "manual", label: "Manual" },
              ]}
              size="xs"
              className="min-w-[70px]"
            />
            {formType === "script" && (
              <CustomSelect value={formLang} onChange={handleLangChange}
                options={[
                  { value: "python", label: "Python" },
                  { value: "bash", label: "Bash" },
                  { value: "go", label: "Go" },
                  { value: "c", label: "C" },
                  { value: "javascript", label: "JS" },
                ]}
                size="xs"
                className="min-w-[60px]"
              />
            )}
          </div>
          <textarea
            value={formContent}
            onChange={(e) => setFormContent(e.target.value)}
            placeholder={t("vulnIntel.pocContentPlaceholder", "Paste or write your PoC template here...")}
            rows={12}
            className="w-full px-3 py-2 text-[10px] font-mono bg-[var(--bg-hover)]/30 rounded border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 resize-y leading-relaxed"
          />
          <div className="flex items-center gap-2">
            <button onClick={handleSavePoc} disabled={!formName.trim() || !formContent.trim()}
              className="px-3 py-1 rounded text-[9px] font-medium text-accent bg-accent/10 hover:bg-accent/20 transition-colors disabled:opacity-30">
              {editing.id ? t("vulnIntel.updatePoc", "Update") : t("vulnIntel.savePoc", "Save PoC")}
            </button>
            <button onClick={() => setEditing(null)}
              className="px-3 py-1 rounded text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">
              {t("common.cancel")}
            </button>
          </div>
        </div>
      )}

      {/* GitHub PoC Search */}
      <div className="border border-border/10 rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2">
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">GitHub PoC</span>
          <button
            onClick={searchGithubPoc}
            disabled={ghSearching}
            className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors disabled:opacity-30"
          >
            {ghSearching ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Search className="w-2.5 h-2.5" />}
            {ghSearched ? "Refresh" : "Search GitHub"}
          </button>
        </div>
        {ghError && (
          <div className="px-3 py-1.5 text-[9px] text-red-400/70 border-t border-border/5">{ghError}</div>
        )}
        {ghSearched && ghResults.length === 0 && !ghError && (
          <div className="px-3 py-2 text-[9px] text-muted-foreground/25 border-t border-border/5">
            No GitHub repositories found for {cveId}
          </div>
        )}
        {ghResults.length > 0 && (
          <div className="border-t border-border/5 max-h-48 overflow-y-auto">
            {ghResults.map((repo) => (
              <div key={repo.full_name} className="flex items-start gap-2 px-3 py-2 hover:bg-muted/5 transition-colors border-b border-border/3 last:border-b-0">
                <div className="flex-1 min-w-0">
                  <a href={repo.html_url} target="_blank" rel="noopener noreferrer"
                    className="text-[10px] text-accent/80 hover:text-accent transition-colors font-medium truncate block">
                    {repo.full_name}
                  </a>
                  {repo.description && (
                    <p className="text-[9px] text-muted-foreground/40 truncate mt-0.5">{repo.description}</p>
                  )}
                  <div className="flex items-center gap-2 mt-0.5">
                    {repo.language && <span className="text-[8px] text-muted-foreground/30">{repo.language}</span>}
                    <span className="text-[8px] text-yellow-400/50">★ {repo.stars}</span>
                    <span className="text-[8px] text-muted-foreground/20">{new Date(repo.updated_at).toLocaleDateString()}</span>
                  </div>
                </div>
                <a href={repo.html_url} target="_blank" rel="noopener noreferrer"
                  className="p-1 text-muted-foreground/25 hover:text-accent transition-colors flex-shrink-0">
                  <ExternalLink className="w-3 h-3" />
                </a>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Nuclei Template Search */}
      <div className="border border-border/10 rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-3 py-2">
          <span className="text-[8px] text-muted-foreground/30 uppercase tracking-wider">Nuclei Templates</span>
          <div className="flex items-center gap-2">
            {nucleiSearched && nucleiResults.filter((t) => t.content).length > 0 && (
              <button
                onClick={importAllNucleiTemplates}
                className="flex items-center gap-1 text-[9px] text-emerald-400/60 hover:text-emerald-400 transition-colors"
              >
                <Plus className="w-2.5 h-2.5" /> Import All
              </button>
            )}
            <button
              onClick={searchNucleiTemplates}
              disabled={nucleiSearching}
              className="flex items-center gap-1 text-[9px] text-accent/60 hover:text-accent transition-colors disabled:opacity-30"
            >
              {nucleiSearching ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : <Search className="w-2.5 h-2.5" />}
              {nucleiSearched ? "Refresh" : "Search"}
            </button>
          </div>
        </div>
        {nucleiError && (
          <div className="px-3 py-1.5 text-[9px] text-red-400/70 border-t border-border/5">{nucleiError}</div>
        )}
        {nucleiSearched && nucleiResults.length === 0 && !nucleiError && (
          <div className="px-3 py-2 text-[9px] text-muted-foreground/25 border-t border-border/5">
            No Nuclei templates found for {cveId}
          </div>
        )}
        {nucleiResults.length > 0 && (
          <div className="border-t border-border/5 max-h-56 overflow-y-auto">
            {nucleiResults.map((tmpl) => {
              const alreadyImported = link.pocTemplates.some((p) => p.name === `[Nuclei] ${tmpl.name}`);
              const severityColor = tmpl.severity === "critical" ? "text-red-400"
                : tmpl.severity === "high" ? "text-orange-400"
                : tmpl.severity === "medium" ? "text-yellow-400"
                : tmpl.severity === "low" ? "text-blue-400"
                : "text-muted-foreground/40";
              return (
                <div key={tmpl.path} className="flex items-start gap-2 px-3 py-2 hover:bg-muted/5 transition-colors border-b border-border/3 last:border-b-0">
                  <Zap className="w-3 h-3 text-orange-400/60 flex-shrink-0 mt-0.5" />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-[10px] text-foreground/70 font-medium truncate">{tmpl.name}</span>
                      {tmpl.severity && (
                        <span className={cn("text-[8px] px-1 py-0.5 rounded bg-muted/10 font-medium", severityColor)}>
                          {tmpl.severity}
                        </span>
                      )}
                    </div>
                    <p className="text-[9px] text-muted-foreground/35 truncate mt-0.5">{tmpl.path}</p>
                  </div>
                  <div className="flex items-center gap-1 flex-shrink-0">
                    {tmpl.content && !alreadyImported && (
                      <button
                        onClick={() => importNucleiTemplate(tmpl)}
                        disabled={nucleiImporting === tmpl.name}
                        className="px-1.5 py-0.5 rounded text-[8px] font-medium text-accent/70 bg-accent/10 hover:bg-accent/20 transition-colors disabled:opacity-30"
                      >
                        {nucleiImporting === tmpl.name ? <Loader2 className="w-2.5 h-2.5 animate-spin" /> : "Import"}
                      </button>
                    )}
                    {alreadyImported && (
                      <span className="text-[8px] text-emerald-400/50 px-1.5">Imported</span>
                    )}
                    <a href={tmpl.html_url} target="_blank" rel="noopener noreferrer"
                      className="p-1 text-muted-foreground/25 hover:text-accent transition-colors">
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {link.pocTemplates.length === 0 && !editing ? (
        <div className="flex flex-col items-center justify-center py-4 gap-2 text-muted-foreground/20">
          <Code className="w-8 h-8" />
          <p className="text-[10px]">{t("vulnIntel.noPoc", "No PoC templates")}</p>
          <p className="text-[9px] text-muted-foreground/15 max-w-xs text-center">{t("vulnIntel.pocHint", "Add Nuclei YAML templates, scripts, or manual testing notes for this vulnerability")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {link.pocTemplates.map((poc) => (
            <div key={poc.id} className="border border-border/10 rounded overflow-hidden">
              <div
                className="flex items-center gap-2 px-2 py-1.5 cursor-pointer hover:bg-muted/5 transition-colors group"
                onClick={() => setExpandedPoc(expandedPoc === poc.id ? null : poc.id)}
              >
                {typeIcon(poc.type)}
                <span className="text-[10px] text-foreground/70 truncate flex-1">{poc.name}</span>
                <span className="text-[8px] text-muted-foreground/25 px-1.5 py-0.5 bg-muted/10 rounded">{poc.type}</span>
                <span className="text-[8px] text-muted-foreground/20">{poc.language}</span>
                <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button onClick={(e) => { e.stopPropagation(); handleCopyContent(poc.content); }}
                    className="p-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors">
                    <Copy className="w-3 h-3" />
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); handleEditPoc(poc); }}
                    className="p-0.5 rounded text-muted-foreground/30 hover:text-accent transition-colors">
                    <FileCode2 className="w-3 h-3" />
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); handleDeletePoc(poc.id); }}
                    className="p-0.5 rounded text-muted-foreground/30 hover:text-destructive transition-colors">
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
                {expandedPoc === poc.id ? <ChevronDown className="w-3 h-3 text-muted-foreground/30" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/30" />}
              </div>
              {expandedPoc === poc.id && (
                <pre className="px-3 py-2 text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/20 border-t border-border/10 overflow-x-auto max-h-64 overflow-y-auto leading-relaxed">
                  {poc.content}
                </pre>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

