import { ChevronDown, ChevronRight, Eye, EyeOff, ExternalLink, Star } from "lucide-react";
import { useState } from "react";
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
import type { AiSettings, OpenRouterProviderPreferences, WebSearchContextSize } from "@/lib/settings";
import { cn } from "@/lib/utils";
import { ModelSelector } from "./ModelSelector";
import { useTranslation } from "react-i18next";
import { useProviderForm, type ProviderConfig } from "./hooks/useProviderForm";

interface ProviderSettingsProps {
  settings: AiSettings;
  onChange: (settings: AiSettings) => void;
}

function PasswordInput({
  id,
  value,
  onChange,
  placeholder,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
}) {
  const [showPassword, setShowPassword] = useState(false);

  return (
    <div className="relative group">
      <Input
        id={id}
        type={showPassword ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="pr-10 font-mono text-[12px] bg-foreground/[0.03] border-foreground/[0.06] focus:border-accent/40 focus:ring-accent/10 transition-colors"
      />
      <button
        type="button"
        onClick={() => setShowPassword(!showPassword)}
        className="absolute right-2 top-1/2 -translate-y-1/2 p-1 rounded text-muted-foreground/40 hover:text-foreground/80 hover:bg-foreground/[0.05] transition-all"
      >
        {showPassword ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
      </button>
    </div>
  );
}

function OpenRouterProviderPreferencesSection({
  settings,
  updatePref,
}: {
  settings: AiSettings;
  updatePref: <K extends keyof OpenRouterProviderPreferences>(field: K, value: OpenRouterProviderPreferences[K]) => void;
}) {
  const prefs = settings.openrouter.provider_preferences;

  const toArray = (val: string): string[] | null => {
    const arr = val
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    return arr.length > 0 ? arr : null;
  };

  // Helper to convert array to comma-separated string
  const fromArray = (arr?: string[] | null): string => (arr || []).join(", ");

  // Check if any preferences are configured
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
          <p className="text-[10px] text-muted-foreground/35">Comma-separated. Try these providers first, in order.</p>
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
              <SelectTrigger id="or-sort" className="w-full text-[12px] bg-foreground/[0.03] border-foreground/[0.06]">
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
              onValueChange={(value) => updatePref("data_collection", value === "__none__" ? null : value)}
            >
              <SelectTrigger id="or-data" className="w-full text-[12px] bg-foreground/[0.03] border-foreground/[0.06]">
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

export function ProviderSettings({ settings, onChange }: ProviderSettingsProps) {
  const { t } = useTranslation();
  const {
    selectedId, setSelectedId, configuredProviders, unconfiguredProviders,
    updateProvider, updateOpenRouterPref, getShowInSelector, getColor,
  } = useProviderForm(settings, onChange);

  const renderProviderFields = (provider: ProviderConfig) => {
    const fieldLabel = "text-[11px] font-medium text-foreground/70";
    const fieldHint = "text-[10px] text-muted-foreground/35 leading-relaxed";
    const fieldInput = "text-[12px] bg-foreground/[0.03] border-foreground/[0.06] focus:border-accent/40 focus:ring-accent/10";
    const fieldLink = "text-accent/60 hover:text-accent transition-colors inline-flex items-center gap-0.5";

    switch (provider.id) {
      case "vertex_ai":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="vertex-credentials" className={fieldLabel}>Credentials Path</label>
              <Input
                id="vertex-credentials"
                value={settings.vertex_ai.credentials_path || ""}
                onChange={(e) => updateProvider("vertex_ai", "credentials_path", e.target.value)}
                placeholder="/path/to/service-account.json"
                className={cn(fieldInput, "font-mono")}
              />
              <p className={fieldHint}>Google Cloud service account JSON file</p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-project" className={fieldLabel}>Project ID</label>
              <Input
                id="vertex-project"
                value={settings.vertex_ai.project_id || ""}
                onChange={(e) => updateProvider("vertex_ai", "project_id", e.target.value)}
                placeholder="your-gcp-project-id"
                className={fieldInput}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-location" className={fieldLabel}>Location</label>
              <Input
                id="vertex-location"
                value={settings.vertex_ai.location || ""}
                onChange={(e) => updateProvider("vertex_ai", "location", e.target.value)}
                placeholder="us-east5"
                className={fieldInput}
              />
              <p className={fieldHint}>Region (e.g., us-east5, europe-west1)</p>
            </div>
          </div>
        );

      case "vertex_gemini":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="vertex-gemini-credentials" className={fieldLabel}>Credentials Path</label>
              <Input
                id="vertex-gemini-credentials"
                value={settings.vertex_gemini.credentials_path || ""}
                onChange={(e) => updateProvider("vertex_gemini", "credentials_path", e.target.value)}
                placeholder="/path/to/service-account.json"
                className={cn(fieldInput, "font-mono")}
              />
              <p className={fieldHint}>Google Cloud service account JSON file</p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-gemini-project" className={fieldLabel}>Project ID</label>
              <Input
                id="vertex-gemini-project"
                value={settings.vertex_gemini.project_id || ""}
                onChange={(e) => updateProvider("vertex_gemini", "project_id", e.target.value)}
                placeholder="your-gcp-project-id"
                className={fieldInput}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-gemini-location" className={fieldLabel}>Location</label>
              <Input
                id="vertex-gemini-location"
                value={settings.vertex_gemini.location || ""}
                onChange={(e) => updateProvider("vertex_gemini", "location", e.target.value)}
                placeholder="us-central1"
                className={fieldInput}
              />
              <p className={fieldHint}>Region (e.g., us-central1, europe-west1)</p>
            </div>
          </div>
        );

      case "anthropic":
        return (
          <div className="space-y-1.5">
            <label htmlFor="anthropic-key" className={fieldLabel}>API Key</label>
            <PasswordInput
              id="anthropic-key"
              value={settings.anthropic.api_key || ""}
              onChange={(value) => updateProvider("anthropic", "api_key", value)}
              placeholder="sk-ant-api03-..."
            />
            <p className={fieldHint}>
              From{" "}
              <a href="https://console.anthropic.com" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                console.anthropic.com <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "openai":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="openai-key" className={fieldLabel}>API Key</label>
              <PasswordInput
                id="openai-key"
                value={settings.openai.api_key || ""}
                onChange={(value) => updateProvider("openai", "api_key", value)}
                placeholder="sk-..."
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="openai-base" className={fieldLabel}>
                Base URL <span className="text-muted-foreground/30 font-normal">(optional)</span>
              </label>
              <Input
                id="openai-base"
                value={settings.openai.base_url || ""}
                onChange={(e) => updateProvider("openai", "base_url", e.target.value)}
                placeholder="https://api.openai.com/v1"
                className={fieldInput}
              />
              <p className={fieldHint}>Custom endpoint for OpenAI-compatible APIs</p>
            </div>
            <div className="flex items-center justify-between py-2.5 border-t border-foreground/[0.04]">
              <div>
                <div className="text-[12px] font-medium text-foreground/80">Web Search</div>
                <div className="text-[10px] text-muted-foreground/35">Native web search tool</div>
              </div>
              <Switch
                checked={settings.openai.enable_web_search}
                onCheckedChange={(checked) => updateProvider("openai", "enable_web_search", checked)}
              />
            </div>
            {settings.openai.enable_web_search && (
              <div className="space-y-1.5">
                <label htmlFor="openai-search-context" className={fieldLabel}>Search Context Size</label>
                <Select
                  value={settings.openai.web_search_context_size}
                  onValueChange={(value: WebSearchContextSize) =>
                    updateProvider("openai", "web_search_context_size", value)
                  }
                >
                  <SelectTrigger id="openai-search-context" className={fieldInput}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="low">Low (faster)</SelectItem>
                    <SelectItem value="medium">Medium (balanced)</SelectItem>
                    <SelectItem value="high">High (thorough)</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            )}
          </div>
        );

      case "openrouter":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="openrouter-key" className={fieldLabel}>API Key</label>
              <PasswordInput
                id="openrouter-key"
                value={settings.openrouter.api_key || ""}
                onChange={(value) => updateProvider("openrouter", "api_key", value)}
                placeholder="sk-or-v1-..."
              />
              <p className={fieldHint}>
                From{" "}
                <a href="https://openrouter.ai" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                  openrouter.ai <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="h-px bg-foreground/[0.04]" />
            <OpenRouterProviderPreferencesSection settings={settings} updatePref={updateOpenRouterPref} />
          </div>
        );

      case "ollama":
        return (
          <div className="space-y-1.5">
            <label htmlFor="ollama-url" className={fieldLabel}>Base URL</label>
            <Input
              id="ollama-url"
              value={settings.ollama.base_url}
              onChange={(e) => updateProvider("ollama", "base_url", e.target.value)}
              placeholder="http://localhost:11434"
              className={cn(fieldInput, "font-mono")}
            />
            <p className={fieldHint}>Ollama server endpoint</p>
          </div>
        );

      case "gemini":
        return (
          <div className="space-y-1.5">
            <label htmlFor="gemini-key" className={fieldLabel}>API Key</label>
            <PasswordInput
              id="gemini-key"
              value={settings.gemini.api_key || ""}
              onChange={(value) => updateProvider("gemini", "api_key", value)}
              placeholder="AIza..."
            />
            <p className={fieldHint}>
              From{" "}
              <a href="https://aistudio.google.com" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                aistudio.google.com <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "groq":
        return (
          <div className="space-y-1.5">
            <label htmlFor="groq-key" className={fieldLabel}>API Key</label>
            <PasswordInput
              id="groq-key"
              value={settings.groq.api_key || ""}
              onChange={(value) => updateProvider("groq", "api_key", value)}
              placeholder="gsk_..."
            />
            <p className={fieldHint}>
              From{" "}
              <a href="https://console.groq.com" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                console.groq.com <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "xai":
        return (
          <div className="space-y-1.5">
            <label htmlFor="xai-key" className={fieldLabel}>API Key</label>
            <PasswordInput
              id="xai-key"
              value={settings.xai.api_key || ""}
              onChange={(value) => updateProvider("xai", "api_key", value)}
              placeholder="xai-..."
            />
            <p className={fieldHint}>
              From{" "}
              <a href="https://x.ai" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                x.ai <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "zai_sdk":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="z-ai-sdk-key" className={fieldLabel}>API Key</label>
              <PasswordInput
                id="z-ai-sdk-key"
                value={settings.zai_sdk?.api_key || ""}
                onChange={(value) => updateProvider("zai_sdk", "api_key", value)}
                placeholder="your-zai-api-key"
              />
              <p className={fieldHint}>
                From{" "}
                <a href="https://open.bigmodel.cn" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                  open.bigmodel.cn <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="z-ai-sdk-base" className={fieldLabel}>
                Base URL <span className="text-muted-foreground/30 font-normal">(optional)</span>
              </label>
              <Input
                id="z-ai-sdk-base"
                value={settings.zai_sdk?.base_url || ""}
                onChange={(e) => updateProvider("zai_sdk", "base_url", e.target.value)}
                placeholder="https://open.bigmodel.cn/api/paas/v4"
                className={cn(fieldInput, "font-mono")}
              />
            </div>
          </div>
        );

      case "nvidia":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="nvidia-key" className={fieldLabel}>API Key</label>
              <PasswordInput
                id="nvidia-key"
                value={settings.nvidia?.api_key || ""}
                onChange={(value) => updateProvider("nvidia", "api_key", value)}
                placeholder="nvapi-..."
              />
              <p className={fieldHint}>
                From{" "}
                <a href="https://build.nvidia.com" target="_blank" rel="noopener noreferrer" className={fieldLink}>
                  build.nvidia.com <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="nvidia-base" className={fieldLabel}>
                Base URL <span className="text-muted-foreground/30 font-normal">(optional)</span>
              </label>
              <Input
                id="nvidia-base"
                value={settings.nvidia?.base_url || ""}
                onChange={(e) => updateProvider("nvidia", "base_url", e.target.value)}
                placeholder="https://integrate.api.nvidia.com/v1"
                className={cn(fieldInput, "font-mono")}
              />
            </div>
          </div>
        );

      default:
        return null;
    }
  };

  return (
    <div className="overflow-y-auto space-y-8 pb-10" style={{ height: "calc(100vh - 140px)" }}>
      {/* Default Model */}
      <div className="rounded-xl border border-foreground/[0.05] bg-foreground/[0.015] p-5">
        <div className="text-[13px] font-semibold text-foreground/90 mb-1">{t("provider.defaultModel")}</div>
        <p className="text-[11px] text-muted-foreground/40 mb-4 leading-relaxed">{t("provider.defaultModelDesc")}</p>
        <div className="max-w-lg">
          <ModelSelector
            provider={settings.default_provider}
            model={settings.default_model}
            reasoningEffort={settings.default_reasoning_effort}
            settings={settings}
            onChange={(provider, model, reasoningEffort) =>
              onChange({
                ...settings,
                default_provider: provider,
                default_model: model,
                default_reasoning_effort: reasoningEffort,
              })
            }
          />
        </div>
      </div>

      {/* Active Providers */}
      {configuredProviders.length > 0 && (
        <div>
          <div className="flex items-center gap-3 mb-3">
            <span className="text-[10px] font-bold uppercase tracking-[0.15em] text-emerald-400/80">
              {t("provider.active", "Active")}
            </span>
            <span className="text-[10px] font-semibold text-emerald-400/50 bg-emerald-400/[0.08] rounded-full px-2 py-0.5 min-w-[20px] text-center">
              {configuredProviders.length}
            </span>
            <div className="flex-1 h-px bg-gradient-to-r from-emerald-400/15 to-transparent" />
          </div>
          <div className="space-y-2">
            {configuredProviders.map((provider) => {
              const isDefault = settings.default_provider === provider.id;
              const isOpen = selectedId === provider.id;
              const color = getColor(provider.id);
              return (
                <div
                  key={provider.id}
                  className={cn(
                    "relative rounded-xl overflow-hidden transition-all duration-200",
                    isOpen
                      ? "ring-1 ring-foreground/[0.08] bg-foreground/[0.025]"
                      : "ring-1 ring-foreground/[0.04] hover:ring-foreground/[0.08] hover:bg-foreground/[0.015]"
                  )}
                >
                  {/* Left accent bar */}
                  <div
                    className="absolute left-0 top-2 bottom-2 w-[2px] rounded-full transition-opacity duration-200"
                    style={{ backgroundColor: color.border, opacity: isOpen ? 1 : 0.6 }}
                  />

                  <button
                    type="button"
                    onClick={() => setSelectedId(isOpen ? null : provider.id)}
                    className="w-full flex items-center gap-3.5 pl-4 pr-3.5 py-3 text-left"
                  >
                    {/* Icon container */}
                    <div
                      className="w-8 h-8 rounded-lg flex items-center justify-center text-[13px] flex-shrink-0 transition-transform duration-200 hover:scale-105"
                      style={{ backgroundColor: color.bg }}
                    >
                      {provider.icon}
                    </div>

                    {/* Name & description */}
                    <div className="flex-1 min-w-0">
                      <div className="text-[13px] font-medium text-foreground/90">{provider.name}</div>
                      <div className="text-[10px] text-muted-foreground/35 mt-0.5 truncate">{provider.description}</div>
                    </div>

                    {/* Indicators */}
                    <div className="flex items-center gap-2.5 flex-shrink-0">
                      {isDefault && (
                        <Star className="w-3.5 h-3.5 text-accent fill-accent/50" />
                      )}
                      <div
                        className="w-2 h-2 rounded-full"
                        style={{
                          backgroundColor: color.dot,
                          boxShadow: `0 0 6px ${color.dot}40`,
                        }}
                      />
                      <ChevronDown className={cn(
                        "w-3.5 h-3.5 text-muted-foreground/25 transition-transform duration-200",
                        isOpen && "rotate-180"
                      )} />
                    </div>
                  </button>

                  {/* Expanded configuration panel */}
                  {isOpen && (
                    <div className="px-4 pb-4 pt-1">
                      <div className="rounded-lg bg-foreground/[0.02] border border-foreground/[0.04] p-4 space-y-4">
                        {/* Show in selector toggle */}
                        <div className="flex items-center justify-between">
                          <span className="text-[11px] text-muted-foreground/50">{t("provider.showInSelector")}</span>
                          <Switch
                            checked={getShowInSelector(provider.id)}
                            onCheckedChange={(checked) => updateProvider(provider.id, "show_in_selector", checked)}
                          />
                        </div>
                        <div className="h-px bg-foreground/[0.04]" />
                        {renderProviderFields(provider)}
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Available Providers */}
      {unconfiguredProviders.length > 0 && (
        <div>
          <div className="flex items-center gap-3 mb-3">
            <span className="text-[10px] font-bold uppercase tracking-[0.15em] text-muted-foreground/40">
              {t("provider.available", "Available")}
            </span>
            <span className="text-[10px] font-semibold text-muted-foreground/30 bg-foreground/[0.03] rounded-full px-2 py-0.5 min-w-[20px] text-center">
              {unconfiguredProviders.length}
            </span>
            <div className="flex-1 h-px bg-gradient-to-r from-foreground/[0.06] to-transparent" />
          </div>
          <div className="space-y-2">
            {unconfiguredProviders.map((provider) => {
              const isOpen = selectedId === provider.id;
              const color = getColor(provider.id);
              return (
                <div
                  key={provider.id}
                  className={cn(
                    "relative rounded-xl overflow-hidden transition-all duration-200",
                    isOpen
                      ? "ring-1 ring-foreground/[0.08] bg-foreground/[0.02]"
                      : "ring-1 ring-foreground/[0.03] hover:ring-foreground/[0.06] hover:bg-foreground/[0.01]"
                  )}
                >
                  <button
                    type="button"
                    onClick={() => setSelectedId(isOpen ? null : provider.id)}
                    className="w-full flex items-center gap-3.5 px-4 py-3 text-left group"
                  >
                    {/* Icon container (dimmed) */}
                    <div
                      className="w-8 h-8 rounded-lg flex items-center justify-center text-[13px] flex-shrink-0 opacity-40 group-hover:opacity-60 transition-opacity"
                      style={{ backgroundColor: color.bg }}
                    >
                      {provider.icon}
                    </div>

                    {/* Name & description */}
                    <div className="flex-1 min-w-0">
                      <div className="text-[13px] font-medium text-foreground/45 group-hover:text-foreground/70 transition-colors">
                        {provider.name}
                      </div>
                      <div className="text-[10px] text-muted-foreground/25 mt-0.5 truncate">{provider.description}</div>
                    </div>

                    <ChevronDown className={cn(
                      "w-3.5 h-3.5 text-muted-foreground/15 group-hover:text-muted-foreground/30 transition-all duration-200",
                      isOpen && "rotate-180"
                    )} />
                  </button>

                  {/* Expanded configuration panel */}
                  {isOpen && (
                    <div className="px-4 pb-4 pt-1">
                      <div className="rounded-lg bg-foreground/[0.02] border border-foreground/[0.04] p-4 space-y-4">
                        <div className="flex items-center justify-between">
                          <span className="text-[11px] text-muted-foreground/50">{t("provider.showInSelector")}</span>
                          <Switch
                            checked={getShowInSelector(provider.id)}
                            onCheckedChange={(checked) => updateProvider(provider.id, "show_in_selector", checked)}
                          />
                        </div>
                        <div className="h-px bg-foreground/[0.04]" />
                        {renderProviderFields(provider)}
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
