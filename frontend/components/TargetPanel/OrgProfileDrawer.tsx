/**
 * `OrgProfileDrawer` — slide-in editor for the org-as-asset-intel record.
 *
 * The backend migration `_organizations_profile_fields` added 18 new
 * columns to `organizations`. The first 10 land in this MVP drawer as
 * five tabs (basic / domains / network / scope / other); the remaining
 * 8 are schema-only for now and get their UI in a follow-up PR.
 *
 * Editing model:
 * - Drawer opens → fetches the latest row via `organization_get`
 * - All edits stay in local state; nothing hits the network until Save
 * - Save builds an `OrganizationProfilePatch` (only the keys the user
 *   touched) and calls `organization_update_profile`
 * - Backend validates CIDR / domain / ASN syntax and returns 400 on bad
 *   data; the message surfaces in the footer banner so the user can
 *   pinpoint which field offended
 * - On success the refreshed row replaces the form state and a 2s
 *   "saved" indicator flashes
 *
 * Field grouping is mirrored in the i18n key `organizations.profile.*`
 * so localizations can rename tabs without code changes.
 */

import { Building2, Loader2, Plus, Save, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Sheet, SheetContent } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { organizations as orgsApi } from "@/lib/api";
import type {
  Organization,
  OrganizationProfilePatch,
  OrgDomainEntry,
  OrgScopeRules,
} from "@/lib/api/organizations";
import { cn } from "@/lib/utils";

const TIER_OPTIONS = ["", "critical", "high", "medium", "low"] as const;

interface OrgProfileDrawerProps {
  orgId: string | null;
  orgName?: string;
  open: boolean;
  onClose: () => void;
  t: (key: string) => string;
}

/** Local form state shape (everything is a string / array of strings so
 * the controlled inputs stay simple; conversion to/from API JSON happens
 * at the boundary). */
interface FormState {
  aliases: string;
  industry: string;
  tier: string;
  credit_code: string;
  domains: OrgDomainEntry[];
  ip_ranges: string;
  asns: string;
  email_domains: string;
  scope_in: string;
  scope_out: string;
  scope_forbid_time: string;
  scope_forbid_paths: string;
  intel: string;
  notes: string;
}

function linesToArray(raw: string): string[] {
  return raw
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function arrayToLines(arr: string[] | undefined | null): string {
  if (!arr || arr.length === 0) return "";
  return arr.join("\n");
}

function orgToForm(org: Organization): FormState {
  return {
    aliases: arrayToLines(org.aliases),
    industry: org.industry ?? "",
    tier: org.tier ?? "",
    credit_code: org.credit_code ?? "",
    domains: Array.isArray(org.domains) ? org.domains : [],
    ip_ranges: arrayToLines(org.ip_ranges),
    asns: arrayToLines(org.asns),
    email_domains: arrayToLines(org.email_domains),
    scope_in: arrayToLines(org.scope_rules?.in),
    scope_out: arrayToLines(org.scope_rules?.out),
    scope_forbid_time: arrayToLines(org.scope_rules?.forbid_time),
    scope_forbid_paths: arrayToLines(org.scope_rules?.forbid_paths),
    intel: org.intel && Object.keys(org.intel).length > 0 ? JSON.stringify(org.intel, null, 2) : "",
    notes: org.notes ?? "",
  };
}

function emptyForm(): FormState {
  return {
    aliases: "",
    industry: "",
    tier: "",
    credit_code: "",
    domains: [],
    ip_ranges: "",
    asns: "",
    email_domains: "",
    scope_in: "",
    scope_out: "",
    scope_forbid_time: "",
    scope_forbid_paths: "",
    intel: "",
    notes: "",
  };
}

/**
 * Build the PATCH payload by sending **every** edited field. We don't
 * try to compute a diff against `original` because that would silently
 * preserve stale data when the user explicitly cleared a field (an
 * empty array IS a valid intent, distinct from "no change"). Instead,
 * the contract is: opening the drawer = "I'm going to redefine this
 * org's profile"; saving sends the full snapshot.
 *
 * Throws if `intel` doesn't parse as JSON so the caller can show the
 * error inline before any network call.
 */
function buildPatch(form: FormState): OrganizationProfilePatch {
  let intel: Record<string, unknown> = {};
  const intelTrim = form.intel.trim();
  if (intelTrim.length > 0) {
    const parsed = JSON.parse(intelTrim);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new Error("intel must be a JSON object");
    }
    intel = parsed as Record<string, unknown>;
  }

  const scope: OrgScopeRules = {
    in: linesToArray(form.scope_in),
    out: linesToArray(form.scope_out),
    forbid_time: linesToArray(form.scope_forbid_time),
    forbid_paths: linesToArray(form.scope_forbid_paths),
  };

  return {
    aliases: linesToArray(form.aliases),
    industry: form.industry.trim(),
    tier: form.tier,
    credit_code: form.credit_code.trim(),
    domains: form.domains
      .filter((d) => d.domain.trim().length > 0)
      .map((d) => ({
        domain: d.domain.trim(),
        wildcard: !!d.wildcard,
        note: (d.note ?? "").trim() || undefined,
      })),
    ip_ranges: linesToArray(form.ip_ranges),
    asns: linesToArray(form.asns),
    email_domains: linesToArray(form.email_domains),
    scope_rules: scope,
    intel,
    notes: form.notes,
  };
}

export function OrgProfileDrawer({ orgId, orgName, open, onClose, t }: OrgProfileDrawerProps) {
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(emptyForm);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState(false);
  const [activeTab, setActiveTab] = useState("basic");

  const update = useCallback(<K extends keyof FormState>(key: K, value: FormState[K]) => {
    setForm((prev) => ({ ...prev, [key]: value }));
  }, []);

  useEffect(() => {
    if (!open || !orgId) return;
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    setSaveError(null);
    setSavedFlash(false);
    setActiveTab("basic");
    orgsApi
      .getOrganization(orgId)
      .then((org) => {
        if (cancelled) return;
        setForm(orgToForm(org));
      })
      .catch((e) => {
        if (cancelled) return;
        setLoadError(String(e));
      })
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, orgId]);

  const intelInvalid = useMemo(() => {
    const trim = form.intel.trim();
    if (trim.length === 0) return false;
    try {
      const parsed = JSON.parse(trim);
      return typeof parsed !== "object" || parsed === null || Array.isArray(parsed);
    } catch {
      return true;
    }
  }, [form.intel]);

  const handleSave = useCallback(async () => {
    if (!orgId) return;
    setSaving(true);
    setSaveError(null);
    setSavedFlash(false);
    try {
      const patch = buildPatch(form);
      const fresh = await orgsApi.updateOrganizationProfile(orgId, patch);
      setForm(orgToForm(fresh));
      setSavedFlash(true);
      setTimeout(() => setSavedFlash(false), 2000);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setSaveError(t("organizations.profile.saveFailed").replace("{{msg}}", msg));
    } finally {
      setSaving(false);
    }
  }, [orgId, form, t]);

  const handleAddDomain = useCallback(() => {
    setForm((prev) => ({
      ...prev,
      domains: [...prev.domains, { domain: "", wildcard: false, note: "" }],
    }));
  }, []);

  const handleUpdateDomain = useCallback((idx: number, patch: Partial<OrgDomainEntry>) => {
    setForm((prev) => ({
      ...prev,
      domains: prev.domains.map((d, i) => (i === idx ? { ...d, ...patch } : d)),
    }));
  }, []);

  const handleRemoveDomain = useCallback((idx: number) => {
    setForm((prev) => ({
      ...prev,
      domains: prev.domains.filter((_, i) => i !== idx),
    }));
  }, []);

  const title = t("organizations.profile.title").replace("{{name}}", orgName ?? "");

  return (
    <Sheet open={open} onOpenChange={(o) => !o && onClose()}>
      <SheetContent
        side="right"
        className="!max-w-2xl sm:!max-w-2xl w-full p-0 flex flex-col gap-0"
      >
        <div className="px-5 py-4 border-b border-border/40 bg-muted/10 flex items-start gap-3">
          <Building2 className="w-5 h-5 text-accent mt-0.5 flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <h2 className="text-sm font-semibold text-foreground truncate">{title}</h2>
            <p className="text-[11px] text-muted-foreground/80 mt-0.5">
              {t("organizations.profile.subtitle")}
            </p>
          </div>
        </div>

        {loading ? (
          <div className="flex-1 flex items-center justify-center text-muted-foreground gap-2 text-xs">
            <Loader2 className="w-4 h-4 animate-spin" />
            {t("organizations.profile.loading")}
          </div>
        ) : loadError ? (
          <div className="m-4 px-3 py-2 rounded bg-red-500/10 text-[11px] text-red-400">
            {t("organizations.profile.loadFailed").replace("{{msg}}", loadError)}
          </div>
        ) : (
          <Tabs
            value={activeTab}
            onValueChange={setActiveTab}
            className="flex-1 flex flex-col min-h-0 px-4 pt-3"
          >
            <TabsList className="w-full grid grid-cols-5 mb-2">
              <TabsTrigger value="basic">{t("organizations.profile.tabBasic")}</TabsTrigger>
              <TabsTrigger value="domains">{t("organizations.profile.tabDomains")}</TabsTrigger>
              <TabsTrigger value="network">{t("organizations.profile.tabNetwork")}</TabsTrigger>
              <TabsTrigger value="scope">{t("organizations.profile.tabScope")}</TabsTrigger>
              <TabsTrigger value="other">{t("organizations.profile.tabOther")}</TabsTrigger>
            </TabsList>

            <div className="flex-1 overflow-y-auto pb-3 pr-1">
              <TabsContent value="basic" className="space-y-4">
                <Field
                  label={t("organizations.profile.fields.aliases")}
                  helper={t("organizations.profile.fields.aliasesHelper")}
                >
                  <textarea
                    className={textareaCls}
                    rows={3}
                    value={form.aliases}
                    onChange={(e) => update("aliases", e.target.value)}
                    placeholder={"中国平安\n平安\nPing An\nPA"}
                  />
                </Field>

                <Field label={t("organizations.profile.fields.industry")}>
                  <input
                    className={inputCls}
                    value={form.industry}
                    onChange={(e) => update("industry", e.target.value)}
                    placeholder={t("organizations.profile.fields.industryPlaceholder")}
                  />
                </Field>

                <Field label={t("organizations.profile.fields.tier")}>
                  <select
                    className={inputCls}
                    value={form.tier}
                    onChange={(e) => update("tier", e.target.value)}
                  >
                    {TIER_OPTIONS.map((opt) => (
                      <option key={opt || "_unset"} value={opt}>
                        {t(`organizations.profile.fields.tierOptions.${opt}`)}
                      </option>
                    ))}
                  </select>
                </Field>

                <Field
                  label={t("organizations.profile.fields.creditCode")}
                  helper={t("organizations.profile.fields.creditCodeHelper")}
                >
                  <input
                    className={inputCls}
                    value={form.credit_code}
                    onChange={(e) => update("credit_code", e.target.value)}
                    placeholder="91110000xxxxxxxxxx"
                  />
                </Field>
              </TabsContent>

              <TabsContent value="domains" className="space-y-3">
                <Field
                  label={t("organizations.profile.fields.domains")}
                  helper={t("organizations.profile.fields.domainsHelper")}
                >
                  <div className="space-y-2">
                    {form.domains.length === 0 && (
                      <p className="text-[11px] text-muted-foreground/60 italic">—</p>
                    )}
                    {form.domains.map((entry, idx) => (
                      <div key={`${idx}-${entry.domain}`} className="flex items-center gap-2">
                        <input
                          className={cn(inputCls, "flex-1")}
                          value={entry.domain}
                          placeholder={t("organizations.profile.fields.domainsValuePlaceholder")}
                          onChange={(e) => handleUpdateDomain(idx, { domain: e.target.value })}
                        />
                        <label className="flex items-center gap-1 text-[11px] text-muted-foreground select-none">
                          <input
                            type="checkbox"
                            checked={!!entry.wildcard}
                            onChange={(e) =>
                              handleUpdateDomain(idx, { wildcard: e.target.checked })
                            }
                          />
                          {t("organizations.profile.fields.domainsWildcardColumn")}
                        </label>
                        <input
                          className={cn(inputCls, "w-40")}
                          value={entry.note ?? ""}
                          placeholder={t("organizations.profile.fields.domainsNotePlaceholder")}
                          onChange={(e) => handleUpdateDomain(idx, { note: e.target.value })}
                        />
                        <button
                          type="button"
                          className="p-1.5 rounded hover:bg-red-500/10 text-muted-foreground hover:text-red-400 transition-colors"
                          onClick={() => handleRemoveDomain(idx)}
                          title={t("common.delete")}
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    ))}
                    <button
                      type="button"
                      className="flex items-center gap-1.5 px-2.5 py-1.5 text-[11px] rounded bg-accent/10 hover:bg-accent/20 text-accent transition-colors"
                      onClick={handleAddDomain}
                    >
                      <Plus className="w-3 h-3" />
                      {t("organizations.profile.fields.domainsAdd")}
                    </button>
                  </div>
                </Field>
              </TabsContent>

              <TabsContent value="network" className="space-y-4">
                <Field
                  label={t("organizations.profile.fields.ipRanges")}
                  helper={t("organizations.profile.fields.ipRangesHelper")}
                >
                  <textarea
                    className={cn(textareaCls, "font-mono")}
                    rows={4}
                    value={form.ip_ranges}
                    onChange={(e) => update("ip_ranges", e.target.value)}
                    placeholder={t("organizations.profile.fields.ipRangesPlaceholder")}
                  />
                </Field>

                <Field
                  label={t("organizations.profile.fields.asns")}
                  helper={t("organizations.profile.fields.asnsHelper")}
                >
                  <textarea
                    className={cn(textareaCls, "font-mono")}
                    rows={3}
                    value={form.asns}
                    onChange={(e) => update("asns", e.target.value)}
                    placeholder={t("organizations.profile.fields.asnsPlaceholder")}
                  />
                </Field>

                <Field
                  label={t("organizations.profile.fields.emailDomains")}
                  helper={t("organizations.profile.fields.emailDomainsHelper")}
                >
                  <textarea
                    className={cn(textareaCls, "font-mono")}
                    rows={3}
                    value={form.email_domains}
                    onChange={(e) => update("email_domains", e.target.value)}
                    placeholder={t("organizations.profile.fields.emailDomainsPlaceholder")}
                  />
                </Field>
              </TabsContent>

              <TabsContent value="scope" className="space-y-4">
                <Field
                  label={t("organizations.profile.fields.scopeIn")}
                  helper={t("organizations.profile.fields.scopeInHelper")}
                >
                  <textarea
                    className={cn(textareaCls, "font-mono")}
                    rows={3}
                    value={form.scope_in}
                    onChange={(e) => update("scope_in", e.target.value)}
                  />
                </Field>

                <Field
                  label={t("organizations.profile.fields.scopeOut")}
                  helper={t("organizations.profile.fields.scopeOutHelper")}
                >
                  <textarea
                    className={cn(textareaCls, "font-mono")}
                    rows={3}
                    value={form.scope_out}
                    onChange={(e) => update("scope_out", e.target.value)}
                  />
                </Field>

                <Field
                  label={t("organizations.profile.fields.scopeForbidTime")}
                  helper={t("organizations.profile.fields.scopeForbidTimeHelper")}
                >
                  <textarea
                    className={textareaCls}
                    rows={2}
                    value={form.scope_forbid_time}
                    onChange={(e) => update("scope_forbid_time", e.target.value)}
                  />
                </Field>

                <Field
                  label={t("organizations.profile.fields.scopeForbidPaths")}
                  helper={t("organizations.profile.fields.scopeForbidPathsHelper")}
                >
                  <textarea
                    className={cn(textareaCls, "font-mono")}
                    rows={2}
                    value={form.scope_forbid_paths}
                    onChange={(e) => update("scope_forbid_paths", e.target.value)}
                  />
                </Field>
              </TabsContent>

              <TabsContent value="other" className="space-y-4">
                <Field
                  label={t("organizations.profile.fields.intel")}
                  helper={t("organizations.profile.fields.intelHelper")}
                >
                  <textarea
                    className={cn(
                      textareaCls,
                      "font-mono",
                      intelInvalid && "border-red-500/60 focus:border-red-500"
                    )}
                    rows={6}
                    value={form.intel}
                    onChange={(e) => update("intel", e.target.value)}
                    placeholder='{"prior_breach": "2023-04", "tags": ["fintech", "bank"]}'
                  />
                  {intelInvalid && (
                    <p className="text-[10px] text-red-400 mt-1">
                      {t("organizations.profile.fields.intelInvalid")}
                    </p>
                  )}
                </Field>

                <Field label={t("organizations.profile.fields.notes")}>
                  <textarea
                    className={textareaCls}
                    rows={5}
                    value={form.notes}
                    onChange={(e) => update("notes", e.target.value)}
                    placeholder={t("organizations.profile.fields.notesPlaceholder")}
                  />
                </Field>
              </TabsContent>
            </div>
          </Tabs>
        )}

        <div className="px-4 py-3 border-t border-border/40 bg-muted/5 flex items-center gap-3">
          <div className="flex-1 min-w-0">
            {saveError && (
              <p className="text-[11px] text-red-400 truncate" title={saveError}>
                {saveError}
              </p>
            )}
            {savedFlash && !saveError && (
              <p className="text-[11px] text-green-400">{t("organizations.profile.saved")}</p>
            )}
          </div>
          <button
            type="button"
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded border border-border/50 text-muted-foreground hover:bg-muted/30 transition-colors"
            onClick={onClose}
          >
            <X className="w-3 h-3" />
            {t("organizations.profile.close")}
          </button>
          <button
            type="button"
            disabled={saving || loading || intelInvalid}
            className={cn(
              "flex items-center gap-1.5 px-3 py-1.5 text-xs rounded bg-accent text-accent-foreground hover:opacity-90 transition-opacity",
              (saving || loading || intelInvalid) && "opacity-50 cursor-not-allowed"
            )}
            onClick={handleSave}
          >
            {saving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
            {saving ? t("organizations.profile.saving") : t("organizations.profile.save")}
          </button>
        </div>
      </SheetContent>
    </Sheet>
  );
}

// ── presentational helpers ─────────────────────────────────────────────

const inputCls =
  "w-full text-xs bg-background border border-border/50 rounded px-2.5 py-1.5 outline-none focus:border-accent transition-colors";

const textareaCls = `${inputCls} resize-y leading-relaxed`;

function Field({
  label,
  helper,
  children,
}: {
  label: string;
  helper?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <label className="text-[11px] font-medium text-foreground/80 block">{label}</label>
      {children}
      {helper && <p className="text-[10px] text-muted-foreground/70">{helper}</p>}
    </div>
  );
}
