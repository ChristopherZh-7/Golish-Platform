import { ChevronRight, ExternalLink } from "lucide-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { AiSettings, OpenRouterProviderPreferences } from "@/lib/settings";

export function OpenRouterProviderPreferencesSection({
  settings,
  updatePref,
}: {
  settings: AiSettings;
  updatePref: <K extends keyof OpenRouterProviderPreferences>(
    field: K,
    value: OpenRouterProviderPreferences[K]
  ) => void;
}) {
  const prefs = settings.openrouter.provider_preferences;

  const toArray = (val: string): string[] | null => {
    const arr = val
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    return arr.length > 0 ? arr : null;
  };

  const fromArray = (arr?: string[] | null): string => (arr || []).join(", ");

  const hasPrefs = !!(prefs && Object.values(prefs).some((v) => v != null));

  return (
    <Collapsible defaultOpen={hasPrefs}>
      <CollapsibleTrigger className="flex w-full items-center gap-2 text-[11px] font-medium text-muted-foreground/60 hover:text-foreground/80 transition-colors py-1">
        <ChevronRight className="h-3 w-3 transition-transform duration-200 [[data-state=open]>&]:rotate-90" />
        Provider Routing Preferences
        {hasPrefs && (
          <span className="ml-auto text-[9px] font-semibold uppercase tracking-wider px-1.5 py-0.5 rounded-full bg-accent/10 text-accent/80">
            Active
          </span>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent className="pt-3 space-y-3">
        <p className="text-[11px] text-muted-foreground/50 leading-relaxed">
          Control which providers handle your requests.{" "}
          <a
            href="https://openrouter.ai/docs/guides/routing/provider-selection"
            target="_blank"
            rel="noopener noreferrer"
            className="text-accent/60 hover:text-accent transition-colors inline-flex items-center gap-0.5"
          >
            Docs <ExternalLink className="w-2.5 h-2.5" />
          </a>
        </p>

        <div className="space-y-1.5">
          <label htmlFor="or-order" className="text-[11px] font-medium text-foreground/70">
            Provider Order
          </label>
          <Input
            id="or-order"
            value={fromArray(prefs?.order)}
            onChange={(e) => updatePref("order", toArray(e.target.value))}
            placeholder="deepinfra, deepseek"
            className="text-[12px] bg-foreground/[0.03] border-foreground/[0.06]"
          />
          <p className="text-[10px] text-muted-foreground/35">
            Comma-separated. Try these providers first, in order.
          </p>
        </div>

        <div className="space-y-1.5">
          <label htmlFor="or-only" className="text-[11px] font-medium text-foreground/70">
            Allowlist
          </label>
          <Input
            id="or-only"
            value={fromArray(prefs?.only)}
            onChange={(e) => updatePref("only", toArray(e.target.value))}
            placeholder="deepinfra, atlascloud"
            className="text-[12px] bg-foreground/[0.03] border-foreground/[0.06]"
          />
          <p className="text-[10px] text-muted-foreground/35">Only route to these providers.</p>
        </div>

        <div className="space-y-1.5">
          <label htmlFor="or-ignore" className="text-[11px] font-medium text-foreground/70">
            Blocklist
          </label>
          <Input
            id="or-ignore"
            value={fromArray(prefs?.ignore)}
            onChange={(e) => updatePref("ignore", toArray(e.target.value))}
            placeholder="google vertex"
            className="text-[12px] bg-foreground/[0.03] border-foreground/[0.06]"
          />
          <p className="text-[10px] text-muted-foreground/35">Never route to these providers.</p>
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <label htmlFor="or-sort" className="text-[11px] font-medium text-foreground/70">
              Sort By
            </label>
            <Select
              value={prefs?.sort || "__none__"}
              onValueChange={(value) => updatePref("sort", value === "__none__" ? null : value)}
            >
              <SelectTrigger
                id="or-sort"
                className="w-full text-[12px] bg-foreground/[0.03] border-foreground/[0.06]"
              >
                <SelectValue placeholder="Default" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">Default</SelectItem>
                <SelectItem value="price">Price</SelectItem>
                <SelectItem value="throughput">Throughput</SelectItem>
                <SelectItem value="latency">Latency</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1.5">
            <label htmlFor="or-data" className="text-[11px] font-medium text-foreground/70">
              Data Collection
            </label>
            <Select
              value={prefs?.data_collection || "__none__"}
              onValueChange={(value) =>
                updatePref("data_collection", value === "__none__" ? null : value)
              }
            >
              <SelectTrigger
                id="or-data"
                className="w-full text-[12px] bg-foreground/[0.03] border-foreground/[0.06]"
              >
                <SelectValue placeholder="Default" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">Default (allow)</SelectItem>
                <SelectItem value="allow">Allow</SelectItem>
                <SelectItem value="deny">Deny</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <div className="space-y-1.5">
          <label htmlFor="or-quant" className="text-[11px] font-medium text-foreground/70">
            Quantizations
          </label>
          <Input
            id="or-quant"
            value={fromArray(prefs?.quantizations)}
            onChange={(e) => updatePref("quantizations", toArray(e.target.value))}
            placeholder="fp8, fp16"
            className="text-[12px] bg-foreground/[0.03] border-foreground/[0.06]"
          />
          <p className="text-[10px] text-muted-foreground/35">int4, int8, fp8, fp16, bf16, fp32</p>
        </div>

        <div className="flex flex-wrap gap-x-5 gap-y-2 pt-1">
          <label className="flex items-center gap-2 text-[11px] text-foreground/60 cursor-pointer hover:text-foreground/80 transition-colors">
            <Switch
              checked={prefs?.allow_fallbacks ?? true}
              onCheckedChange={(checked) => updatePref("allow_fallbacks", checked)}
            />
            Fallbacks
          </label>
          <label className="flex items-center gap-2 text-[11px] text-foreground/60 cursor-pointer hover:text-foreground/80 transition-colors">
            <Switch
              checked={prefs?.zdr ?? false}
              onCheckedChange={(checked) => updatePref("zdr", checked || null)}
            />
            Zero Data Retention
          </label>
          <label className="flex items-center gap-2 text-[11px] text-foreground/60 cursor-pointer hover:text-foreground/80 transition-colors">
            <Switch
              checked={prefs?.require_parameters ?? false}
              onCheckedChange={(checked) => updatePref("require_parameters", checked || null)}
            />
            Require Params
          </label>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
