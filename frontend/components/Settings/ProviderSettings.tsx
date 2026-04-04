import { ChevronDown, Eye, EyeOff, Star } from "lucide-react";
import { useEffect, useState } from "react";
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
import { logger } from "@/lib/logger";
import { getProviders, type ProviderInfo } from "@/lib/model-registry";
import type { AiSettings, OpenRouterProviderPreferences, WebSearchContextSize } from "@/lib/settings";
import { cn } from "@/lib/utils";
import { ModelSelector } from "./ModelSelector";
import { useTranslation } from "react-i18next";

interface ProviderSettingsProps {
  settings: AiSettings;
  onChange: (settings: AiSettings) => void;
}

type ProviderSettingsKey = keyof Pick<
  AiSettings,
  | "vertex_ai"
  | "vertex_gemini"
  | "openrouter"
  | "anthropic"
  | "openai"
  | "ollama"
  | "gemini"
  | "groq"
  | "xai"
  | "zai_sdk"
  | "nvidia"
>;

interface ProviderConfig {
  id: ProviderSettingsKey;
  name: string;
  icon: string;
  description: string;
  getConfigured: (settings: AiSettings) => boolean;
}

/**
 * Check if a provider is configured based on its credentials.
 * This logic must remain in the frontend since it depends on the settings object.
 */
function isProviderConfigured(id: ProviderSettingsKey, settings: AiSettings): boolean {
  switch (id) {
    case "anthropic":
      return !!settings.anthropic.api_key;
    case "gemini":
      return !!settings.gemini.api_key;
    case "groq":
      return !!settings.groq.api_key;
    case "ollama":
      return !!settings.ollama.base_url;
    case "openai":
      return !!settings.openai.api_key;
    case "openrouter":
      return !!settings.openrouter.api_key;
    case "vertex_ai":
      return !!(settings.vertex_ai.credentials_path || settings.vertex_ai.project_id);
    case "vertex_gemini":
      return !!(settings.vertex_gemini.credentials_path || settings.vertex_gemini.project_id);
    case "xai":
      return !!settings.xai.api_key;
    case "zai_sdk":
      return !!settings.zai_sdk?.api_key;
    case "nvidia":
      return !!settings.nvidia?.api_key;
    default:
      return false;
  }
}

/**
 * Map backend AiProvider to frontend settings key.
 */
function providerToSettingsKey(provider: string): ProviderSettingsKey | null {
  const mapping: Record<string, ProviderSettingsKey> = {
    vertex_ai: "vertex_ai",
    vertex_gemini: "vertex_gemini",
    anthropic: "anthropic",
    openai: "openai",
    openrouter: "openrouter",
    ollama: "ollama",
    gemini: "gemini",
    groq: "groq",
    xai: "xai",
    zai_sdk: "zai_sdk",
    nvidia: "nvidia",
  };
  return mapping[provider] ?? null;
}

/**
 * Convert backend ProviderInfo to ProviderConfig with configuration checker.
 */
function toProviderConfig(info: ProviderInfo): ProviderConfig | null {
  const settingsKey = providerToSettingsKey(info.provider);
  if (!settingsKey) return null;

  return {
    id: settingsKey,
    name: info.name,
    icon: info.icon,
    description: info.description,
    getConfigured: (settings: AiSettings) => isProviderConfigured(settingsKey, settings),
  };
}

// Fallback static providers in case backend fetch fails
const FALLBACK_PROVIDERS: ProviderConfig[] = [
  {
    id: "anthropic",
    name: "Anthropic",
    icon: "🔶",
    description: "Direct Claude API access",
    getConfigured: (s) => !!s.anthropic.api_key,
  },
  {
    id: "gemini",
    name: "Gemini",
    icon: "💎",
    description: "Google AI models via API",
    getConfigured: (s) => !!s.gemini.api_key,
  },
  {
    id: "groq",
    name: "Groq",
    icon: "⚡",
    description: "Ultra-fast LLM inference",
    getConfigured: (s) => !!s.groq.api_key,
  },
  {
    id: "ollama",
    name: "Ollama",
    icon: "🦙",
    description: "Run models locally on your machine",
    getConfigured: (s) => !!s.ollama.base_url,
  },
  {
    id: "openai",
    name: "OpenAI",
    icon: "⚪",
    description: "GPT models via OpenAI API",
    getConfigured: (s) => !!s.openai.api_key,
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    icon: "🔀",
    description: "Access multiple models via one API",
    getConfigured: (s) => !!s.openrouter.api_key,
  },
  {
    id: "vertex_ai",
    name: "Vertex AI",
    icon: "🔷",
    description: "Claude models via Google Cloud",
    getConfigured: (s) => !!(s.vertex_ai.credentials_path || s.vertex_ai.project_id),
  },
  {
    id: "vertex_gemini",
    name: "Vertex AI Gemini",
    icon: "💎",
    description: "Gemini models via Google Cloud",
    getConfigured: (s) => !!(s.vertex_gemini.credentials_path || s.vertex_gemini.project_id),
  },
  {
    id: "xai",
    name: "xAI",
    icon: "𝕏",
    description: "Grok models from xAI",
    getConfigured: (s) => !!s.xai.api_key,
  },
  {
    id: "zai_sdk",
    name: "Z.AI SDK",
    icon: "🤖",
    description: "Z.AI native SDK (GLM models)",
    getConfigured: (s) => !!s.zai_sdk?.api_key,
  },
  {
    id: "nvidia",
    name: "NVIDIA NIM",
    icon: "🟢",
    description: "NVIDIA NIM inference (OpenAI-compatible)",
    getConfigured: (s) => !!s.nvidia?.api_key,
  },
];

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
    <div className="relative">
      <Input
        id={id}
        type={showPassword ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="pr-10 font-mono text-sm"
      />
      <button
        type="button"
        onClick={() => setShowPassword(!showPassword)}
        className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground hover:text-foreground transition-colors"
      >
        {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
      </button>
    </div>
  );
}

/**
 * OpenRouter Provider Preferences sub-section.
 * Allows configuring provider routing, filtering, and prioritization.
 */
function OpenRouterProviderPreferencesSection({
  settings,
  onChange,
}: {
  settings: AiSettings;
  onChange: (settings: AiSettings) => void;
}) {
  const prefs = settings.openrouter.provider_preferences;

  const updatePref = <K extends keyof OpenRouterProviderPreferences>(
    field: K,
    value: OpenRouterProviderPreferences[K]
  ) => {
    onChange({
      ...settings,
      openrouter: {
        ...settings.openrouter,
        provider_preferences: {
          ...(prefs || {}),
          [field]: value,
        },
      },
    });
  };

  // Helper to convert comma-separated string to array or null
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
      <CollapsibleTrigger className="flex w-full items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors">
        <ChevronDown className="h-3.5 w-3.5 transition-transform duration-200 [[data-state=open]>&]:rotate-180" />
        Provider Preferences
        {hasPrefs && (
          <span className="ml-auto text-[10px] px-1.5 py-0.5 rounded bg-accent/20 text-accent">
            Active
          </span>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent className="pt-2 space-y-3">
        <p className="text-xs text-muted-foreground">
          Control which providers handle your requests.{" "}
          <a
            href="https://openrouter.ai/docs/guides/routing/provider-selection"
            target="_blank"
            rel="noopener noreferrer"
            className="underline"
          >
            Learn more
          </a>
        </p>

        {/* Provider Order */}
        <div className="space-y-1">
          <label htmlFor="or-order" className="text-xs text-foreground">
            Provider Order
          </label>
          <Input
            id="or-order"
            value={fromArray(prefs?.order)}
            onChange={(e) => updatePref("order", toArray(e.target.value))}
            placeholder="deepinfra, deepseek"
          />
          <p className="text-[11px] text-muted-foreground">Comma-separated. Try these providers first, in order.</p>
        </div>

        {/* Only */}
        <div className="space-y-1">
          <label htmlFor="or-only" className="text-xs text-foreground">
            Allowlist (Only)
          </label>
          <Input
            id="or-only"
            value={fromArray(prefs?.only)}
            onChange={(e) => updatePref("only", toArray(e.target.value))}
            placeholder="deepinfra, atlascloud"
          />
          <p className="text-[11px] text-muted-foreground">Comma-separated. Only use these providers.</p>
        </div>

        {/* Ignore */}
        <div className="space-y-1">
          <label htmlFor="or-ignore" className="text-xs text-foreground">
            Blocklist (Ignore)
          </label>
          <Input
            id="or-ignore"
            value={fromArray(prefs?.ignore)}
            onChange={(e) => updatePref("ignore", toArray(e.target.value))}
            placeholder="google vertex"
          />
          <p className="text-[11px] text-muted-foreground">Comma-separated. Never use these providers.</p>
        </div>

        {/* Sort Strategy */}
        <div className="space-y-1">
          <label htmlFor="or-sort" className="text-xs text-foreground">
            Sort By
          </label>
          <Select
            value={prefs?.sort || "__none__"}
            onValueChange={(value) => updatePref("sort", value === "__none__" ? null : value)}
          >
            <SelectTrigger id="or-sort" className="w-full">
              <SelectValue placeholder="Default (no preference)" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">Default (no preference)</SelectItem>
              <SelectItem value="price">Price (cheapest first)</SelectItem>
              <SelectItem value="throughput">Throughput (fastest generation)</SelectItem>
              <SelectItem value="latency">Latency (lowest latency)</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* Quantizations */}
        <div className="space-y-1">
          <label htmlFor="or-quant" className="text-xs text-foreground">
            Quantizations
          </label>
          <Input
            id="or-quant"
            value={fromArray(prefs?.quantizations)}
            onChange={(e) => updatePref("quantizations", toArray(e.target.value))}
            placeholder="fp8, fp16"
          />
          <p className="text-[11px] text-muted-foreground">Comma-separated. Filter by: int4, int8, fp8, fp16, bf16, fp32</p>
        </div>

        {/* Toggles row */}
        <div className="flex flex-wrap gap-x-4 gap-y-2">
          <label className="flex items-center gap-1.5 text-xs text-foreground">
            <Switch
              checked={prefs?.allow_fallbacks ?? true}
              onCheckedChange={(checked) => updatePref("allow_fallbacks", checked)}
            />
            Allow Fallbacks
          </label>
          <label className="flex items-center gap-1.5 text-xs text-foreground">
            <Switch
              checked={prefs?.zdr ?? false}
              onCheckedChange={(checked) => updatePref("zdr", checked || null)}
            />
            Zero Data Retention
          </label>
          <label className="flex items-center gap-1.5 text-xs text-foreground">
            <Switch
              checked={prefs?.require_parameters ?? false}
              onCheckedChange={(checked) => updatePref("require_parameters", checked || null)}
            />
            Require Parameters
          </label>
        </div>

        {/* Data Collection */}
        <div className="space-y-1">
          <label htmlFor="or-data" className="text-xs text-foreground">
            Data Collection Policy
          </label>
          <Select
            value={prefs?.data_collection || "__none__"}
            onValueChange={(value) => updatePref("data_collection", value === "__none__" ? null : value)}
          >
            <SelectTrigger id="or-data" className="w-full">
              <SelectValue placeholder="Default (allow)" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">Default (allow)</SelectItem>
              <SelectItem value="allow">Allow</SelectItem>
              <SelectItem value="deny">Deny</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

export function ProviderSettings({ settings, onChange }: ProviderSettingsProps) {
  const { t } = useTranslation();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderConfig[]>(FALLBACK_PROVIDERS);

  useEffect(() => {
    getProviders()
      .then((backendProviders) => {
        const configs = backendProviders
          .map(toProviderConfig)
          .filter((p): p is ProviderConfig => p !== null);
        if (configs.length > 0) {
          setProviders(configs);
        }
      })
      .catch((err) => {
        logger.warn("Failed to fetch providers from backend, using fallback:", err);
      });
  }, []);

  const updateProvider = <K extends keyof AiSettings>(
    provider: K,
    field: string,
    value: string | boolean | null
  ) => {
    const providerSettings = settings[provider];
    if (typeof providerSettings === "object" && providerSettings !== null) {
      onChange({
        ...settings,
        [provider]: {
          ...providerSettings,
          [field]: typeof value === "boolean" ? value : value || null,
        },
      });
    }
  };

  const selectedProvider = providers.find((p) => p.id === selectedId) ?? null;

  const renderProviderFields = (provider: ProviderConfig) => {
    switch (provider.id) {
      case "vertex_ai":
        return (
          <div className="space-y-4">
            <div className="space-y-2">
              <label htmlFor="vertex-credentials" className="text-sm text-foreground">
                Credentials Path
              </label>
              <Input
                id="vertex-credentials"
                value={settings.vertex_ai.credentials_path || ""}
                onChange={(e) => updateProvider("vertex_ai", "credentials_path", e.target.value)}
                placeholder="/path/to/service-account.json"
                className="font-mono text-sm"
              />
              <p className="text-xs text-muted-foreground">
                Path to your Google Cloud service account JSON file
              </p>
            </div>

            <div className="space-y-2">
              <label htmlFor="vertex-project" className="text-sm text-foreground">
                Project ID
              </label>
              <Input
                id="vertex-project"
                value={settings.vertex_ai.project_id || ""}
                onChange={(e) => updateProvider("vertex_ai", "project_id", e.target.value)}
                placeholder="your-gcp-project-id"
              />
            </div>

            <div className="space-y-2">
              <label htmlFor="vertex-location" className="text-sm text-foreground">
                Location
              </label>
              <Input
                id="vertex-location"
                value={settings.vertex_ai.location || ""}
                onChange={(e) => updateProvider("vertex_ai", "location", e.target.value)}
                placeholder="us-east5"
              />
              <p className="text-xs text-muted-foreground">
                Google Cloud region (e.g., us-east5, europe-west1)
              </p>
            </div>
          </div>
        );

      case "vertex_gemini":
        return (
          <div className="space-y-4">
            <div className="space-y-2">
              <label htmlFor="vertex-gemini-credentials" className="text-sm text-foreground">
                Credentials Path
              </label>
              <Input
                id="vertex-gemini-credentials"
                value={settings.vertex_gemini.credentials_path || ""}
                onChange={(e) =>
                  updateProvider("vertex_gemini", "credentials_path", e.target.value)
                }
                placeholder="/path/to/service-account.json"
                className="font-mono text-sm"
              />
              <p className="text-xs text-muted-foreground">
                Path to your Google Cloud service account JSON file
              </p>
            </div>

            <div className="space-y-2">
              <label htmlFor="vertex-gemini-project" className="text-sm text-foreground">
                Project ID
              </label>
              <Input
                id="vertex-gemini-project"
                value={settings.vertex_gemini.project_id || ""}
                onChange={(e) => updateProvider("vertex_gemini", "project_id", e.target.value)}
                placeholder="your-gcp-project-id"
              />
            </div>

            <div className="space-y-2">
              <label htmlFor="vertex-gemini-location" className="text-sm text-foreground">
                Location
              </label>
              <Input
                id="vertex-gemini-location"
                value={settings.vertex_gemini.location || ""}
                onChange={(e) => updateProvider("vertex_gemini", "location", e.target.value)}
                placeholder="us-central1"
              />
              <p className="text-xs text-muted-foreground">
                Google Cloud region (e.g., us-central1, europe-west1)
              </p>
            </div>
          </div>
        );

      case "anthropic":
        return (
          <div className="space-y-2">
            <label htmlFor="anthropic-key" className="text-sm text-foreground">
              API Key
            </label>
            <PasswordInput
              id="anthropic-key"
              value={settings.anthropic.api_key || ""}
              onChange={(value) => updateProvider("anthropic", "api_key", value)}
              placeholder="sk-ant-api03-..."
            />
            <p className="text-xs text-muted-foreground">
              Get your API key from console.anthropic.com
            </p>
          </div>
        );

      case "openai":
        return (
          <div className="space-y-4">
            <div className="space-y-2">
              <label htmlFor="openai-key" className="text-sm text-foreground">
                API Key
              </label>
              <PasswordInput
                id="openai-key"
                value={settings.openai.api_key || ""}
                onChange={(value) => updateProvider("openai", "api_key", value)}
                placeholder="sk-..."
              />
            </div>

            <div className="space-y-2">
              <label htmlFor="openai-base" className="text-sm text-foreground">
                Base URL <span className="text-muted-foreground font-normal">(optional)</span>
              </label>
              <Input
                id="openai-base"
                value={settings.openai.base_url || ""}
                onChange={(e) => updateProvider("openai", "base_url", e.target.value)}
                placeholder="https://api.openai.com/v1"
              />
              <p className="text-xs text-muted-foreground">
                Custom endpoint for OpenAI-compatible APIs
              </p>
            </div>

            {/* Web Search */}
            <div className="flex items-center justify-between py-2 border-t border-[var(--border-subtle)]">
              <div>
                <div className="text-sm font-medium text-foreground">Web Search</div>
                <div className="text-xs text-muted-foreground">
                  Enable OpenAI&apos;s native web search tool
                </div>
              </div>
              <Switch
                checked={settings.openai.enable_web_search}
                onCheckedChange={(checked) =>
                  updateProvider("openai", "enable_web_search", checked)
                }
              />
            </div>

            {settings.openai.enable_web_search && (
              <div className="space-y-2">
                <label htmlFor="openai-search-context" className="text-sm text-foreground">
                  Search Context Size
                </label>
                <Select
                  value={settings.openai.web_search_context_size}
                  onValueChange={(value: WebSearchContextSize) =>
                    updateProvider("openai", "web_search_context_size", value)
                  }
                >
                  <SelectTrigger id="openai-search-context">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="low">Low (faster, cheaper)</SelectItem>
                    <SelectItem value="medium">Medium (balanced)</SelectItem>
                    <SelectItem value="high">High (more thorough)</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  Controls how much web content is retrieved for context
                </p>
              </div>
            )}
          </div>
        );

      case "openrouter":
        return (
          <div className="space-y-3">
            <div className="space-y-2">
              <label htmlFor="openrouter-key" className="text-sm text-foreground">
                API Key
              </label>
              <PasswordInput
                id="openrouter-key"
                value={settings.openrouter.api_key || ""}
                onChange={(value) => updateProvider("openrouter", "api_key", value)}
                placeholder="sk-or-v1-..."
              />
              <p className="text-xs text-muted-foreground">Get your API key from openrouter.ai</p>
            </div>

            <OpenRouterProviderPreferencesSection
              settings={settings}
              onChange={onChange}
            />
          </div>
        );

      case "ollama":
        return (
          <div className="space-y-2">
            <label htmlFor="ollama-url" className="text-sm text-foreground">
              Base URL
            </label>
            <Input
              id="ollama-url"
              value={settings.ollama.base_url}
              onChange={(e) => updateProvider("ollama", "base_url", e.target.value)}
              placeholder="http://localhost:11434"
            />
            <p className="text-xs text-muted-foreground">
              Ollama server endpoint (default: http://localhost:11434)
            </p>
          </div>
        );

      case "gemini":
        return (
          <div className="space-y-2">
            <label htmlFor="gemini-key" className="text-sm text-foreground">
              API Key
            </label>
            <PasswordInput
              id="gemini-key"
              value={settings.gemini.api_key || ""}
              onChange={(value) => updateProvider("gemini", "api_key", value)}
              placeholder="AIza..."
            />
            <p className="text-xs text-muted-foreground">
              Get your API key from aistudio.google.com
            </p>
          </div>
        );

      case "groq":
        return (
          <div className="space-y-2">
            <label htmlFor="groq-key" className="text-sm text-foreground">
              API Key
            </label>
            <PasswordInput
              id="groq-key"
              value={settings.groq.api_key || ""}
              onChange={(value) => updateProvider("groq", "api_key", value)}
              placeholder="gsk_..."
            />
            <p className="text-xs text-muted-foreground">Get your API key from console.groq.com</p>
          </div>
        );

      case "xai":
        return (
          <div className="space-y-2">
            <label htmlFor="xai-key" className="text-sm text-foreground">
              API Key
            </label>
            <PasswordInput
              id="xai-key"
              value={settings.xai.api_key || ""}
              onChange={(value) => updateProvider("xai", "api_key", value)}
              placeholder="xai-..."
            />
            <p className="text-xs text-muted-foreground">Get your API key from x.ai</p>
          </div>
        );

      case "zai_sdk":
        return (
          <div className="space-y-4">
            <div className="space-y-2">
              <label htmlFor="z-ai-sdk-key" className="text-sm text-foreground">
                API Key
              </label>
              <PasswordInput
                id="z-ai-sdk-key"
                value={settings.zai_sdk?.api_key || ""}
                onChange={(value) => updateProvider("zai_sdk", "api_key", value)}
                placeholder="your-zai-api-key"
              />
              <p className="text-xs text-muted-foreground">Get your API key from z.ai</p>
            </div>

            <div className="space-y-2">
              <label htmlFor="z-ai-sdk-base" className="text-sm text-foreground">
                Base URL <span className="text-muted-foreground font-normal">(optional)</span>
              </label>
              <Input
                id="z-ai-sdk-base"
                value={settings.zai_sdk?.base_url || ""}
                onChange={(e) => updateProvider("zai_sdk", "base_url", e.target.value)}
                placeholder="https://open.bigmodel.cn/api/paas/v4"
              />
              <p className="text-xs text-muted-foreground">Custom endpoint for Z.AI SDK API</p>
            </div>
          </div>
        );

      case "nvidia":
        return (
          <div className="space-y-4">
            <div className="space-y-2">
              <label htmlFor="nvidia-key" className="text-sm text-foreground">
                API Key
              </label>
              <PasswordInput
                id="nvidia-key"
                value={settings.nvidia?.api_key || ""}
                onChange={(value) => updateProvider("nvidia", "api_key", value)}
                placeholder="nvapi-..."
              />
              <p className="text-xs text-muted-foreground">
                Get your API key from{" "}
                <a
                  href="https://build.nvidia.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="underline"
                >
                  build.nvidia.com
                </a>
              </p>
            </div>

            <div className="space-y-2">
              <label htmlFor="nvidia-base" className="text-sm text-foreground">
                Base URL <span className="text-muted-foreground font-normal">(optional)</span>
              </label>
              <Input
                id="nvidia-base"
                value={settings.nvidia?.base_url || ""}
                onChange={(e) => updateProvider("nvidia", "base_url", e.target.value)}
                placeholder="https://integrate.api.nvidia.com/v1"
              />
              <p className="text-xs text-muted-foreground">
                Custom endpoint (defaults to NVIDIA NIM API)
              </p>
            </div>
          </div>
        );

      default:
        return null;
    }
  };

  const getShowInSelector = (providerId: ProviderConfig["id"]): boolean => {
    const providerSettings = settings[providerId];
    if (typeof providerSettings === "object" && "show_in_selector" in providerSettings) {
      return providerSettings.show_in_selector;
    }
    return true;
  };

  return (
    <div className="flex flex-col" style={{ height: "calc(100vh - 140px)" }}>
      {/* Top: left-right split */}
      <div className="flex gap-4 flex-1 min-h-0">
        {/* Left: provider list */}
        <div className="w-56 flex-shrink-0 overflow-y-auto space-y-0.5 pr-1">
          {providers.map((provider) => {
            const isConfigured = provider.getConfigured(settings);
            const isDefault = settings.default_provider === provider.id;
            const isSelected = selectedId === provider.id;

            return (
              <button key={provider.id} type="button"
                onClick={() => setSelectedId(isSelected ? null : provider.id)}
                className={cn(
                  "w-full flex items-center gap-2.5 px-3 py-2 rounded-lg text-left transition-colors",
                  isSelected ? "bg-accent/10 text-foreground" : "hover:bg-[var(--bg-hover)] text-foreground/70"
                )}
              >
                <span className="text-sm w-6 h-6 flex items-center justify-center flex-shrink-0">{provider.icon}</span>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="text-[12px] font-medium truncate">{provider.name}</span>
                    {isDefault && <Star className="w-2.5 h-2.5 text-accent fill-current flex-shrink-0" />}
                  </div>
                </div>
                <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0", isConfigured ? "bg-emerald-400" : "bg-muted-foreground/20")} />
              </button>
            );
          })}
        </div>

        {/* Right: config panel */}
        <div className="flex-1 border-l border-border/15 pl-4 overflow-y-auto">
          {selectedProvider ? (
            <div className="space-y-5">
              {/* Header */}
              <div className="flex items-center gap-3">
                <span className="text-xl">{selectedProvider.icon}</span>
                <div>
                  <div className="text-[14px] font-medium text-foreground">{selectedProvider.name}</div>
                  <div className="text-[11px] text-muted-foreground/50">{selectedProvider.description}</div>
                </div>
              </div>

              {/* Show in selector toggle */}
              <div className="flex items-center justify-between py-2.5 border-y border-border/15">
                <div>
                  <div className="text-[12px] font-medium text-foreground/80">{t("provider.showInSelector")}</div>
                  <div className="text-[11px] text-muted-foreground/40">{t("provider.showInSelectorDesc")}</div>
                </div>
                <Switch
                  checked={getShowInSelector(selectedProvider.id)}
                  onCheckedChange={(checked) => updateProvider(selectedProvider.id, "show_in_selector", checked)}
                />
              </div>

              {/* Provider fields */}
              {renderProviderFields(selectedProvider)}
            </div>
          ) : (
            <div className="h-full flex items-center justify-center">
              <span className="text-[12px] text-muted-foreground/30">{t("provider.selectProvider")}</span>
            </div>
          )}
        </div>
      </div>

      {/* Bottom: default model */}
      <div className="pt-4 mt-4 border-t border-border/20 flex-shrink-0">
        <div className="text-[12px] font-medium text-foreground/70 mb-2">{t("provider.defaultModel")}</div>
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
        <p className="text-[11px] text-muted-foreground/40 mt-1.5">{t("provider.defaultModelDesc")}</p>
      </div>
    </div>
  );
}
