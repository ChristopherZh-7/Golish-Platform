import { Building2, Check, Loader2, Search, Shield } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
  const [minOwnership, setMinOwnership] = useState("51");
  const [depth, setDepth] = useState("2");
  const [includeBranches, setIncludeBranches] = useState(true);
  const [createCandidates, setCreateCandidates] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    setMinOwnership("51");
    setDepth("2");
    setIncludeBranches(true);
    setCreateCandidates(true);
    setError(null);
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
      await onUpdateOrganizationProfile(org.id, {
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
          },
        },
      });
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
                      ? "border-accent bg-accent/10 text-accent"
                      : "border-border/50 bg-muted/10 text-muted-foreground hover:text-foreground"
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
              <label className="space-y-1">
                <span className="text-[10px] text-muted-foreground">Organization name *</span>
                <input
                  aria-label="Organization name"
                  className="w-full text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
                  placeholder="中国平安 / Acme Corp"
                  value={orgName}
                  onChange={(e) => setOrgName(e.target.value)}
                />
              </label>
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
                Discovery orchestration is not wired yet; this creates the org and preserves the UI
                path for the next backend phase.
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
              "bg-accent text-accent-foreground hover:bg-accent/90",
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
