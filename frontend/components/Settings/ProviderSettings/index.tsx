import { ChevronDown, ExternalLink, Star } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { AiSettings, WebSearchContextSize } from "@/lib/settings";
import { cn } from "@/lib/utils";
import { type ProviderConfig, useProviderForm } from "../hooks/useProviderForm";
import { ModelSelector } from "../ModelSelector";
import { PasswordInput } from "./ApiKeyInput";
import { OpenRouterProviderPreferencesSection } from "./ProviderForm";

interface ProviderSettingsProps {
  settings: AiSettings;
  onChange: (settings: AiSettings) => void;
}

export function ProviderSettings({ settings, onChange }: ProviderSettingsProps) {
  const { t } = useTranslation();
  const {
    selectedId,
    setSelectedId,
    configuredProviders,
    unconfiguredProviders,
    updateProvider,
    updateOpenRouterPref,
    getShowInSelector,
    getColor,
  } = useProviderForm(settings, onChange);

  const renderProviderFields = (provider: ProviderConfig) => {
    const fieldLabel = "text-[11px] font-medium text-foreground/70";
    const fieldHint = "text-[10px] text-muted-foreground/35 leading-relaxed";
    const fieldInput =
      "text-[12px] bg-foreground/[0.03] border-foreground/[0.06] focus:border-accent/40 focus:ring-accent/10";
    const fieldLink =
      "text-accent/60 hover:text-accent transition-colors inline-flex items-center gap-0.5";

    switch (provider.id) {
      case "vertex_ai":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="vertex-credentials" className={fieldLabel}>
                {t("provider.credentialsPath")}
              </label>
              <Input
                id="vertex-credentials"
                value={settings.vertex_ai.credentials_path || ""}
                onChange={(e) => updateProvider("vertex_ai", "credentials_path", e.target.value)}
                placeholder="/path/to/service-account.json"
                className={cn(fieldInput, "font-mono")}
              />
              <p className={fieldHint}>{t("provider.googleServiceAccountHint")}</p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-project" className={fieldLabel}>
                {t("provider.projectId")}
              </label>
              <Input
                id="vertex-project"
                value={settings.vertex_ai.project_id || ""}
                onChange={(e) => updateProvider("vertex_ai", "project_id", e.target.value)}
                placeholder="your-gcp-project-id"
                className={fieldInput}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-location" className={fieldLabel}>
                {t("provider.location")}
              </label>
              <Input
                id="vertex-location"
                value={settings.vertex_ai.location || ""}
                onChange={(e) => updateProvider("vertex_ai", "location", e.target.value)}
                placeholder="us-east5"
                className={fieldInput}
              />
              <p className={fieldHint}>{t("provider.regionHintVertex")}</p>
            </div>
          </div>
        );

      case "vertex_gemini":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="vertex-gemini-credentials" className={fieldLabel}>
                {t("provider.credentialsPath")}
              </label>
              <Input
                id="vertex-gemini-credentials"
                value={settings.vertex_gemini.credentials_path || ""}
                onChange={(e) =>
                  updateProvider("vertex_gemini", "credentials_path", e.target.value)
                }
                placeholder="/path/to/service-account.json"
                className={cn(fieldInput, "font-mono")}
              />
              <p className={fieldHint}>{t("provider.googleServiceAccountHint")}</p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-gemini-project" className={fieldLabel}>
                {t("provider.projectId")}
              </label>
              <Input
                id="vertex-gemini-project"
                value={settings.vertex_gemini.project_id || ""}
                onChange={(e) => updateProvider("vertex_gemini", "project_id", e.target.value)}
                placeholder="your-gcp-project-id"
                className={fieldInput}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="vertex-gemini-location" className={fieldLabel}>
                {t("provider.location")}
              </label>
              <Input
                id="vertex-gemini-location"
                value={settings.vertex_gemini.location || ""}
                onChange={(e) => updateProvider("vertex_gemini", "location", e.target.value)}
                placeholder="us-central1"
                className={fieldInput}
              />
              <p className={fieldHint}>{t("provider.regionHintGemini")}</p>
            </div>
          </div>
        );

      case "anthropic":
        return (
          <div className="space-y-1.5">
            <label htmlFor="anthropic-key" className={fieldLabel}>
              {t("provider.apiKey")}
            </label>
            <PasswordInput
              id="anthropic-key"
              value={settings.anthropic.api_key || ""}
              onChange={(value) => updateProvider("anthropic", "api_key", value)}
              placeholder="sk-ant-api03-..."
            />
            <p className={fieldHint}>
              {t("provider.from")}{" "}
              <a
                href="https://console.anthropic.com"
                target="_blank"
                rel="noopener noreferrer"
                className={fieldLink}
              >
                console.anthropic.com <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "openai":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="openai-key" className={fieldLabel}>
                {t("provider.apiKey")}
              </label>
              <PasswordInput
                id="openai-key"
                value={settings.openai.api_key || ""}
                onChange={(value) => updateProvider("openai", "api_key", value)}
                placeholder="sk-..."
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="openai-base" className={fieldLabel}>
                {t("provider.baseUrl")}{" "}
                <span className="text-muted-foreground/30 font-normal">
                  ({t("provider.optional")})
                </span>
              </label>
              <Input
                id="openai-base"
                value={settings.openai.base_url || ""}
                onChange={(e) => updateProvider("openai", "base_url", e.target.value)}
                placeholder="https://api.openai.com/v1"
                className={fieldInput}
              />
              <p className={fieldHint}>{t("provider.openAiCompatibleEndpoint")}</p>
            </div>
            <div className="flex items-center justify-between py-2.5 border-t border-foreground/[0.04]">
              <div>
                <div className="text-[12px] font-medium text-foreground/80">
                  {t("provider.webSearch")}
                </div>
                <div className="text-[10px] text-muted-foreground/35">
                  {t("provider.nativeWebSearchTool")}
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
              <div className="space-y-1.5">
                <label htmlFor="openai-search-context" className={fieldLabel}>
                  {t("provider.searchContextSize")}
                </label>
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
                    <SelectItem value="low">{t("provider.searchContext.low")}</SelectItem>
                    <SelectItem value="medium">{t("provider.searchContext.medium")}</SelectItem>
                    <SelectItem value="high">{t("provider.searchContext.high")}</SelectItem>
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
              <label htmlFor="openrouter-key" className={fieldLabel}>
                {t("provider.apiKey")}
              </label>
              <PasswordInput
                id="openrouter-key"
                value={settings.openrouter.api_key || ""}
                onChange={(value) => updateProvider("openrouter", "api_key", value)}
                placeholder="sk-or-v1-..."
              />
              <p className={fieldHint}>
                {t("provider.from")}{" "}
                <a
                  href="https://openrouter.ai"
                  target="_blank"
                  rel="noopener noreferrer"
                  className={fieldLink}
                >
                  openrouter.ai <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="h-px bg-foreground/[0.04]" />
            <OpenRouterProviderPreferencesSection
              settings={settings}
              updatePref={updateOpenRouterPref}
            />
          </div>
        );

      case "ollama":
        return (
          <div className="space-y-1.5">
            <label htmlFor="ollama-url" className={fieldLabel}>
              {t("provider.baseUrl")}
            </label>
            <Input
              id="ollama-url"
              value={settings.ollama.base_url}
              onChange={(e) => updateProvider("ollama", "base_url", e.target.value)}
              placeholder="http://localhost:11434"
              className={cn(fieldInput, "font-mono")}
            />
            <p className={fieldHint}>{t("provider.ollamaEndpoint")}</p>
          </div>
        );

      case "gemini":
        return (
          <div className="space-y-1.5">
            <label htmlFor="gemini-key" className={fieldLabel}>
              {t("provider.apiKey")}
            </label>
            <PasswordInput
              id="gemini-key"
              value={settings.gemini.api_key || ""}
              onChange={(value) => updateProvider("gemini", "api_key", value)}
              placeholder="AIza..."
            />
            <p className={fieldHint}>
              {t("provider.from")}{" "}
              <a
                href="https://aistudio.google.com"
                target="_blank"
                rel="noopener noreferrer"
                className={fieldLink}
              >
                aistudio.google.com <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "groq":
        return (
          <div className="space-y-1.5">
            <label htmlFor="groq-key" className={fieldLabel}>
              {t("provider.apiKey")}
            </label>
            <PasswordInput
              id="groq-key"
              value={settings.groq.api_key || ""}
              onChange={(value) => updateProvider("groq", "api_key", value)}
              placeholder="gsk_..."
            />
            <p className={fieldHint}>
              {t("provider.from")}{" "}
              <a
                href="https://console.groq.com"
                target="_blank"
                rel="noopener noreferrer"
                className={fieldLink}
              >
                console.groq.com <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "xai":
        return (
          <div className="space-y-1.5">
            <label htmlFor="xai-key" className={fieldLabel}>
              {t("provider.apiKey")}
            </label>
            <PasswordInput
              id="xai-key"
              value={settings.xai.api_key || ""}
              onChange={(value) => updateProvider("xai", "api_key", value)}
              placeholder="xai-..."
            />
            <p className={fieldHint}>
              {t("provider.from")}{" "}
              <a
                href="https://x.ai"
                target="_blank"
                rel="noopener noreferrer"
                className={fieldLink}
              >
                x.ai <ExternalLink className="w-2.5 h-2.5" />
              </a>
            </p>
          </div>
        );

      case "zai_sdk":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="z-ai-sdk-key" className={fieldLabel}>
                {t("provider.apiKey")}
              </label>
              <PasswordInput
                id="z-ai-sdk-key"
                value={settings.zai_sdk?.api_key || ""}
                onChange={(value) => updateProvider("zai_sdk", "api_key", value)}
                placeholder="your-zai-api-key"
              />
              <p className={fieldHint}>
                {t("provider.from")}{" "}
                <a
                  href="https://open.bigmodel.cn"
                  target="_blank"
                  rel="noopener noreferrer"
                  className={fieldLink}
                >
                  open.bigmodel.cn <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="z-ai-sdk-base" className={fieldLabel}>
                {t("provider.baseUrl")}{" "}
                <span className="text-muted-foreground/30 font-normal">
                  ({t("provider.optional")})
                </span>
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
              <label htmlFor="nvidia-key" className={fieldLabel}>
                {t("provider.apiKey")}
              </label>
              <PasswordInput
                id="nvidia-key"
                value={settings.nvidia?.api_key || ""}
                onChange={(value) => updateProvider("nvidia", "api_key", value)}
                placeholder="nvapi-..."
              />
              <p className={fieldHint}>
                {t("provider.from")}{" "}
                <a
                  href="https://build.nvidia.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className={fieldLink}
                >
                  build.nvidia.com <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="nvidia-base" className={fieldLabel}>
                {t("provider.baseUrl")}{" "}
                <span className="text-muted-foreground/30 font-normal">
                  ({t("provider.optional")})
                </span>
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

      case "deepseek":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="deepseek-key" className={fieldLabel}>
                {t("provider.apiKey")}
              </label>
              <PasswordInput
                id="deepseek-key"
                value={settings.deepseek?.api_key || ""}
                onChange={(value) => updateProvider("deepseek", "api_key", value)}
                placeholder="sk-..."
              />
              <p className={fieldHint}>
                {t("provider.from")}{" "}
                <a
                  href="https://platform.deepseek.com/api_keys"
                  target="_blank"
                  rel="noopener noreferrer"
                  className={fieldLink}
                >
                  platform.deepseek.com <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="deepseek-base" className={fieldLabel}>
                {t("provider.baseUrl")}{" "}
                <span className="text-muted-foreground/30 font-normal">
                  ({t("provider.optional")})
                </span>
              </label>
              <Input
                id="deepseek-base"
                value={settings.deepseek?.base_url || ""}
                onChange={(e) => updateProvider("deepseek", "base_url", e.target.value)}
                placeholder="https://api.deepseek.com"
                className={cn(fieldInput, "font-mono")}
              />
              <p className={fieldHint}>{t("provider.deepSeekEndpoint")}</p>
            </div>
          </div>
        );

      case "xiaomi":
        return (
          <div className="space-y-3.5">
            <div className="space-y-1.5">
              <label htmlFor="xiaomi-key" className={fieldLabel}>
                {t("provider.apiKey")}
              </label>
              <PasswordInput
                id="xiaomi-key"
                value={settings.xiaomi?.api_key || ""}
                onChange={(value) => updateProvider("xiaomi", "api_key", value)}
                placeholder="tp-... (Token Plan)  |  sk-... (Pay-as-you-go)"
              />
              <p className={fieldHint}>
                {t("provider.from")}{" "}
                <a
                  href="https://platform.xiaomimimo.com/"
                  target="_blank"
                  rel="noopener noreferrer"
                  className={fieldLink}
                >
                  platform.xiaomimimo.com <ExternalLink className="w-2.5 h-2.5" />
                </a>
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="xiaomi-region" className={fieldLabel}>
                Region
              </label>
              <Select
                value={settings.xiaomi?.region || "cn"}
                onValueChange={(value) => updateProvider("xiaomi", "region", value)}
              >
                <SelectTrigger id="xiaomi-region" className={fieldInput}>
                  <SelectValue placeholder="cn" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="cn">Token Plan · 中国 (cn)</SelectItem>
                  <SelectItem value="sgp">Token Plan · 新加坡 (sgp)</SelectItem>
                  <SelectItem value="ams">Token Plan · 欧洲 (ams)</SelectItem>
                  <SelectItem value="payg">按量付费 (api.xiaomimimo.com)</SelectItem>
                </SelectContent>
              </Select>
              <p className={fieldHint}>
                Token Plan key (tp-) 选对应区域；按量付费 key (sk-) 选「按量付费」自动指向
                api.xiaomimimo.com。
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="xiaomi-protocol" className={fieldLabel}>
                Default protocol
              </label>
              <Select
                value={settings.xiaomi?.default_protocol || "auto"}
                onValueChange={(value) => updateProvider("xiaomi", "default_protocol", value)}
              >
                <SelectTrigger id="xiaomi-protocol" className={fieldInput}>
                  <SelectValue placeholder="auto" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="auto">Auto (按模型 ID 后缀决定)</SelectItem>
                  <SelectItem value="openai">OpenAI 兼容 (Chat Completions)</SelectItem>
                  <SelectItem value="anthropic">Anthropic 兼容 (Messages)</SelectItem>
                </SelectContent>
              </Select>
              <p className={fieldHint}>
                model id 加 `@anthropic` / `@openai` 后缀可强制切换协议（如
                `mimo-v2.5-pro@anthropic`）。
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="xiaomi-openai-base" className={fieldLabel}>
                {t("provider.baseUrl")} (OpenAI){" "}
                <span className="text-muted-foreground/30 font-normal">
                  ({t("provider.optional")})
                </span>
              </label>
              <Input
                id="xiaomi-openai-base"
                value={settings.xiaomi?.openai_base_url || ""}
                onChange={(e) => updateProvider("xiaomi", "openai_base_url", e.target.value)}
                placeholder="https://token-plan-cn.xiaomimimo.com/v1"
                className={cn(fieldInput, "font-mono")}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="xiaomi-anthropic-base" className={fieldLabel}>
                {t("provider.baseUrl")} (Anthropic){" "}
                <span className="text-muted-foreground/30 font-normal">
                  ({t("provider.optional")})
                </span>
              </label>
              <Input
                id="xiaomi-anthropic-base"
                value={settings.xiaomi?.anthropic_base_url || ""}
                onChange={(e) => updateProvider("xiaomi", "anthropic_base_url", e.target.value)}
                placeholder="https://token-plan-cn.xiaomimimo.com/anthropic"
                className={cn(fieldInput, "font-mono")}
              />
              <p className={fieldHint}>留空时按 Region 自动推导；前缀到 `/anthropic` 即可。</p>
            </div>
          </div>
        );

      default:
        return null;
    }
  };

  return (
    <div className="overflow-y-auto space-y-8 pb-10" style={{ height: "calc(100vh - 140px)" }}>
      <div className="rounded-xl border border-foreground/[0.05] bg-foreground/[0.015] p-5">
        <div className="text-[13px] font-semibold text-foreground/90 mb-1">
          {t("provider.defaultModel")}
        </div>
        <p className="text-[11px] text-muted-foreground/40 mb-4 leading-relaxed">
          {t("provider.defaultModelDesc")}
        </p>
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
                  <div
                    className="absolute left-0 top-2 bottom-2 w-[2px] rounded-full transition-opacity duration-200"
                    style={{ backgroundColor: color.border, opacity: isOpen ? 1 : 0.6 }}
                  />
                  <button
                    type="button"
                    onClick={() => setSelectedId(isOpen ? null : provider.id)}
                    className="w-full flex items-center gap-3.5 pl-4 pr-3.5 py-3 text-left"
                  >
                    <div
                      className="w-8 h-8 rounded-lg flex items-center justify-center text-[13px] flex-shrink-0 transition-transform duration-200 hover:scale-105"
                      style={{ backgroundColor: color.bg }}
                    >
                      {provider.icon}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="text-[13px] font-medium text-foreground/90">
                        {provider.name}
                      </div>
                      <div className="text-[10px] text-muted-foreground/35 mt-0.5 truncate">
                        {provider.description}
                      </div>
                    </div>
                    <div className="flex items-center gap-2.5 flex-shrink-0">
                      {isDefault && <Star className="w-3.5 h-3.5 text-accent fill-accent/50" />}
                      <div
                        className="w-2 h-2 rounded-full"
                        style={{ backgroundColor: color.dot, boxShadow: `0 0 6px ${color.dot}40` }}
                      />
                      <ChevronDown
                        className={cn(
                          "w-3.5 h-3.5 text-muted-foreground/25 transition-transform duration-200",
                          isOpen && "rotate-180"
                        )}
                      />
                    </div>
                  </button>
                  {isOpen && (
                    <div className="px-4 pb-4 pt-1">
                      <div className="rounded-lg bg-foreground/[0.02] border border-foreground/[0.04] p-4 space-y-4">
                        <div className="flex items-center justify-between">
                          <span className="text-[11px] text-muted-foreground/50">
                            {t("provider.showInSelector")}
                          </span>
                          <Switch
                            checked={getShowInSelector(provider.id)}
                            onCheckedChange={(checked) =>
                              updateProvider(provider.id, "show_in_selector", checked)
                            }
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
                    <div
                      className="w-8 h-8 rounded-lg flex items-center justify-center text-[13px] flex-shrink-0 opacity-40 group-hover:opacity-60 transition-opacity"
                      style={{ backgroundColor: color.bg }}
                    >
                      {provider.icon}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="text-[13px] font-medium text-foreground/45 group-hover:text-foreground/70 transition-colors">
                        {provider.name}
                      </div>
                      <div className="text-[10px] text-muted-foreground/25 mt-0.5 truncate">
                        {provider.description}
                      </div>
                    </div>
                    <ChevronDown
                      className={cn(
                        "w-3.5 h-3.5 text-muted-foreground/15 group-hover:text-muted-foreground/30 transition-all duration-200",
                        isOpen && "rotate-180"
                      )}
                    />
                  </button>
                  {isOpen && (
                    <div className="px-4 pb-4 pt-1">
                      <div className="rounded-lg bg-foreground/[0.02] border border-foreground/[0.04] p-4 space-y-4">
                        <div className="flex items-center justify-between">
                          <span className="text-[11px] text-muted-foreground/50">
                            {t("provider.showInSelector")}
                          </span>
                          <Switch
                            checked={getShowInSelector(provider.id)}
                            onCheckedChange={(checked) =>
                              updateProvider(provider.id, "show_in_selector", checked)
                            }
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
