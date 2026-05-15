import { Brain, ChevronDown } from "lucide-react";
import { memo, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  modelOverrideKey,
  subscribeToModelOverrideChanges,
} from "@/lib/ai/model-overrides";
import { PROVIDER_GROUPS } from "@/lib/models";
import { getSettingsCached } from "@/lib/settings";
import { cn } from "@/lib/utils";

import { ModelSettingsPopover } from "./ModelSettingsPopover";

interface ChatModelSelectorProps {
  modelDisplay: string;
  currentModel: string;
  currentProvider: string;
  configuredProviders: Set<string>;
  onModelSelect: (modelId: string, provider: string) => void;
}

export function getVisibleProviderGroups(
  configuredProviders: Set<string>,
  currentProvider: string
): typeof PROVIDER_GROUPS {
  const filtered = PROVIDER_GROUPS.filter((g) => configuredProviders.has(g.provider));
  const selectedProvider = currentProvider === "anthropic_vertex" ? "vertex_ai" : currentProvider;
  const selectedIndex = filtered.findIndex((g) => g.provider === selectedProvider);

  if (selectedIndex <= 0) return filtered;

  return [
    filtered[selectedIndex],
    ...filtered.slice(0, selectedIndex),
    ...filtered.slice(selectedIndex + 1),
  ];
}

export function getModelItemClassName(isSelected: boolean): string {
  return cn(
    "text-xs cursor-pointer",
    isSelected
      ? "bg-accent/20 text-foreground ring-1 ring-accent/35 focus:bg-accent/25 focus:text-foreground"
      : "text-foreground/90 hover:bg-[var(--bg-hover)]/80 hover:text-foreground"
  );
}

function modelIsThinkingByDefault(provider: string, model: string): boolean {
  const m = model.toLowerCase();
  if (provider === "anthropic" || provider === "vertex_ai") return true;
  if (provider === "openai") {
    return (
      m.startsWith("o") || m.startsWith("gpt-5") || m.includes("codex")
    );
  }
  if (provider === "nvidia" || provider === "openrouter" || provider === "zai_sdk") {
    return (
      m.includes("kimi-k2-thinking") ||
      m.includes("deepseek-r1") ||
      m.includes("deepseek-v3.2") ||
      m.includes("phi-4-mini-flash-reasoning") ||
      m.includes("step-3.5-flash") ||
      m.includes("qwq") ||
      (m.includes("qwen3") && m.includes("thinking")) ||
      m.includes("glm-4.7")
    );
  }
  return false;
}

function useEffectiveThinkingEnabled(
  provider: string,
  model: string
): boolean {
  const [enabled, setEnabled] = useState(() =>
    modelIsThinkingByDefault(provider, model)
  );

  useEffect(() => {
    if (!provider || !model) return undefined;
    let cancelled = false;
    const key = modelOverrideKey(provider, model);

    const recompute = async () => {
      try {
        const settings = await getSettingsCached();
        const override = settings.ai.model_overrides?.[key];
        const next =
          override?.thinking !== undefined
            ? !!override.thinking
            : modelIsThinkingByDefault(provider, model);
        if (!cancelled) setEnabled(next);
      } catch (err) {
        console.warn("[ChatModelSelector] failed to read thinking state", err);
      }
    };

    void recompute();
    const unsubscribe = subscribeToModelOverrideChanges((changedKey) => {
      if (changedKey === key) void recompute();
    });

    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [provider, model]);

  return enabled;
}

export const ChatModelSelector = memo(function ChatModelSelector({
  modelDisplay,
  currentModel,
  currentProvider,
  configuredProviders,
  onModelSelect,
}: ChatModelSelectorProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const filtered = getVisibleProviderGroups(configuredProviders, currentProvider);
  const thinkingEnabled = useEffectiveThinkingEnabled(
    currentProvider,
    currentModel
  );

  return (
    <div className="flex items-center gap-1">
      <DropdownMenu modal={false} open={open} onOpenChange={setOpen}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex items-center gap-1 px-2 py-1 rounded-md text-[11px] text-accent hover:bg-[var(--bg-hover)] transition-colors"
          >
            {modelDisplay}
            {thinkingEnabled && (
              <Brain
                className="w-3 h-3 text-amber-400/85"
                aria-label={t("ai.thinkingEnabled", "Thinking enabled")}
              />
            )}
            <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          side="top"
          className="bg-card border-[var(--border-medium)] min-w-[200px] max-h-[400px] overflow-y-auto"
        >
          {filtered.length === 0 ? (
            <div className="px-3 py-4 text-center">
              <p className="text-xs text-muted-foreground">
                {t("ai.noProviders", "No providers configured")}
              </p>
              <p className="text-[10px] text-muted-foreground/60 mt-1">
                {t("ai.configureInSettings", "Configure API keys in Settings → Providers")}
              </p>
            </div>
          ) : (
            filtered.map((group, gi) => (
              <div key={group.provider}>
                {gi > 0 && <DropdownMenuSeparator />}
                <div className="px-2 py-1 text-[10px] text-muted-foreground uppercase tracking-wide">
                  {group.providerName}
                </div>
                {group.models.map((model) => {
                  const isSelected =
                    currentModel === model.id &&
                    (currentProvider === group.provider || currentProvider === "anthropic_vertex");
                  return (
                    <DropdownMenuItem
                      key={`${group.provider}-${model.id}-${model.reasoningEffort ?? ""}`}
                      onClick={() => onModelSelect(model.id, group.provider)}
                      className={getModelItemClassName(isSelected)}
                    >
                      {model.name}
                    </DropdownMenuItem>
                  );
                })}
              </div>
            ))
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {currentProvider && currentModel && (
        <ModelSettingsPopover
          provider={currentProvider}
          model={currentModel}
          modelLabel={modelDisplay}
          align="end"
          side="top"
        />
      )}
    </div>
  );
});
