/**
 * Organization intel field grouping + value formatting for the Target panel.
 *
 * Pure helpers extracted from `TargetGroupedView.tsx`. They turn a raw
 * organization record into UI-ready field groups (`getOrgFieldGroups`), format
 * scalar/array/intel values for display (`formatFieldValue`/`formatIntelValue`),
 * and provide the i18n fallback helpers the panel uses everywhere.
 */

export interface OrgFieldInput {
  aliases?: unknown[];
  industry?: string;
  tier?: string;
  credit_code?: string;
  domains?: unknown[];
  ip_ranges?: unknown[];
  asns?: unknown[];
  email_domains?: unknown[];
  scope_rules?: unknown;
  intel?: Record<string, unknown>;
  notes?: string;
  certificates?: unknown[];
  subsidiaries?: unknown[];
  business_systems?: unknown[];
  cloud_assets?: unknown[];
  github_orgs?: unknown[];
  social_accounts?: unknown[];
  historical_vulns?: unknown[];
  contacts?: unknown[];
}

export interface OrgFieldView {
  key: string;
  label: string;
  value: string;
  filled: boolean;
  raw?: unknown;
}

export interface OrgFieldGroup {
  key?: string;
  title: string;
  fields: OrgFieldView[];
}

function displayAtom(value: unknown): string {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    return String(record.domain ?? record.name ?? record.value ?? record.id ?? "").trim();
  }
  return String(value ?? "").trim();
}

const INTEL_FIELD_LABELS: Record<string, string> = {
  legal_representative: "Legal",
  registered_address: "Address",
  registered_at: "Registered",
  registered_capital: "Capital",
  business_status: "Status",
  business_scope: "Scope",
  aqc_pid: "AQC PID",
  icp_records: "ICP",
  asn_org: "ASN org",
  cname: "CNAME",
  operator: "Operator",
  cms: "CMS",
  component: "Component",
  server_name: "Server",
  server_version: "Server version",
  service: "Service",
  protocol: "Protocol",
  exposed_emails: "Exposed emails",
  email_leakage_total: "Pwned hits",
  code_leaks: "Code leaks",
  mail_mx: "MX records",
  mobile_apps: "Mobile apps",
  mini_programs: "Mini programs",
  app_domains: "App domains",
};

const INTEL_DISPLAY_ORDER = [
  "legal_representative",
  "registered_capital",
  "business_status",
  "registered_at",
  "registered_address",
  "business_scope",
  "icp_records",
  "operator",
  "cms",
  "component",
  "server_name",
  "service",
  "asn_org",
];

const LEAKAGE_INTEL_KEYS = ["exposed_emails", "email_leakage_total", "code_leaks"] as const;

const DNS_INTEL_KEYS = ["mail_mx"] as const;

const APP_INTEL_KEYS = ["mobile_apps", "mini_programs", "app_domains"] as const;

function intelGet(org: OrgFieldInput, key: string): unknown {
  const intel = org.intel;
  if (!intel || typeof intel !== "object" || Array.isArray(intel)) return undefined;
  return (intel as Record<string, unknown>)[key];
}

function formatIntelValue(value: unknown): string {
  if (!value || typeof value !== "object" || Array.isArray(value)) return formatFieldValue(value);
  const record = value as Record<string, unknown>;
  const keys = INTEL_DISPLAY_ORDER.filter((key) => displayAtom(record[key]));
  if (keys.length === 0) return "—";
  const shown = keys.slice(0, 3).map((key) => {
    const raw = record[key];
    const text = Array.isArray(raw) ? formatFieldValue(raw) : displayAtom(raw);
    return `${INTEL_FIELD_LABELS[key] ?? key}: ${text}`;
  });
  const rest = keys.length - shown.length;
  return rest > 0 ? `${shown.join(", ")} +${rest}` : shown.join(", ");
}

export type OrgFieldDisplayKind = "atom" | "chips" | "records";

export function getOrgFieldDisplayKind(fieldView: OrgFieldView): OrgFieldDisplayKind {
  if (
    fieldView.key === "intel" &&
    fieldView.raw &&
    typeof fieldView.raw === "object" &&
    !Array.isArray(fieldView.raw)
  ) {
    return "records";
  }
  if (Array.isArray(fieldView.raw) && fieldView.raw.length > 0) {
    return "chips";
  }
  return "atom";
}

export function getOrgFieldChips(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  return raw.map(displayAtom).filter((value) => value.length > 0);
}

const INTEL_RECORD_LABELS: Record<string, string> = {
  legal_representative: "Legal representative",
  registered_capital: "Registered capital",
  business_status: "Business status",
  registered_at: "Registered at",
  registered_address: "Registered address",
  business_scope: "Business scope",
  icp_records: "ICP records",
  aqc_pid: "AQC PID",
  asn_org: "ASN org",
  cname: "CNAME",
  operator: "Operator",
  cms: "CMS",
  component: "Component",
  server_name: "Server",
  server_version: "Server version",
  service: "Service",
  protocol: "Protocol",
  exposed_emails: "Exposed emails",
  email_leakage_total: "Pwned hits (HIBP-style)",
  code_leaks: "Code leaks (URLs)",
  mail_mx: "MX records",
  mobile_apps: "Mobile apps",
  mini_programs: "Mini programs",
  app_domains: "App domains",
};

export function getOrgFieldIntelRecords(
  raw: unknown
): { key: string; label: string; value: string }[] {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return [];
  const record = raw as Record<string, unknown>;
  return INTEL_DISPLAY_ORDER.filter((key) => displayAtom(record[key])).map((key) => {
    const value = record[key];
    const formatted = Array.isArray(value)
      ? value.map(displayAtom).filter(Boolean).join(", ")
      : displayAtom(value);
    return {
      key,
      label: INTEL_RECORD_LABELS[key] ?? key,
      value: formatted,
    };
  });
}

export const ORG_FIELD_CHIP_INLINE_LIMIT = 24;

export function formatFieldValue(value: unknown): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return "—";
    const samples = value.map(displayAtom).filter(Boolean);
    if (samples.length === 0) return `${value.length} item(s)`;
    const shown = samples.slice(0, 3).join(", ");
    const rest = samples.length - 3;
    return rest > 0 ? `${shown} +${rest}` : shown;
  }
  if (value && typeof value === "object") {
    return Object.keys(value as Record<string, unknown>).length > 0 ? "configured" : "—";
  }
  const text = String(value ?? "").trim();
  return text || "—";
}

function field(key: string, label: string, value: unknown): OrgFieldView {
  const formatted = key === "intel" ? formatIntelValue(value) : formatFieldValue(value);
  return { key, label, value: formatted, filled: formatted !== "—", raw: value };
}

export function translateWithFallback(
  t: (key: string) => string,
  key: string,
  fallback: string
): string {
  const translated = t(key);
  return translated === key ? fallback : translated;
}

export function translateOrgFieldGroups(
  groups: OrgFieldGroup[],
  t: (key: string) => string
): OrgFieldGroup[] {
  return groups.map((group) => ({
    ...group,
    title: group.key
      ? translateWithFallback(t, `targetWorkspace.fieldGroups.${group.key}`, group.title)
      : group.title,
    fields: group.fields.map((item) => ({
      ...item,
      label: translateWithFallback(t, `targetWorkspace.fields.${item.key}`, item.label),
    })),
  }));
}

export function getOrgFieldGroups(org: OrgFieldInput): OrgFieldGroup[] {
  return [
    {
      key: "basic",
      title: "Basic",
      fields: [
        field("aliases", "Aliases", org.aliases),
        field("industry", "Industry", org.industry),
        field("tier", "Priority", org.tier),
        field("credit_code", "Unified social credit code", org.credit_code),
      ],
    },
    {
      key: "apps",
      title: "Apps & Mini Programs",
      fields: APP_INTEL_KEYS.map((key) =>
        field(key, INTEL_RECORD_LABELS[key] ?? key, intelGet(org, key))
      ),
    },
    {
      key: "domains",
      title: "Domains",
      fields: [field("domains", "Domains", org.domains)],
    },
    {
      key: "network",
      title: "Network",
      fields: [
        field("ip_ranges", "IP ranges", org.ip_ranges),
        field("asns", "ASNs", org.asns),
        field("email_domains", "Email domains", org.email_domains),
      ],
    },
    {
      key: "scope",
      title: "Scope",
      fields: [field("scope_rules", "Scope rules", org.scope_rules)],
    },
    {
      key: "identity",
      title: "Identity",
      fields: [
        field("intel", "Intel records", org.intel),
        field("subsidiaries", "Subsidiaries", org.subsidiaries),
      ],
    },
    {
      key: "surfaces",
      title: "Surfaces",
      fields: [
        field("business_systems", "Business systems", org.business_systems),
        field("cloud_assets", "Cloud assets", org.cloud_assets),
        field("github_orgs", "GitHub orgs", org.github_orgs),
        field("social_accounts", "Social accounts", org.social_accounts),
      ],
    },
    {
      key: "leakage",
      title: "Leakage Intel",
      fields: LEAKAGE_INTEL_KEYS.map((key) =>
        field(key, INTEL_RECORD_LABELS[key] ?? key, intelGet(org, key))
      ),
    },
    {
      key: "dns",
      title: "DNS",
      fields: DNS_INTEL_KEYS.map((key) =>
        field(key, INTEL_RECORD_LABELS[key] ?? key, intelGet(org, key))
      ),
    },
    {
      key: "risk",
      title: "Risk & Notes",
      fields: [
        field("certificates", "Certificates", org.certificates),
        field("historical_vulns", "Historical vulns", org.historical_vulns),
        field("contacts", "Contacts", org.contacts),
        field("notes", "Notes", org.notes),
      ],
    },
  ];
}
