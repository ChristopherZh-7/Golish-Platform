import { invoke } from "@/lib/api/client";

type ZapJson = Record<string, unknown>;

export interface WordlistOption {
  id: string;
  name: string;
  category: string;
  line_count: number;
}

export interface SensitiveResult {
  id: string;
  baseUrl: string;
  probePath: string;
  fullUrl: string;
  statusCode: number;
  contentLength: number;
  contentType: string;
  isConfirmed: boolean;
  aiVerdict: string | null;
  createdAt: number;
}

export interface SensitiveProgress {
  scanId: string;
  total: number;
  completed: number;
  hits: number;
  currentUrl: string;
  running: boolean;
  dirsFound: number;
}

export interface VaultEntrySafe {
  id: string;
  name: string;
  entry_type: string;
  username: string;
  notes: string;
  project: string;
  tags: string[];
  created_at: string;
}

export interface CustomPassiveRule {
  id: string;
  name: string;
  pattern: string;
  scope: "body" | "headers" | "all";
  severity: "low" | "medium" | "high";
  enabled: boolean;
}

export const securityApi = {
  // Runtime install
  installRuntime: (runtimeType: string, proxyUrl?: string) =>
    invoke("pentest_install_runtime", { runtimeType, proxyUrl }),

  // ZAP API
  zapApiCall: <T = ZapJson>(
    component: string,
    actionType: string,
    method: string,
    params: Record<string, unknown>
  ) => invoke<T>("zap_api_call", { component, actionType, method, params }),
  zapListScanPolicies: () => invoke<string[]>("zap_list_scan_policies"),
  zapGetScanners: () =>
    invoke<Array<{ id: string; name: string; enabled: boolean; quality: string }>>(
      "zap_get_scanners"
    ),
  zapSetScannersEnabled: (ids: string[], enabled: boolean) =>
    invoke("zap_set_scanners_enabled", { ids, enabled }),
  zapScanMessageCount: (scanId: string) => invoke<number>("zap_scan_message_count", { scanId }),

  // Vault
  vaultList: (projectPath: string | null) => invoke<VaultEntrySafe[]>("vault_list", { projectPath }),
  vaultGetValue: (id: string, projectPath: string | null) =>
    invoke<string>("vault_get_value", { id, projectPath }),

  // Passive scans
  passiveScansByUrl: (url: string, limit: number) => invoke("passive_scans_by_url", { url, limit }),
  customRulesList: (projectPath: string | null) =>
    invoke<CustomPassiveRule[]>("custom_rules_list", { projectPath }),
  customRulesUpsert: (rule: CustomPassiveRule, projectPath: string | null) =>
    invoke("custom_rules_upsert", { rule, projectPath }),
  customRulesDelete: (id: string) => invoke("custom_rules_delete", { id }),
  customRulesSaveAll: (rules: CustomPassiveRule[], projectPath: string | null) =>
    invoke("custom_rules_save_all", { rules, projectPath }),
  findingsImportParsed: (items: unknown[], toolName: string, projectPath: string | null) =>
    invoke("findings_import_parsed", { items, toolName, projectPath }),

  // Sensitive scan
  wordlistList: () => invoke<WordlistOption[]>("wordlist_list"),
  sensitiveScanResults: (projectPath: string | null, confirmedOnly: boolean) =>
    invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly }),
  sensitiveScanStatus: () => invoke<boolean>("sensitive_scan_status"),
  sensitiveScanStart: (
    projectPath: string | null,
    baseUrl: string,
    wordlistId: string,
    ratePerSecond: number,
    useSitemapDirs: boolean
  ) =>
    invoke("sensitive_scan_start", {
      projectPath,
      baseUrl,
      wordlistId,
      ratePerSecond,
      useSitemapDirs,
    }),
  sensitiveScanStop: () => invoke("sensitive_scan_stop"),
  sensitiveScanClear: (projectPath: string | null) => invoke("sensitive_scan_clear", { projectPath }),
  sensitiveScanConfirm: (ids: string[], confirmed: boolean) =>
    invoke("sensitive_scan_confirm", { ids, confirmed }),
  sensitiveScanAiAnalyze: (projectPath: string | null) =>
    invoke<{ analyzed: number; true_positives: number }>("sensitive_scan_ai_analyze", {
      projectPath,
    }),

  // Nuclei
  nucleiCancel: () => invoke("nuclei_cancel"),

  // Targets
  targetList: (projectPath: string | null) =>
    invoke<{ targets: Array<{ id: string; value: string; type: string; scope: string }> }>(
      "target_list",
      { projectPath }
    ),
};
