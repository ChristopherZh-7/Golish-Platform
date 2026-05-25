import { Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { CustomSelect } from "@/components/ui/custom-select";
import { Input } from "@/components/ui/input";
import type { AiProvider, SubAgentModelConfig } from "@/lib/settings";

export const PROVIDER_OPTIONS: { value: AiProvider; label: string }[] = [
  { value: "vertex_ai", label: "Vertex AI (Claude)" },
  { value: "vertex_gemini", label: "Vertex AI (Gemini)" },
  { value: "anthropic", label: "Anthropic" },
  { value: "openai", label: "OpenAI" },
  { value: "openrouter", label: "OpenRouter" },
  { value: "gemini", label: "Gemini" },
  { value: "groq", label: "Groq" },
  { value: "ollama", label: "Ollama" },
  { value: "xai", label: "xAI (Grok)" },
  { value: "zai_sdk", label: "Z.AI SDK" },
  { value: "nvidia", label: "NVIDIA NIM" },
  { value: "deepseek", label: "DeepSeek" },
];

export const MODEL_SUGGESTIONS: Record<AiProvider, string[]> = {
  vertex_ai: [
    "claude-sonnet-4-6@default",
    "claude-opus-4-5@20251101",
    "claude-sonnet-4-5@20250929",
    "claude-haiku-4-5@20251001",
  ],
  vertex_gemini: ["gemini-2.5-pro-preview-05-06", "gemini-2.5-flash-preview-04-17"],
  anthropic: [
    "claude-sonnet-4-6-20260217",
    "claude-opus-4-5-20251101",
    "claude-sonnet-4-5-20250929",
    "claude-haiku-4-5-20251001",
  ],
  openai: ["gpt-4o", "gpt-4o-mini", "o3", "o3-mini", "gpt-5"],
  openrouter: [
    "anthropic/claude-opus-4.5",
    "anthropic/claude-sonnet-4.5",
    "openai/gpt-4o",
    "google/gemini-2.5-pro",
  ],
  gemini: ["gemini-2.5-pro", "gemini-2.5-flash", "gemini-3-pro-preview"],
  groq: ["llama-3.3-70b-versatile", "llama-3.1-8b-instant"],
  ollama: ["llama3.2", "codellama", "mistral"],
  xai: ["grok-4-1-fast-reasoning", "grok-4-1-fast-non-reasoning"],
  zai_sdk: ["glm-4.7", "glm-4.6v", "glm-4.5-air", "glm-4-flash"],
  nvidia: [
    "nvidia/nemotron-3-super-120b-a12b",
    "nvidia/nemotron-3-nano-30b-a3b",
    "nvidia/llama-3.3-nemotron-super-49b-v1.5",
    "nvidia/nvidia-nemotron-nano-9b-v2",
    "qwen/qwen3-coder-480b-a35b-instruct",
    "mistralai/mistral-small-4-119b-2603",
    "mistralai/mistral-medium-3.5-128b",
    "deepseek-ai/deepseek-v4-flash",
    "deepseek-ai/deepseek-v3.1-terminus",
    "moonshotai/kimi-k2.6",
    "z-ai/glm-5.1",
    "minimaxai/minimax-m2.7",
    "google/gemma-4-31b-it",
  ],
  deepseek: ["deepseek-v4-flash", "deepseek-v4-pro"],
};

interface ModelOverridePanelProps {
  agentId: string;
  modelConfig: SubAgentModelConfig;
  hasOverride: boolean;
  onUpdate: (config: SubAgentModelConfig | null) => void;
}

export function ModelOverridePanel({
  agentId,
  modelConfig,
  hasOverride,
  onUpdate,
}: ModelOverridePanelProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2 p-3 rounded bg-background border border-[var(--border-medium)]">
      <div className="flex items-center justify-between">
        <span className="text-[10px] text-muted-foreground uppercase tracking-wider">
          {t("subAgentSettings.runtimeModelOverride")}
        </span>
        {hasOverride && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onUpdate(null)}
            className="h-6 px-2 text-muted-foreground hover:text-destructive"
          >
            <Trash2 className="w-3 h-3" />
          </Button>
        )}
      </div>
      <div className="grid grid-cols-2 gap-2">
        <CustomSelect
          value={modelConfig.provider || ""}
          onChange={(value) =>
            onUpdate({
              ...modelConfig,
              provider: value as AiProvider,
              model: value !== modelConfig.provider ? undefined : modelConfig.model,
            })
          }
          options={PROVIDER_OPTIONS}
          placeholder={t("subAgentSettings.useDefault")}
        />
        {modelConfig.provider ? (
          <div className="relative">
            <Input
              value={modelConfig.model || ""}
              onChange={(e) => onUpdate({ ...modelConfig, model: e.target.value })}
              placeholder={t("subAgentSettings.enterModelName")}
              list={`override-${agentId}-models`}
              className="bg-background border-border h-9 text-xs"
            />
            <datalist id={`override-${agentId}-models`}>
              {(MODEL_SUGGESTIONS[modelConfig.provider] || []).map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </div>
        ) : (
          <Input
            disabled
            placeholder={t("subAgentSettings.selectProviderFirst")}
            className="bg-muted border-border h-9 text-xs"
          />
        )}
      </div>
      {hasOverride && (
        <p className="text-[10px] text-[var(--success)]">
          {t("subAgentSettings.runtimeOverride", {
            provider: modelConfig.provider,
            model: modelConfig.model,
          })}
        </p>
      )}
    </div>
  );
}
