import { ChevronDown, Cpu } from "lucide-react";
import { memo } from "react";
import { useTranslation } from "react-i18next";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { PROVIDER_GROUPS } from "@/lib/models";
import { cn } from "@/lib/utils";

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

export const ChatModelSelector = memo(function ChatModelSelector({
  modelDisplay,
  currentModel,
  currentProvider,
  configuredProviders,
  onModelSelect,
}: ChatModelSelectorProps) {
  const { t } = useTranslation();
  const filtered = getVisibleProviderGroups(configuredProviders, currentProvider);

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex items-center gap-1 px-2 py-1 rounded-md text-[11px] text-accent hover:bg-[var(--bg-hover)] transition-colors"
        >
          <Cpu className="w-3 h-3" />
          {modelDisplay}
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
  );
});
