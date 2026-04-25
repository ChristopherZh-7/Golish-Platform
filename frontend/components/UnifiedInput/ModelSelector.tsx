import { type JSX, memo, useCallback, useMemo } from "react";
import { Cpu } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useProviderSettings } from "@/hooks/useProviderSettings";
import {
  getOpenAiApiKey,
  getOpenRouterApiKey,
  initAiSession,
  type ProviderConfig,
  type ReasoningEffort,
  saveProjectModel,
} from "@/lib/ai";
import { logger } from "@/lib/logger";
import type { AiProvider } from "@/lib/ai";
import { formatModelName, getProviderGroup, getProviderGroupNested, type ModelEntry } from "@/lib/models";
import { notify } from "@/lib/notify";
import { cn } from "@/lib/utils";
import { useSessionAiConfig, useStore } from "@/store";

export type ModelProvider =
  | "vertex"
  | "vertex_gemini"
  | "openrouter"
  | "openai"
  | "anthropic"
  | "ollama"
  | "gemini"
  | "groq"
  | "xai"
  | "zai_sdk"
  | "nvidia";

function isAnyNestedSelected(
  entries: ModelEntry[],
  currentProvider: string,
  currentModel: string,
  currentReasoningEffort: ReasoningEffort | undefined,
  checkReasoningEffort: boolean
): boolean {
  return entries.some((e) => {
    if (e.id) {
      if (checkReasoningEffort) {
        return currentProvider === "openai" && currentModel === e.id && e.reasoningEffort === currentReasoningEffort;
      }
      return currentProvider === "vertex_gemini" && currentModel === e.id;
    }
    if (e.subModels) {
      return isAnyNestedSelected(e.subModels, currentProvider, currentModel, currentReasoningEffort, checkReasoningEffort);
    }
    return false;
  });
}

function renderModelEntry(
  entry: ModelEntry,
  keyPrefix: string,
  targetProvider: ModelProvider,
  currentProvider: string,
  currentModel: string,
  currentReasoningEffort: ReasoningEffort | undefined,
  checkReasoningEffort: boolean,
  credentials: unknown,
  onSelect: (modelId: string, provider: ModelProvider, reasoningEffort?: ReasoningEffort) => void
): JSX.Element | null {
  if (entry.subModels && entry.subModels.length > 0) {
    const isSubSelected = isAnyNestedSelected(entry.subModels, currentProvider, currentModel, currentReasoningEffort, checkReasoningEffort);
    return (
      <DropdownMenuSub key={`${keyPrefix}-${entry.name}`}>
        <DropdownMenuSubTrigger className={cn("text-xs cursor-pointer", isSubSelected ? "text-accent bg-[var(--accent-dim)]" : "text-foreground hover:text-accent")}>
          {entry.name}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent className="bg-card border-[var(--border-medium)]">
          {entry.subModels.map((sub) =>
            renderModelEntry(sub, `${keyPrefix}-${entry.name}`, targetProvider, currentProvider, currentModel, currentReasoningEffort, checkReasoningEffort, credentials, onSelect)
          )}
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    );
  }

  if (!entry.id) return null;
  const entryId = entry.id;
  const isSelected = checkReasoningEffort
    ? currentProvider === "openai" && currentModel === entryId && entry.reasoningEffort === currentReasoningEffort
    : currentProvider === targetProvider && currentModel === entryId;
  return (
    <DropdownMenuItem
      key={checkReasoningEffort ? `${keyPrefix}-${entryId}-${entry.reasoningEffort ?? "default"}` : `${keyPrefix}-${entryId}`}
      onClick={() => onSelect(entryId, targetProvider, entry.reasoningEffort)}
      disabled={!checkReasoningEffort && !credentials}
      className={cn("text-xs cursor-pointer", isSelected ? "text-accent bg-[var(--accent-dim)]" : "text-foreground hover:text-accent")}
    >
      {entry.name}
    </DropdownMenuItem>
  );
}

const PROVIDER_MAP: Record<ModelProvider, string> = {
  vertex: "anthropic_vertex",
  vertex_gemini: "vertex_gemini",
  openrouter: "openrouter",
  openai: "openai",
  anthropic: "anthropic",
  ollama: "ollama",
  gemini: "gemini",
  groq: "groq",
  xai: "xai",
  zai_sdk: "zai_sdk",
  nvidia: "nvidia",
};

interface ModelSelectorBadgeProps {
  sessionId: string;
}

export const ModelSelectorBadge = memo(function ModelSelectorBadge({ sessionId }: ModelSelectorBadgeProps) {
  const aiConfig = useSessionAiConfig(sessionId);
  const model = aiConfig?.model ?? "";
  const provider = aiConfig?.provider ?? "";
  const currentReasoningEffort = aiConfig?.reasoningEffort;
  const sessionWorkingDirectory = useStore((state) => state.sessions[sessionId]?.workingDirectory);
  const setSessionAiConfig = useStore((state) => state.setSessionAiConfig);

  const [providerSettings, refreshProviderSettings] = useProviderSettings();
  const { enabled: providerEnabled, apiKeys, vertexAiCredentials, vertexGeminiCredentials, visibility: providerVisibility } = providerSettings;

  const {
    showVertexAi, showVertexGemini, showOpenRouter, showOpenAi, showAnthropic,
    showOllama, showGemini, showGroq, showXai, showZaiSdk, showNvidia, hasVisibleProviders,
  } = useMemo(() => {
    const flags = {
      showVertexAi: providerVisibility.vertex_ai && providerEnabled.vertex_ai,
      showVertexGemini: providerVisibility.vertex_gemini && providerEnabled.vertex_gemini,
      showOpenRouter: providerVisibility.openrouter && providerEnabled.openrouter,
      showOpenAi: providerVisibility.openai && providerEnabled.openai,
      showAnthropic: providerVisibility.anthropic && providerEnabled.anthropic,
      showOllama: providerVisibility.ollama && providerEnabled.ollama,
      showGemini: providerVisibility.gemini && providerEnabled.gemini,
      showGroq: providerVisibility.groq && providerEnabled.groq,
      showXai: providerVisibility.xai && providerEnabled.xai,
      showZaiSdk: providerVisibility.zai_sdk && providerEnabled.zai_sdk,
      showNvidia: providerVisibility.nvidia && providerEnabled.nvidia,
    };
    return {
      ...flags,
      hasVisibleProviders: Object.values(flags).some(Boolean),
    };
  }, [providerVisibility, providerEnabled]);

  const handleModelSelect = useCallback(
    async (modelId: string, modelProvider: ModelProvider, reasoningEffort?: ReasoningEffort) => {
      if (model === modelId && provider === PROVIDER_MAP[modelProvider]) {
        if (modelProvider !== "openai" || reasoningEffort === currentReasoningEffort) return;
      }

      const modelName = formatModelName(modelId, reasoningEffort);
      const workspace = aiConfig?.vertexConfig?.workspace ?? sessionWorkingDirectory ?? ".";

      try {
        setSessionAiConfig(sessionId, { status: "initializing", model: modelId });

        let config: ProviderConfig;

        if (modelProvider === "vertex") {
          const vertexConfig = aiConfig?.vertexConfig;
          const credentials = vertexConfig
            ? { credentials_path: vertexConfig.credentialsPath, project_id: vertexConfig.projectId, location: vertexConfig.location }
            : vertexAiCredentials;
          if (!credentials?.credentials_path && !credentials?.project_id) throw new Error("Vertex AI credentials not configured");
          const credentialsPath = credentials.credentials_path ?? "";
          const projectId = credentials.project_id ?? "";
          const location = credentials.location ?? "us-east5";
          config = { provider: "vertex_ai", workspace, model: modelId, credentials_path: credentialsPath, project_id: projectId, location };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "anthropic_vertex", vertexConfig: { workspace, credentialsPath, projectId, location } });
        } else if (modelProvider === "vertex_gemini") {
          const credentials = vertexGeminiCredentials;
          if (!credentials?.credentials_path && !credentials?.project_id) throw new Error("Vertex Gemini credentials not configured");
          const credentialsPath = credentials.credentials_path ?? "";
          const projectId = credentials.project_id ?? "";
          const location = credentials.location ?? "us-central1";
          config = { provider: "vertex_gemini", workspace, model: modelId, credentials_path: credentialsPath, project_id: projectId, location };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "vertex_gemini" });
        } else if (modelProvider === "openrouter") {
          const apiKey = apiKeys.openrouter ?? (await getOpenRouterApiKey());
          if (!apiKey) throw new Error("OpenRouter API key not configured");
          config = { provider: "openrouter", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "openrouter" });
        } else if (modelProvider === "openai") {
          const apiKey = apiKeys.openai ?? (await getOpenAiApiKey());
          if (!apiKey) throw new Error("OpenAI API key not configured");
          config = { provider: "openai", workspace, model: modelId, api_key: apiKey, reasoning_effort: reasoningEffort };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "openai", reasoningEffort });
        } else if (modelProvider === "anthropic") {
          const apiKey = apiKeys.anthropic;
          if (!apiKey) throw new Error("Anthropic API key not configured");
          config = { provider: "anthropic", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "anthropic" });
        } else if (modelProvider === "ollama") {
          config = { provider: "ollama", workspace, model: modelId };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "ollama" });
        } else if (modelProvider === "gemini") {
          const apiKey = apiKeys.gemini;
          if (!apiKey) throw new Error("Gemini API key not configured");
          config = { provider: "gemini", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "gemini" });
        } else if (modelProvider === "groq") {
          const apiKey = apiKeys.groq;
          if (!apiKey) throw new Error("Groq API key not configured");
          config = { provider: "groq", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "groq" });
        } else if (modelProvider === "xai") {
          const apiKey = apiKeys.xai;
          if (!apiKey) throw new Error("xAI API key not configured");
          config = { provider: "xai", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "xai" });
        } else if (modelProvider === "zai_sdk") {
          const apiKey = apiKeys.zai_sdk;
          if (!apiKey) throw new Error("Z.AI SDK API key not configured");
          config = { provider: "zai_sdk", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "zai_sdk" });
        } else if (modelProvider === "nvidia") {
          const apiKey = apiKeys.nvidia;
          if (!apiKey) throw new Error("NVIDIA API key not configured");
          config = { provider: "nvidia", workspace, model: modelId, api_key: apiKey };
          await initAiSession(sessionId, config);
          setSessionAiConfig(sessionId, { status: "ready", provider: "nvidia" });
        }

        notify.success(`Switched to ${modelName}`);

        try {
          const providerForSettings = modelProvider === "vertex" ? "vertex_ai" : modelProvider;
          await saveProjectModel(workspace, providerForSettings, modelId);
        } catch (saveError) {
          logger.warn("Failed to save project model settings:", saveError);
        }
      } catch (error) {
        logger.error("Failed to switch model:", error);
        setSessionAiConfig(sessionId, { status: "error", errorMessage: error instanceof Error ? error.message : "Failed to switch model" });
        notify.error(`Failed to switch to ${modelName}`);
      }
    },
    [sessionId, model, provider, currentReasoningEffort, aiConfig, sessionWorkingDirectory, vertexAiCredentials, vertexGeminiCredentials, apiKeys, setSessionAiConfig]
  );

  const status = aiConfig?.status ?? "disconnected";

  if (status === "disconnected") {
    return (
      <div className="h-6 px-2.5 gap-1.5 text-xs font-medium rounded-lg bg-muted/60 text-muted-foreground flex items-center border border-transparent">
        <Cpu className="size-icon-status-bar" />
        <span>AI Disconnected</span>
      </div>
    );
  }
  if (status === "error") {
    return (
      <div className="h-6 px-2.5 gap-1.5 text-xs font-medium rounded-lg bg-destructive/10 text-destructive flex items-center border border-destructive/20">
        <Cpu className="size-icon-status-bar" />
        <span>AI Error</span>
      </div>
    );
  }
  if (status === "initializing") {
    return (
      <div className="h-6 px-2.5 gap-1.5 text-xs font-medium rounded-lg bg-accent/10 text-accent flex items-center border border-accent/20">
        <Cpu className="size-icon-status-bar animate-pulse" />
        <span>Initializing...</span>
      </div>
    );
  }
  if (!hasVisibleProviders) {
    return (
      <div className="h-6 px-2.5 gap-1.5 text-xs font-medium rounded-lg bg-muted/60 text-muted-foreground flex items-center border border-transparent">
        <Cpu className="size-icon-status-bar" />
        <span>Enable a provider in settings</span>
      </div>
    );
  }

  const providerSections: { key: string; label: string; provider: ModelProvider; nested?: boolean; checkReasoning?: boolean; creds?: unknown }[] = [
    { key: "vertex_ai", label: "Vertex AI", provider: "vertex", creds: aiConfig?.vertexConfig || vertexAiCredentials },
    { key: "vertex_gemini", label: "Vertex AI Gemini", provider: "vertex_gemini", nested: true, creds: vertexGeminiCredentials },
    { key: "openrouter", label: "OpenRouter", provider: "openrouter" },
    { key: "openai", label: "OpenAI", provider: "openai", nested: true, checkReasoning: true },
    { key: "anthropic", label: "Anthropic", provider: "anthropic" },
    { key: "ollama", label: "Ollama (Local)", provider: "ollama" },
    { key: "gemini", label: "Google Gemini", provider: "gemini" },
    { key: "groq", label: "Groq", provider: "groq" },
    { key: "xai", label: "xAI (Grok)", provider: "xai" },
    { key: "zai_sdk", label: "Z.AI SDK", provider: "zai_sdk" },
    { key: "nvidia", label: "NVIDIA NIM", provider: "nvidia" },
  ];

  const visibilityMap: Record<string, boolean> = {
    vertex_ai: showVertexAi, vertex_gemini: showVertexGemini, openrouter: showOpenRouter,
    openai: showOpenAi, anthropic: showAnthropic, ollama: showOllama, gemini: showGemini,
    groq: showGroq, xai: showXai, zai_sdk: showZaiSdk, nvidia: showNvidia,
  };

  let hasPrevious = false;

  return (
    <DropdownMenu onOpenChange={(open) => open && refreshProviderSettings()}>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm"
          className="h-6 px-2.5 gap-1.5 text-xs font-medium rounded-lg bg-accent/10 text-accent hover:text-accent hover:bg-accent/20 border border-accent/20 hover:border-accent/30">
          <Cpu className="size-icon-status-bar" />
          <span>{formatModelName(model, currentReasoningEffort)}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="bg-card border-[var(--border-medium)] min-w-[200px]">
        {providerSections.map((section) => {
          if (!visibilityMap[section.key]) return null;
          const separator = hasPrevious;
          hasPrevious = true;

          const providerKey = section.key as AiProvider;
          const models = section.nested
            ? getProviderGroupNested(providerKey)?.models ?? []
            : getProviderGroup(providerKey)?.models ?? [];

          return (
            <div key={section.key}>
              {separator && <DropdownMenuSeparator />}
              <div className="px-2 py-1 text-[10px] text-muted-foreground uppercase tracking-wide">
                {section.label}
              </div>
              {section.key === "ollama" ? (
                <div className="px-2 py-1.5 text-xs text-muted-foreground">Configure in settings</div>
              ) : section.nested ? (
                (models as ModelEntry[]).map((entry) =>
                  renderModelEntry(entry, section.key, section.provider, provider, model, currentReasoningEffort, !!section.checkReasoning, section.creds ?? null, handleModelSelect)
                )
              ) : (
                (models as { id: string; name: string }[]).map((m) => (
                  <DropdownMenuItem
                    key={m.id}
                    onClick={() => handleModelSelect(m.id, section.provider)}
                    disabled={section.key === "vertex_ai" && !aiConfig?.vertexConfig && !vertexAiCredentials}
                    className={cn("text-xs cursor-pointer",
                      model === m.id && provider === PROVIDER_MAP[section.provider]
                        ? "text-accent bg-[var(--accent-dim)]"
                        : "text-foreground hover:text-accent"
                    )}
                  >
                    {m.name}
                  </DropdownMenuItem>
                ))
              )}
            </div>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
});
