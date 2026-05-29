import { Building2, Check, Loader2, Search, Shield, Sparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { assetIntel } from "@/lib/api";
import type { LookupCompanyMatch } from "@/lib/api/asset-intel";
import type { Organization, OrganizationProfilePatch } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";

export type EngagementMode = "customer_targets" | "discover_assets" | "profile_only";

interface NewEngagementDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialMode?: EngagementMode;
  onCreateOrganization: (params: {
    name: string;
    owner: string;
    description: string;
  }) => Promise<Organization>;
  onUpdateOrganizationProfile: (
    id: string,
    patch: OrganizationProfilePatch
  ) => Promise<Organization>;
  onBatchAddTargets: (values: string, organizationId: string, source: string) => Promise<Target[]>;
  onCreated: () => void | Promise<void>;
}

const MODE_OPTIONS: Array<{
  id: EngagementMode;
  title: string;
  subtitle: string;
  icon: React.ReactNode;
}> = [
  {
    id: "customer_targets",
    title: "Customer Targets",
    subtitle: "客户已提供目标清单",
    icon: <Shield className="w-4 h-4" />,
  },
  {
    id: "discover_assets",
    title: "Discover Assets",
    subtitle: "按单位名称发现资产",
    icon: <Search className="w-4 h-4" />,
  },
  {
    id: "profile_only",
    title: "Org Profile Only",
    subtitle: "只创建组织档案",
    icon: <Building2 className="w-4 h-4" />,
  },
];

const MODE_OPTION_STYLES: Record<EngagementMode, { active: string; inactive: string }> = {
  customer_targets: {
    active: "border-green-500/40 bg-green-500/10 text-green-100",
    inactive:
      "border-border/50 bg-muted/10 text-muted-foreground hover:border-green-500/25 hover:bg-green-500/5 hover:text-green-100",
  },
  discover_assets: {
    active: "border-blue-500/40 bg-blue-500/10 text-blue-100",
    inactive:
      "border-border/50 bg-muted/10 text-muted-foreground hover:border-blue-500/25 hover:bg-blue-500/5 hover:text-blue-100",
  },
  profile_only: {
    active: "border-border/60 bg-muted/25 text-foreground",
    inactive:
      "border-border/50 bg-muted/10 text-muted-foreground hover:bg-muted/20 hover:text-foreground",
  },
};

function parseTargets(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of raw.split(/[\n,]+/)) {
    const value = item.trim();
    if (!value || seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

export function NewEngagementDialog({
  open,
  onOpenChange,
  initialMode = "customer_targets",
  onCreateOrganization,
  onUpdateOrganizationProfile,
  onBatchAddTargets,
  onCreated,
}: NewEngagementDialogProps) {
  const [mode, setMode] = useState<EngagementMode>(initialMode);
  const [orgName, setOrgName] = useState("");
  const [owner, setOwner] = useState("");
  const [description, setDescription] = useState("");
  const [targetsRaw, setTargetsRaw] = useState("");
  const [minOwnership, setMinOwnership] = useState("");
  const [depth, setDepth] = useState("");
  const [includeBranches, setIncludeBranches] = useState(false);
  const [createCandidates, setCreateCandidates] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lookupRunning, setLookupRunning] = useState(false);
  const [lookupError, setLookupError] = useState<string | null>(null);
  const [lookupMatches, setLookupMatches] = useState<LookupCompanyMatch[] | null>(null);
  const [selectedMatch, setSelectedMatch] = useState<LookupCompanyMatch | null>(null);

  const parsedTargets = useMemo(() => parseTargets(targetsRaw), [targetsRaw]);
  const canSubmit =
    orgName.trim().length > 0 && (mode !== "customer_targets" || parsedTargets.length > 0);

  useEffect(() => {
    if (open) setMode(initialMode);
  }, [initialMode, open]);

  const reset = () => {
    setMode("customer_targets");
    setOrgName("");
    setOwner("");
    setDescription("");
    setTargetsRaw("");
    setMinOwnership("");
    setDepth("");
    setIncludeBranches(false);
    setCreateCandidates(true);
    setError(null);
    setLookupRunning(false);
    setLookupError(null);
    setLookupMatches(null);
    setSelectedMatch(null);
  };

  /**
   * Run a disambiguation lookup using whatever's currently in the
   * `orgName` field as the keyword. Hidden when no `enabled` lookup
   * descriptor is registered backend-side (`lookupCompany` will throw
   * a validation error in that case and we surface it inline).
   */
  const handleLookup = async () => {
    const keyword = orgName.trim();
    if (!keyword || lookupRunning) return;
    setLookupRunning(true);
    setLookupError(null);
    setLookupMatches(null);
    setSelectedMatch(null);
    try {
      const result = await assetIntel.lookupCompany({ keyword });
      setLookupMatches(result.matches);
      if (result.matches.length === 0) {
        // Per-provider statuses surface here so the user can see *why*
        // we got nothing (auth expired, no descriptor enabled, …).
        const summary = result.providerStatus
          .map((status) => `${status.providerId}: ${status.message}`)
          .join(" / ");
        setLookupError(summary || "No matches.");
      }
    } catch (e) {
      setLookupError(String(e));
    } finally {
      setLookupRunning(false);
    }
  };

  const handleSelectMatch = (match: LookupCompanyMatch) => {
    setSelectedMatch(match);
    setOrgName(match.name);
  };

  const handleClearMatch = () => {
    setSelectedMatch(null);
    setLookupMatches(null);
    setLookupError(null);
  };

  const handleSubmit = async () => {
    if (!canSubmit || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const org = await onCreateOrganization({
        name: orgName.trim(),
        owner: owner.trim(),
        description: description.trim(),
      });
      // Build the profile patch. The intel.engagement block is always
      // present; canonical fields (credit_code / industry) only land when
      // the user committed a selectedMatch via lookup — keeps random
      // user-typed orgs from picking up stray master record data.
      const patch: OrganizationProfilePatch = {
        intel: {
          ...(org.intel ?? {}),
          engagement: {
            mode,
            source: mode === "customer_targets" ? "customer_provided" : "manual",
            target_count: mode === "customer_targets" ? parsedTargets.length : 0,
            min_ownership_percent: mode === "discover_assets" ? minOwnership.trim() : undefined,
            depth: mode === "discover_assets" ? depth.trim() : undefined,
            include_branches: mode === "discover_assets" ? includeBranches : undefined,
            create_candidates: mode === "discover_assets" ? createCandidates : undefined,
            created_at: Date.now(),
            lookup_match: selectedMatch
              ? {
                  provider_id: selectedMatch.providerId,
                  name: selectedMatch.name,
                  credit_code: selectedMatch.creditCode ?? null,
                  industry: selectedMatch.industry ?? null,
                  legal_representative: selectedMatch.legalRepresentative ?? null,
                  address: selectedMatch.address ?? null,
                  registered_at: selectedMatch.registeredAt ?? null,
                }
              : undefined,
          },
        },
      };
      if (selectedMatch?.creditCode) {
        patch.credit_code = selectedMatch.creditCode;
      }
      if (selectedMatch?.industry) {
        patch.industry = selectedMatch.industry;
      }
      await onUpdateOrganizationProfile(org.id, patch);
      if (mode === "customer_targets") {
        await onBatchAddTargets(parsedTargets.join("\n"), org.id, "customer_provided");
      }
      await onCreated();
      reset();
      onOpenChange(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const submitLabel =
    mode === "customer_targets"
      ? "Create & Import"
      : mode === "discover_assets"
        ? "Create & Prepare Discovery"
        : "Create Org";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>New Engagement</DialogTitle>
          <DialogDescription>
            Create a customer organization first, then choose how targets enter scope.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <section className="space-y-2">
            <p className="text-[11px] font-medium text-muted-foreground">1. Choose workflow</p>
            <div className="grid grid-cols-3 gap-2">
              {MODE_OPTIONS.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => setMode(option.id)}
                  className={cn(
                    "text-left rounded-lg border px-3 py-2 transition-colors",
                    mode === option.id
                      ? MODE_OPTION_STYLES[option.id].active
                      : MODE_OPTION_STYLES[option.id].inactive
                  )}
                >
                  <span className="flex items-center gap-2 text-xs font-medium">
                    {option.icon}
                    {option.title}
                  </span>
                  <span className="mt-1 block text-[10px] opacity-75">{option.subtitle}</span>
                </button>
              ))}
            </div>
          </section>

          <section className="space-y-2">
            <p className="text-[11px] font-medium text-muted-foreground">2. Organization</p>
            <div className="grid grid-cols-[1fr_180px] gap-2">
              <div className="space-y-1">
                <label
                  htmlFor="engagement-org-name"
                  className="block text-[10px] text-muted-foreground"
                >
                  Organization name *
                </label>
                <div className="flex items-center gap-1">
                  <input
                    id="engagement-org-name"
                    aria-label="Organization name"
                    className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
                    placeholder="中国平安 / Acme Corp"
                    value={orgName}
                    onChange={(e) => {
                      setOrgName(e.target.value);
                      if (selectedMatch && e.target.value !== selectedMatch.name) {
                        setSelectedMatch(null);
                      }
                    }}
                  />
                  {mode === "discover_assets" && (
                    <button
                      type="button"
                      title="Look up company canonical name + credit code before hydrate"
                      aria-label="Look up company"
                      className={cn(
                        "inline-flex items-center gap-1 rounded border px-2 py-1 text-[10px]",
                        "border-blue-500/35 bg-blue-500/10 text-blue-200 hover:bg-blue-500/15 hover:text-blue-100",
                        (!orgName.trim() || lookupRunning) && "opacity-50 cursor-not-allowed"
                      )}
                      disabled={!orgName.trim() || lookupRunning}
                      onClick={() => void handleLookup()}
                    >
                      {lookupRunning ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        <Sparkles className="w-3 h-3" />
                      )}
                      Look up
                    </button>
                  )}
                </div>
              </div>
              <label className="space-y-1">
                <span className="text-[10px] text-muted-foreground">Owner</span>
                <input
                  className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
                  placeholder="Red Team"
                  value={owner}
                  onChange={(e) => setOwner(e.target.value)}
                />
              </label>
            </div>

            {selectedMatch && (
              <div className="flex items-start justify-between gap-2 rounded border border-emerald-500/30 bg-emerald-500/5 p-2 text-[10px] text-emerald-200">
                <div className="min-w-0 space-y-0.5">
                  <p className="truncate font-medium text-emerald-100">{selectedMatch.name}</p>
                  <p className="text-emerald-200/80">
                    {selectedMatch.creditCode
                      ? `Credit: ${selectedMatch.creditCode}`
                      : "no credit code"}
                    {selectedMatch.industry ? ` · ${selectedMatch.industry}` : ""}
                    {selectedMatch.legalRepresentative
                      ? ` · 法人 ${selectedMatch.legalRepresentative}`
                      : ""}
                  </p>
                  {selectedMatch.address && (
                    <p className="truncate text-emerald-200/70">{selectedMatch.address}</p>
                  )}
                </div>
                <button
                  type="button"
                  className="rounded border border-emerald-500/30 px-1.5 py-0.5 text-[9px] text-emerald-100/90 hover:bg-emerald-500/15"
                  onClick={handleClearMatch}
                >
                  Clear
                </button>
              </div>
            )}

            {lookupError && !lookupRunning && (
              <p className="rounded border border-amber-500/30 bg-amber-500/10 p-2 text-[10px] text-amber-200">
                {lookupError}
              </p>
            )}

            {!selectedMatch && lookupMatches && lookupMatches.length > 0 && (
              <div className="space-y-1 rounded border border-border/40 bg-muted/10 p-2">
                <p className="text-[10px] font-medium text-muted-foreground">
                  Pick the canonical company (top {lookupMatches.length})
                </p>
                {lookupMatches.map((match) => {
                  const matchKey =
                    match.creditCode?.trim() ||
                    `${match.providerId}:${match.name.trim().toLowerCase()}`;
                  return (
                    <button
                      key={matchKey}
                      type="button"
                      className="block w-full rounded border border-border/30 bg-background/50 p-2 text-left hover:border-accent/40 hover:bg-accent/5"
                      onClick={() => handleSelectMatch(match)}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <p className="truncate text-[11px] text-foreground">{match.name}</p>
                        <span className="text-[9px] text-muted-foreground">
                          {Math.round(match.confidence * 100)}%
                        </span>
                      </div>
                      <p className="mt-0.5 truncate text-[10px] text-muted-foreground">
                        {match.creditCode ? match.creditCode : "no credit code"}
                        {match.industry ? ` · ${match.industry}` : ""}
                        {match.legalRepresentative ? ` · 法人 ${match.legalRepresentative}` : ""}
                      </p>
                      {match.address && (
                        <p className="truncate text-[10px] text-muted-foreground/70">
                          {match.address}
                        </p>
                      )}
                    </button>
                  );
                })}
              </div>
            )}

            <textarea
              className="w-full min-h-16 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent resize-y"
              placeholder="Notes / engagement description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </section>

          {mode === "customer_targets" && (
            <section className="space-y-2">
              <p className="text-[11px] font-medium text-muted-foreground">3. Customer targets</p>
              <label className="space-y-1 block">
                <span className="text-[10px] text-muted-foreground">Targets *</span>
                <textarea
                  aria-label="Targets"
                  className="w-full min-h-28 font-mono text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent resize-y"
                  placeholder={"example.com\nhttps://portal.example.com\n1.2.3.4\n10.0.0.0/24"}
                  value={targetsRaw}
                  onChange={(e) => setTargetsRaw(e.target.value)}
                />
              </label>
              <p className="text-[10px] text-muted-foreground/70">
                Preview: {parsedTargets.length} unique target(s), linked to this organization.
              </p>
            </section>
          )}

          {mode === "discover_assets" && (
            <section className="space-y-3 rounded-lg border border-border/40 bg-muted/10 p-3">
              <p className="text-[11px] font-medium text-muted-foreground">3. Discovery setup</p>
              <div className="grid grid-cols-2 gap-2">
                <label className="space-y-1">
                  <span className="text-[10px] text-muted-foreground">Minimum ownership %</span>
                  <input
                    className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
                    value={minOwnership}
                    onChange={(e) => setMinOwnership(e.target.value)}
                  />
                </label>
                <label className="space-y-1">
                  <span className="text-[10px] text-muted-foreground">Depth</span>
                  <input
                    className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
                    value={depth}
                    onChange={(e) => setDepth(e.target.value)}
                  />
                </label>
              </div>
              <div className="flex flex-wrap gap-3 text-[11px] text-muted-foreground">
                <label className="inline-flex items-center gap-1.5">
                  <input
                    type="checkbox"
                    checked={includeBranches}
                    onChange={(e) => setIncludeBranches(e.target.checked)}
                  />
                  Include branches
                </label>
                <label className="inline-flex items-center gap-1.5">
                  <input
                    type="checkbox"
                    checked={createCandidates}
                    onChange={(e) => setCreateCandidates(e.target.checked)}
                  />
                  Create target candidates after review
                </label>
              </div>
              <p className="text-[10px] text-amber-400/80">
                This creates the organization and saves discovery settings. From the org workspace,
                run subsidiary discovery, enrich fields, then promote approved targets into scope.
              </p>
            </section>
          )}

          {mode === "profile_only" && (
            <section className="rounded-lg border border-border/40 bg-muted/10 p-3 text-[11px] text-muted-foreground">
              This creates only the customer organization. Add targets or hydrate asset intel later.
            </section>
          )}

          {error && <p className="text-[11px] text-red-400 bg-red-500/10 rounded p-2">{error}</p>}
        </div>

        <DialogFooter>
          <button
            type="button"
            className="px-3 py-1.5 text-xs rounded-md bg-muted/50 text-foreground/70 hover:bg-muted"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canSubmit || submitting}
            className={cn(
              "px-3 py-1.5 text-xs rounded-md inline-flex items-center gap-1.5",
              "border border-green-500/25 bg-green-500/15 text-green-100 hover:bg-green-500/20",
              (!canSubmit || submitting) && "opacity-50 cursor-not-allowed"
            )}
            onClick={handleSubmit}
          >
            {submitting ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <Check className="w-3 h-3" />
            )}
            {submitLabel}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
