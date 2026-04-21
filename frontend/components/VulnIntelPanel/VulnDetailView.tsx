import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle, BookOpen, Bot, Code, History, Loader2, MessageSquare, Shield, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { VulnEntry, VulnLink, DetailTab, DbVulnLinkFull } from "./types";
import { SEV_COLORS, SEV_DOT, dbToVulnLink } from "./types";
import { useStore } from "@/store";
import { initAiSession, buildProviderConfig, sendPromptSession, type AiProvider } from "@/lib/ai";
import { getSettings } from "@/lib/settings";
import { IntelTab } from "./IntelTab";
import { ResearchTab } from "./ResearchTab";
import { WikiTab } from "./WikiTab";
import { PocTab } from "./PocTab";
import { HistoryTab } from "./HistoryTab";

export function VulnDetailView({
  entry,
  link,
  detailTab,
  onTabChange,
  onUpdateLink,
}: {
  entry: VulnEntry;
  link: VulnLink;
  detailTab: DetailTab;
  onTabChange: (tab: DetailTab) => void;
  onUpdateLink: (updater: (link: VulnLink) => VulnLink) => void;
}) {
  const [ingesting, setIngesting] = useState(false);
  const [researchSessionId, setResearchSessionId] = useState<string | null>(null);
  const [researchError, setResearchError] = useState<string | null>(null);
  const [hasResearchHistory, setHasResearchHistory] = useState(false);

  // Check if DB has previous research for this CVE
  useEffect(() => {
    invoke<{ turns: unknown[]; status: string } | null>("kb_research_load", { cveId: entry.cve_id })
      .then((log) => {
        if (log?.turns && Array.isArray(log.turns) && log.turns.length > 0) {
          setHasResearchHistory(true);
        }
      })
      .catch(() => {});
  }, [entry.cve_id]);

  // Load fresh link data from DB when viewing this CVE
  useEffect(() => {
    invoke<DbVulnLinkFull>("vuln_link_get", { cveId: entry.cve_id })
      .then((dbLink) => {
        const link = dbToVulnLink(dbLink);
        if (link.wikiPaths.length > 0 || link.pocTemplates.length > 0 || link.scanHistory.length > 0) {
          onUpdateLink(() => link);
        }
      })
      .catch(() => {});
  }, [entry.cve_id, onUpdateLink]);

  const handleAiResearch = useCallback(async () => {
    setIngesting(true);
    setResearchError(null);
    try {
      const sessionId = `kb-research-${entry.cve_id}-${Date.now()}`;
      const state = useStore.getState();
      const parentSession = state.sessions[state.activeSessionId ?? ""];
      const workspace = parentSession?.workingDirectory || ".";

      state.addSession(
        {
          id: sessionId,
          logicalTerminalId: crypto.randomUUID(),
          name: `KB: ${entry.cve_id}`,
          workingDirectory: workspace,
          createdAt: new Date().toISOString(),
          mode: "agent",
          inputMode: "agent",
        },
        { isPaneSession: true }
      );

      const settings = await getSettings();
      const researchProvider = (settings.ai.research_provider ?? settings.ai.default_provider) as AiProvider;
      const researchModel = settings.ai.research_model ?? settings.ai.default_model;

      const config = await buildProviderConfig(settings, workspace, {
        provider: researchProvider,
        model: researchModel,
      });
      await initAiSession(sessionId, config);
      useStore.getState().setSessionAiConfig(sessionId, {
        provider: researchProvider,
        model: researchModel,
        status: "ready",
      });

      setResearchSessionId(sessionId);
      onTabChange("research");

      const product = entry.affected_products?.length > 0
        ? entry.affected_products.join(", ")
        : "unknown product";

      const slug = product.split(",")[0].trim().toLowerCase().replace(/\s+/g, "-");
      const prompt = `# Vulnerability Knowledge Base — Ingest Guide

## Source CVE
- **CVE**: ${entry.cve_id}
- **Title**: ${entry.title}
- **Severity**: ${entry.severity}${entry.cvss_score != null ? ` (CVSS ${entry.cvss_score})` : ""}
- **Affected**: ${product}
- **Description**: ${entry.description.slice(0, 500)}

## Wiki Architecture

The vulnerability wiki follows a Karpathy-style compounding knowledge model. You maintain TWO types of pages:

### Products (\`products/{product-slug}/\`)
One page per CVE, specific to the affected product. Contains vulnerability details, exploitation, PoC, detection.
- Path: \`products/${slug}/${entry.cve_id}.md\`

### Techniques (\`techniques/\`)
One page per attack technique. Shared across multiple CVEs. Contains methodology, variants, examples.
- Example: \`techniques/jndi-injection.md\`, \`techniques/deserialization.md\`, \`techniques/ssrf.md\`

## Writing Standards

Every wiki page MUST have:
- YAML frontmatter: \`title\`, \`category\`, \`tags\`, \`cves\`, \`status\`
- Rich content with clear sections (## headings)
- Cross-references to related pages (markdown links to other wiki paths)
- Citations to sources (NVD, advisories, blog posts)
- Status: \`draft\`, \`partial\`, \`complete\`, \`needs-poc\`, \`verified\`

## Ingest Workflow

### Step 1: Check existing knowledge
Use \`search_knowledge_base\` with query "${entry.cve_id}" to find existing pages.
- If the CVE product page exists and status is \`complete\`/\`verified\`, check if technique pages also exist. If everything is covered, report completion.
- If pages exist but are \`draft\`/\`partial\`/\`needs-poc\`, continue to enrich them.

### Step 2: Create the product page
Use \`ingest_cve\` with cve_id="${entry.cve_id}" and product="${slug}" to create the base product page (if it doesn't exist).

### Step 3: Research
Search the web for:
- Exploit details and attack chains
- PoC code or exploit scripts
- Technical advisories and patch details
- Related CVEs and attack techniques

### Step 4: Update the product page
Use \`write_knowledge\` with \`cve_id: "${entry.cve_id}"\` to update \`products/${slug}/${entry.cve_id}.md\` with:
- Detailed vulnerability analysis
- Exploitation method and attack chain
- PoC code (if publicly available)
- Detection signatures and mitigation
- References and citations
- Cross-references to technique pages

### Step 5: Create/update technique pages
Identify the core attack technique(s) used (e.g., JNDI injection, deserialization, SSRF).
For EACH technique:
1. \`search_knowledge_base\` to check if a technique page exists
2. If not, use \`write_knowledge\` with \`cve_id: "${entry.cve_id}"\` to create \`techniques/{technique-slug}.md\` with:
   - Technique overview and methodology
   - Common variants
   - List of CVEs that use this technique (including this one)
   - Detection and prevention strategies
3. If it exists, use \`read_knowledge\` then \`write_knowledge\` with \`cve_id: "${entry.cve_id}"\` to ADD this CVE to the existing technique page's CVE list and update any new information.
**CRITICAL**: Always pass the \`cve_id\` parameter when calling \`write_knowledge\` so the page is automatically linked to this CVE and appears in the Wiki tab.

### Step 6: Cross-reference check
Use \`search_knowledge_base\` to find related pages. Add "See Also" cross-references where appropriate.

### Step 7: Save PoC templates
If you found exploit code, detection templates, or testing scripts during research, save them using \`save_poc\`:
- Use \`cve_id: "${entry.cve_id}"\`
- For Nuclei YAML templates: \`poc_type: "nuclei"\`, \`language: "yaml"\`
- For exploit scripts: \`poc_type: "script"\`, \`language: "python"\` (or bash, go, etc.)
- For manual testing procedures: \`poc_type: "manual"\`, \`language: "markdown"\`
Save each distinct PoC or template as a separate entry.

### Step 8: Set status
Update the product page frontmatter \`status\`:
- \`complete\` if you found exploit details + PoC
- \`partial\` if missing key sections
- \`needs-poc\` if analysis is thorough but no public PoC exists

## IMPORTANT
- A single CVE ingest should typically create/update 2-5 wiki pages.
- Technique pages are SHARED — multiple CVEs reference the same technique page.
- Always check for existing content before creating new pages.
- Never overwrite existing content — merge and enrich.
- Always save found PoC/exploit code using \`save_poc\` so it appears in the PoC tab.`;

      await sendPromptSession(sessionId, prompt);

      const expectedPath = `products/${slug}/${entry.cve_id}.md`;
      onUpdateLink((l) => ({
        ...l,
        wikiPaths: l.wikiPaths.includes(expectedPath) ? l.wikiPaths : [...l.wikiPaths, expectedPath],
      }));
      invoke("vuln_link_add_wiki", { cveId: entry.cve_id, wikiPath: expectedPath }).catch(console.error);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("Failed to trigger AI research:", e);
      setResearchError(msg);
    } finally {
      setIngesting(false);
    }
  }, [entry, onTabChange]);

  return (
    <>
      {/* Detail header */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <span className={cn("w-2 h-2 rounded-full flex-shrink-0", SEV_DOT[entry.severity] || "bg-slate-500")} />
        <span className="text-[12px] font-mono font-semibold text-accent">{entry.cve_id}</span>
        <span className={cn("text-[9px] px-2 py-0.5 rounded-full border capitalize", SEV_COLORS[entry.severity] || SEV_COLORS.info)}>
          {entry.severity}
          {entry.cvss_score != null && ` ${entry.cvss_score}`}
        </span>
        <span className="text-[9px] text-muted-foreground/25">{entry.source}</span>
        <div className="ml-auto">
          <button
            onClick={handleAiResearch}
            disabled={ingesting}
            className="flex items-center gap-1 px-2.5 py-1 rounded text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors disabled:opacity-50"
            title="AI researches this CVE: searches web, writes wiki page with exploit details, PoCs, and analysis"
          >
            {ingesting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Bot className="w-3.5 h-3.5" />}
            AI Research
          </button>
        </div>
      </div>

      {researchError && (
        <div className="flex items-center gap-2 px-4 py-2 bg-red-500/10 border-b border-red-500/20">
          <AlertTriangle className="w-3.5 h-3.5 text-red-400 flex-shrink-0" />
          <span className="text-[10px] text-red-400 flex-1">{researchError}</span>
          <button onClick={() => setResearchError(null)} className="text-red-400/50 hover:text-red-400">
            <X className="w-3 h-3" />
          </button>
        </div>
      )}

      {/* Detail tabs */}
      <div className="flex items-center gap-0.5 px-3 py-1.5 border-b border-border/10 bg-muted/3 flex-shrink-0">
        {([
          { id: "intel" as const, icon: Shield, label: "Intel" },
          { id: "wiki" as const, icon: BookOpen, label: `Wiki${link.wikiPaths.length > 0 ? ` (${link.wikiPaths.length})` : ""}` },
          { id: "poc" as const, icon: Code, label: `PoC${link.pocTemplates.length > 0 ? ` (${link.pocTemplates.length})` : ""}` },
          { id: "history" as const, icon: History, label: `History${link.scanHistory.length > 0 ? ` (${link.scanHistory.length})` : ""}` },
          ...(researchSessionId || hasResearchHistory ? [{ id: "research" as const, icon: MessageSquare, label: "Research" }] : []),
        ]).map((tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            className={cn(
              "flex items-center gap-1 px-2.5 py-1 rounded text-[10px] transition-colors",
              detailTab === tab.id ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground"
            )}
          >
            <tab.icon className="w-3 h-3" />
            {tab.label}
          </button>
        ))}
      </div>

      {/* Detail content */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {detailTab === "intel" && <IntelTab entry={entry} />}
        {detailTab === "wiki" && <WikiTab link={link} cveId={entry.cve_id} onUpdateLink={onUpdateLink} />}
        {detailTab === "poc" && <PocTab link={link} cveId={entry.cve_id} onUpdateLink={onUpdateLink} />}
        {detailTab === "history" && <HistoryTab link={link} />}
        {detailTab === "research" && <ResearchTab sessionId={researchSessionId} cveId={entry.cve_id} />}
      </div>
    </>
  );
}

