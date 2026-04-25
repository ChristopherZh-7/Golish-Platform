import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Eye, Loader2, Play, Search,
} from "lucide-react";
import { SEV_TEXT } from "@/lib/severity";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import {
  zapGetHistory, zapGetHistoryCount, zapGetMessage,
} from "@/lib/pentest/zap-api";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import { StyledSelect } from "./shared";
// ── Passive Scan Panel ──

interface PassiveRule {
  id: string;
  name: string;
  enabled: boolean;
  quality: string;
}

interface CustomPassiveRule {
  id: string;
  name: string;
  pattern: string;
  scope: "body" | "headers" | "all";
  severity: "low" | "medium" | "high";
  enabled: boolean;
}

interface CustomRuleMatch {
  ruleId: string;
  ruleName: string;
  severity: string;
  msgId: number;
  url: string;
  matchSnippet: string;
}

function saveCustomRulesToDb(rules: CustomPassiveRule[], pp: string | null) {
  invoke("custom_rules_save_all", { rules, projectPath: pp }).catch(() => {});
}

export function PassiveScanPanel() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(true);
  const [records, setRecords] = useState(0);
  const [rules, setRules] = useState<PassiveRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [tab, setTab] = useState<"zap" | "custom">("zap");
  const projectPath = useStore((s) => s.currentProjectPath);
  const [customRules, setCustomRules] = useState<CustomPassiveRule[]>([]);
  const [editing, setEditing] = useState<CustomPassiveRule | null>(null);
  const [matches, setMatches] = useState<CustomRuleMatch[]>([]);
  const [scanning, setScanning] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLoading(true);
      try {
        type ZapJson = Record<string, unknown>;
        const [recordsResult, scannersResult, dbRules] = await Promise.all([
          invoke<ZapJson>("zap_api_call", { component: "pscan", actionType: "view", method: "recordsToScan", params: {} }).catch(() => ({})),
          invoke<ZapJson>("zap_api_call", { component: "pscan", actionType: "view", method: "scanners", params: {} }).catch(() => ({})),
          invoke<CustomPassiveRule[]>("custom_rules_list", { projectPath: getProjectPath() }).catch(() => []),
        ]);
        if (cancelled) return;
        const recordsVal = (recordsResult as Record<string, unknown>)?.recordsToScan;
        setRecords(typeof recordsVal === "string" ? Number.parseInt(recordsVal) || 0 : 0);
        const scanners = (scannersResult as Record<string, unknown>)?.scanners;
        if (Array.isArray(scanners)) {
          setRules(scanners.map((r: Record<string, string>) => ({
            id: r.id || "",
            name: r.name || "",
            enabled: r.enabled === "true",
            quality: r.quality || "",
          })));
          const anyEnabled = scanners.some((r: Record<string, string>) => r.enabled === "true");
          setEnabled(anyEnabled);
        }
        if (Array.isArray(dbRules) && dbRules.length > 0) setCustomRules(dbRules);
      } catch { /* ignore */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const r = await invoke<Record<string, unknown>>("zap_api_call", { component: "pscan", actionType: "view", method: "recordsToScan", params: {} });
        const val = r?.recordsToScan;
        setRecords(typeof val === "string" ? Number.parseInt(val) || 0 : 0);
      } catch { /* ignore */ }
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const handleToggleAll = useCallback(async (enable: boolean) => {
    try {
      const method = enable ? "enableAllScanners" : "disableAllScanners";
      const result = await invoke<Record<string, unknown>>("zap_api_call", { component: "pscan", actionType: "action", method, params: {} });
      if (result?.Result === "OK") {
        setRules((prev) => prev.map((r) => ({ ...r, enabled: enable })));
        setEnabled(enable);
      }
    } catch (err) {
      console.error("Failed to toggle all passive scanners:", err);
    }
  }, []);

  const handleToggleRule = useCallback(async (ruleId: string, enable: boolean) => {
    try {
      const method = enable ? "enableScanners" : "disableScanners";
      const result = await invoke<Record<string, unknown>>("zap_api_call", { component: "pscan", actionType: "action", method, params: { ids: ruleId } });
      if (result?.Result === "OK") {
        setRules((prev) => prev.map((r) => r.id === ruleId ? { ...r, enabled: enable } : r));
      }
    } catch (err) {
      console.error("Failed to toggle passive scanner:", err);
    }
  }, []);

  const handleSaveCustomRule = useCallback((rule: CustomPassiveRule) => {
    setCustomRules((prev) => {
      const existing = prev.findIndex((r) => r.id === rule.id);
      const next = existing >= 0 ? prev.map((r) => r.id === rule.id ? rule : r) : [...prev, rule];
      saveCustomRulesToDb(next, projectPath);
      return next;
    });
    invoke("custom_rules_upsert", { rule, projectPath }).catch(() => {});
    setEditing(null);
  }, [projectPath]);

  const handleDeleteCustomRule = useCallback((id: string) => {
    setCustomRules((prev) => {
      const next = prev.filter((r) => r.id !== id);
      saveCustomRulesToDb(next, projectPath);
      return next;
    });
    invoke("custom_rules_delete", { id }).catch(() => {});
    setMatches((prev) => prev.filter((m) => m.ruleId !== id));
  }, [projectPath]);

  const handleRunCustomScan = useCallback(async () => {
    const enabledRules = customRules.filter((r) => r.enabled);
    if (enabledRules.length === 0) return;
    setScanning(true);
    setMatches([]);
    const newMatches: CustomRuleMatch[] = [];
    try {
      const count = await zapGetHistoryCount();
      const batchSize = 50;
      for (let start = 0; start < count; start += batchSize) {
        const entries = await zapGetHistory(start, batchSize);
        for (const entry of entries) {
          try {
            const detail = await zapGetMessage(entry.id, entry.url);
            for (const rule of enabledRules) {
              const re = new RegExp(rule.pattern, "i");
              const targets: string[] = [];
              if (rule.scope === "body" || rule.scope === "all") targets.push(detail.response_body || "");
              if (rule.scope === "headers" || rule.scope === "all") targets.push(detail.response_headers || "");
              for (const text of targets) {
                const match = re.exec(text);
                if (match) {
                  const idx = match.index;
                  const snippet = text.substring(Math.max(0, idx - 30), idx + match[0].length + 30);
                  newMatches.push({
                    ruleId: rule.id, ruleName: rule.name, severity: rule.severity,
                    msgId: entry.id, url: entry.url, matchSnippet: snippet,
                  });
                  break;
                }
              }
            }
          } catch { /* skip individual messages */ }
        }
      }
    } catch { /* ignore */ }
    setMatches(newMatches);
    setScanning(false);
    if (newMatches.length > 0) {
      const items = newMatches.map((m) => ({
        title: m.ruleName,
        severity: m.severity,
        url: m.url,
        target: (() => { try { return new URL(m.url).host; } catch { return ""; } })(),
        description: `Pattern match: ${m.matchSnippet}`,
      }));
      invoke("findings_import_parsed", { items, toolName: "Custom Passive Scan", projectPath: getProjectPath() }).catch(() => {});
    }
  }, [customRules]);

  const filtered = useMemo(() => {
    if (!search.trim()) return rules;
    const q = search.toLowerCase();
    return rules.filter((r) => r.name.toLowerCase().includes(q) || r.id.includes(q));
  }, [rules, search]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/50" />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-4 py-3 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-3">
          <Eye className="w-3.5 h-3.5 text-accent" />
          <span className="text-[12px] font-medium text-foreground/80">{t("security.passiveScan")}</span>
          <span className={cn(
            "text-[9px] px-2 py-0.5 rounded-full font-medium",
            enabled ? "bg-green-500/15 text-green-400" : "bg-zinc-500/15 text-zinc-400"
          )}>
            {enabled ? t("security.passiveEnabled") : t("security.passiveDisabled")}
          </span>
          {records > 0 && (
            <span className="text-[10px] text-muted-foreground/40">
              {records} {t("security.passiveRecords")}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setTab("zap")}
            className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "zap" ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground")}
          >
            {t("security.passiveRules")}
          </button>
          <button
            type="button"
            onClick={() => setTab("custom")}
            className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "custom" ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground")}
          >
            {t("security.customRules")}
            {customRules.length > 0 && <span className="ml-1 text-[8px] text-muted-foreground/50">({customRules.length})</span>}
          </button>
        </div>
      </div>

      {tab === "zap" ? (
        <>
          <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => handleToggleAll(true)}
                className="px-2.5 py-1 rounded-md text-[10px] font-medium text-green-400 bg-green-500/10 hover:bg-green-500/20 transition-colors"
              >
                {t("security.enableAllPassive")}
              </button>
              <button
                type="button"
                onClick={() => handleToggleAll(false)}
                className="px-2.5 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 bg-muted/20 hover:bg-muted/30 transition-colors"
              >
                {t("security.disableAllPassive")}
              </button>
            </div>
            <div className="relative flex-1 max-w-sm">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("security.passiveRules")}
                className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
              />
            </div>
            <span className="text-[10px] text-muted-foreground/50">
              {filtered.length} / {rules.length}
            </span>
          </div>
          <div className="flex-1 overflow-y-auto">
            {filtered.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/40">
                <Eye className="w-12 h-12" />
                <p className="text-[13px] font-medium">{t("security.noPassiveRules")}</p>
              </div>
            ) : (
              <table className="w-full text-[11px]">
                <thead className="sticky top-0 bg-card z-10">
                  <tr className="text-muted-foreground/40 text-left">
                    <th className="px-3 py-1.5 font-medium w-[50px]" />
                    <th className="px-3 py-1.5 font-medium w-[80px]">{t("security.ruleId")}</th>
                    <th className="px-3 py-1.5 font-medium">{t("security.scannerName")}</th>
                    <th className="px-3 py-1.5 font-medium w-[80px]">{t("security.status")}</th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((rule) => (
                    <tr key={rule.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/30 transition-colors">
                      <td className="px-3 py-1.5">
                        <button
                          type="button"
                          onClick={() => handleToggleRule(rule.id, !rule.enabled)}
                          className={cn("w-7 h-4 rounded-full transition-colors flex items-center px-0.5", rule.enabled ? "bg-green-500/30" : "bg-muted/30")}
                        >
                          <div className={cn("w-3 h-3 rounded-full transition-all", rule.enabled ? "bg-green-400 ml-3" : "bg-muted-foreground/40 ml-0")} />
                        </button>
                      </td>
                      <td className="px-3 py-1.5 font-mono text-muted-foreground/40">{rule.id}</td>
                      <td className="px-3 py-1.5 text-foreground/70">{rule.name}</td>
                      <td className="px-3 py-1.5">
                        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", rule.enabled ? "text-green-400 bg-green-500/10" : "text-muted-foreground/40 bg-muted/20")}>
                          {rule.enabled ? "ON" : "OFF"}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </>
      ) : (
        <CustomRulesView
          rules={customRules}
          editing={editing}
          matches={matches}
          scanning={scanning}
          onEdit={setEditing}
          onSave={handleSaveCustomRule}
          onDelete={handleDeleteCustomRule}
          onScan={handleRunCustomScan}
        />
      )}
    </div>
  );
}

function CustomRulesView({
  rules, editing, matches, scanning,
  onEdit, onSave, onDelete, onScan,
}: {
  rules: CustomPassiveRule[];
  editing: CustomPassiveRule | null;
  matches: CustomRuleMatch[];
  scanning: boolean;
  onEdit: (rule: CustomPassiveRule | null) => void;
  onSave: (rule: CustomPassiveRule) => void;
  onDelete: (id: string) => void;
  onScan: () => void;
}) {
  const { t } = useTranslation();
  const [formName, setFormName] = useState("");
  const [formPattern, setFormPattern] = useState("");
  const [formScope, setFormScope] = useState<"body" | "headers" | "all">("all");
  const [formSeverity, setFormSeverity] = useState<"low" | "medium" | "high">("medium");

  useEffect(() => {
    if (editing) {
      setFormName(editing.name);
      setFormPattern(editing.pattern);
      setFormScope(editing.scope);
      setFormSeverity(editing.severity);
    }
  }, [editing]);

  const handleSubmit = () => {
    if (!formName.trim() || !formPattern.trim()) return;
    try { new RegExp(formPattern); } catch { return; }
    onSave({
      id: editing?.id || `custom-${Date.now()}`,
      name: formName.trim(),
      pattern: formPattern.trim(),
      scope: formScope,
      severity: formSeverity,
      enabled: editing?.enabled ?? true,
    });
    setFormName("");
    setFormPattern("");
    setFormScope("all");
    setFormSeverity("medium");
  };

  const handleNewRule = () => {
    setFormName("");
    setFormPattern("");
    setFormScope("all");
    setFormSeverity("medium");
    onEdit({ id: "", name: "", pattern: "", scope: "all", severity: "medium", enabled: true });
  };

  const sevColor = (s: string) => SEV_TEXT[s] || "text-blue-400";

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-2">
          <button type="button" onClick={handleNewRule} className="px-2.5 py-1 rounded-md text-[10px] font-medium text-accent bg-accent/10 hover:bg-accent/20 transition-colors">
            + {t("security.addRule")}
          </button>
          <button
            type="button"
            onClick={onScan}
            disabled={scanning || rules.filter((r) => r.enabled).length === 0}
            className="flex items-center gap-1 px-2.5 py-1 rounded-md text-[10px] font-medium text-green-400 bg-green-500/10 hover:bg-green-500/20 transition-colors disabled:opacity-30"
          >
            {scanning ? <Loader2 className="w-3 h-3 animate-spin" /> : <Play className="w-3 h-3" />}
            {t("security.runScan")}
          </button>
        </div>
        {matches.length > 0 && (
          <span className="text-[10px] text-yellow-400/60">
            {matches.length} {t("security.matchesFound")}
          </span>
        )}
      </div>

      {editing && (
        <div className="px-4 py-3 border-b border-border/10 flex-shrink-0 space-y-2 bg-[var(--bg-hover)]/20">
          <div className="flex items-center gap-2">
            <input
              value={formName}
              onChange={(e) => setFormName(e.target.value)}
              placeholder={t("security.ruleName")}
              className="flex-1 h-7 px-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <StyledSelect
              value={formSeverity}
              onChange={(v) => setFormSeverity(v as "low" | "medium" | "high")}
              options={[{ value: "low", label: "Low" }, { value: "medium", label: "Medium" }, { value: "high", label: "High" }]}
              className="h-7"
            />
            <StyledSelect
              value={formScope}
              onChange={(v) => setFormScope(v as "body" | "headers" | "all")}
              options={[{ value: "all", label: "Body + Headers" }, { value: "body", label: "Body Only" }, { value: "headers", label: "Headers Only" }]}
              className="h-7"
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              value={formPattern}
              onChange={(e) => setFormPattern(e.target.value)}
              placeholder={t("security.regexPattern")}
              className="flex-1 h-7 px-3 text-[11px] font-mono bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <button type="button" onClick={handleSubmit} className="px-3 py-1 rounded-md text-[10px] font-medium text-accent bg-accent/10 hover:bg-accent/20 transition-colors">
              {editing.id ? t("security.updateRule") : t("security.saveRule")}
            </button>
            <button type="button" onClick={() => onEdit(null)} className="px-3 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 hover:text-foreground transition-colors">
              {t("security.cancel")}
            </button>
          </div>
        </div>
      )}

      <div className="flex-1 overflow-y-auto">
        {rules.length === 0 && matches.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/40">
            <Eye className="w-12 h-12" />
            <p className="text-[13px] font-medium">{t("security.noCustomRules")}</p>
            <p className="text-[11px] text-muted-foreground/50 max-w-sm text-center">{t("security.customRulesHint")}</p>
          </div>
        ) : (
          <div className="divide-y divide-border/5">
            {rules.map((rule) => (
              <div key={rule.id} className="flex items-center gap-3 px-4 py-2 hover:bg-[var(--bg-hover)]/30 transition-colors group">
                <button
                  type="button"
                  onClick={() => {
                    const updated = { ...rule, enabled: !rule.enabled };
                    onSave(updated);
                  }}
                  className={cn("w-7 h-4 rounded-full transition-colors flex items-center px-0.5 flex-shrink-0", rule.enabled ? "bg-green-500/30" : "bg-muted/30")}
                >
                  <div className={cn("w-3 h-3 rounded-full transition-all", rule.enabled ? "bg-green-400 ml-3" : "bg-muted-foreground/40 ml-0")} />
                </button>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-foreground/70 truncate">{rule.name}</span>
                    <span className={cn("text-[9px] font-medium", sevColor(rule.severity))}>{rule.severity.toUpperCase()}</span>
                  </div>
                  <span className="text-[10px] font-mono text-muted-foreground/50 truncate block">{rule.pattern}</span>
                </div>
                <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
                  <button type="button" onClick={() => onEdit(rule)} className="px-1.5 py-0.5 rounded text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">Edit</button>
                  <button type="button" onClick={() => onDelete(rule.id)} className="px-1.5 py-0.5 rounded text-[9px] text-destructive/50 hover:text-destructive transition-colors">Del</button>
                </div>
              </div>
            ))}
            {matches.length > 0 && (
              <div className="px-4 py-2">
                <h4 className="text-[10px] font-medium text-foreground/60 mb-2">{t("security.matchesFound")} ({matches.length})</h4>
                <div className="space-y-1">
                  {matches.map((m, i) => (
                    <div key={`${m.ruleId}-${m.msgId}-${i}`} className="flex items-start gap-2 text-[10px] py-1">
                      <span className={cn("flex-shrink-0 font-medium", sevColor(m.severity))}>{m.severity.toUpperCase()}</span>
                      <span className="text-foreground/60 truncate flex-1">{m.url}</span>
                      <span className="text-muted-foreground/50 font-mono text-[9px] max-w-[200px] truncate flex-shrink-0">{m.matchSnippet}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// (Op Logs, Recon Data, JS Analysis panels removed — now live in TargetPanel)


