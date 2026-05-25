import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CustomSelect } from "@/components/ui/custom-select";
import { Input } from "@/components/ui/input";
import type { ApiKeysSettings, SidecarSettings, SynthesisBackendType } from "@/lib/settings";

interface AiSettingsProps {
  apiKeys: ApiKeysSettings;
  sidecarSettings: SidecarSettings;
  onApiKeysChange: (keys: ApiKeysSettings) => void;
  onSidecarChange: (settings: SidecarSettings) => void;
}

function SimpleSelect({
  value,
  onValueChange,
  options,
}: {
  id?: string;
  value: string;
  onValueChange: (value: string) => void;
  options: { value: string; label: string }[];
}) {
  return <CustomSelect value={value} onChange={onValueChange} options={options} />;
}

export function AiSettings({
  apiKeys,
  sidecarSettings,
  onApiKeysChange,
  onSidecarChange,
}: AiSettingsProps) {
  const { t } = useTranslation();
  const [synthesisStatus, setSynthesisStatus] = useState<string>("");
  const [isChangingBackend, setIsChangingBackend] = useState(false);

  const handleSynthesisBackendChange = (value: string) => {
    setIsChangingBackend(true);
    setSynthesisStatus("");

    onSidecarChange({ ...sidecarSettings, synthesis_backend: value as SynthesisBackendType });

    const backendNames: Record<string, string> = {
      local: t("aiSettings.backends.local"),
      vertex_anthropic: t("aiSettings.backends.vertexAnthropic"),
      openai: "OpenAI",
      grok: "Grok",
      template: t("aiSettings.backends.templateBased"),
    };
    setSynthesisStatus(t("aiSettings.backendSetTo", { backend: backendNames[value] || value }));
    setIsChangingBackend(false);
  };

  return (
    <div className="space-y-6">
      {/* API Keys */}
      <div className="space-y-4 p-4 rounded-lg bg-muted border border-[var(--border-medium)]">
        <h4 className="text-sm font-medium text-accent">{t("aiSettings.apiKeys")}</h4>

        <div className="space-y-2">
          <label htmlFor="api-key-tavily" className="text-sm text-foreground">
            {t("aiSettings.tavily")}
          </label>
          <Input
            id="api-key-tavily"
            type="password"
            value={apiKeys.tavily || ""}
            onChange={(e) => onApiKeysChange({ ...apiKeys, tavily: e.target.value || null })}
            placeholder="tvly-..."
            className="bg-background border-border text-foreground"
          />
          <p className="text-xs text-muted-foreground">{t("aiSettings.tavilyEnvHint")}</p>
        </div>

        <div className="space-y-2">
          <label htmlFor="api-key-brave" className="text-sm text-foreground">
            {t("aiSettings.braveSearchApi")}
          </label>
          <Input
            id="api-key-brave"
            type="password"
            value={apiKeys.brave || ""}
            onChange={(e) => onApiKeysChange({ ...apiKeys, brave: e.target.value || null })}
            placeholder="BSA..."
            className="bg-background border-border text-foreground"
          />
          <p className="text-xs text-muted-foreground">
            {t("aiSettings.braveKeyPrefix")}{" "}
            <a
              href="https://brave.com/search/api/"
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary underline decoration-primary/30 hover:decoration-primary/70"
            >
              brave.com/search/api
            </a>
          </p>
        </div>
      </div>

      {/* Synthesis Backend (Sidecar) */}
      <div className="space-y-4 p-4 rounded-lg bg-muted border border-[var(--border-medium)]">
        <h4 className="text-sm font-medium text-accent">
          {t("aiSettings.commitSynthesisBackend")}
        </h4>
        <p className="text-xs text-muted-foreground">{t("aiSettings.commitSynthesisDesc")}</p>

        <div className="space-y-2">
          <label htmlFor="synthesis-backend" className="text-sm text-foreground">
            {t("aiSettings.backend")}
          </label>
          <SimpleSelect
            id="synthesis-backend"
            value={sidecarSettings.synthesis_backend}
            onValueChange={handleSynthesisBackendChange}
            options={[
              { value: "local", label: t("aiSettings.backends.localQwen") },
              { value: "vertex_anthropic", label: t("aiSettings.backends.vertexClaude") },
              { value: "openai", label: "OpenAI" },
              { value: "grok", label: "xAI Grok" },
              { value: "template", label: t("aiSettings.backends.templateOnly") },
            ]}
          />
          {isChangingBackend && <p className="text-xs text-accent">{t("aiSettings.switching")}</p>}
          {synthesisStatus && <p className="text-xs text-[var(--success)]">{synthesisStatus}</p>}
        </div>

        {sidecarSettings.synthesis_backend === "local" && (
          <div className="text-xs text-muted-foreground space-y-1">
            <p>• Uses Qwen 2.5 0.5B model for on-device inference</p>
            <p>• Slower but works offline</p>
            <p>• Model downloads automatically on first use (~350MB)</p>
          </div>
        )}

        {sidecarSettings.synthesis_backend === "vertex_anthropic" && (
          <div className="space-y-3">
            <div className="text-xs text-muted-foreground space-y-1">
              <p>• Uses Claude via your Vertex AI configuration</p>
              <p>• Fast and high quality</p>
              <p>• Requires active Vertex AI credentials</p>
            </div>

            <div className="space-y-2">
              <label htmlFor="synthesis-vertex-model" className="text-sm text-foreground">
                Model
              </label>
              <SimpleSelect
                id="synthesis-vertex-model"
                value={sidecarSettings.synthesis_vertex.model}
                onValueChange={(value) =>
                  onSidecarChange({
                    ...sidecarSettings,
                    synthesis_vertex: {
                      ...sidecarSettings.synthesis_vertex,
                      model: value,
                    },
                  })
                }
                options={[
                  {
                    value: "claude-sonnet-4-6-20260217",
                    label: "Claude Sonnet 4.6",
                  },
                  {
                    value: "claude-opus-4-5-20251101",
                    label: "Claude Opus 4.5 (Most Capable)",
                  },
                  {
                    value: "claude-sonnet-4-5@20250929",
                    label: "Claude Sonnet 4.5",
                  },
                  {
                    value: "claude-haiku-4-5-20251001",
                    label: "Claude Haiku 4.5 (Fastest)",
                  },
                ]}
              />
            </div>

            {/* Optional: Override credentials for synthesis */}
            <details className="text-xs">
              <summary className="text-muted-foreground cursor-pointer hover:text-foreground">
                Override Vertex AI credentials (optional)
              </summary>
              <div className="mt-2 space-y-2 pl-2 border-l border-border">
                <p className="text-muted-foreground">
                  By default, synthesis uses your Vertex AI configuration from the Providers
                  section.
                </p>
                <Input
                  placeholder="Project ID (leave empty to use main config)"
                  value={sidecarSettings.synthesis_vertex.project_id || ""}
                  onChange={(e) =>
                    onSidecarChange({
                      ...sidecarSettings,
                      synthesis_vertex: {
                        ...sidecarSettings.synthesis_vertex,
                        project_id: e.target.value || null,
                      },
                    })
                  }
                  className="bg-background border-border text-foreground h-8"
                />
                <Input
                  placeholder="Location (leave empty to use main config)"
                  value={sidecarSettings.synthesis_vertex.location || ""}
                  onChange={(e) =>
                    onSidecarChange({
                      ...sidecarSettings,
                      synthesis_vertex: {
                        ...sidecarSettings.synthesis_vertex,
                        location: e.target.value || null,
                      },
                    })
                  }
                  className="bg-background border-border text-foreground h-8"
                />
              </div>
            </details>
          </div>
        )}

        {sidecarSettings.synthesis_backend === "openai" && (
          <div className="space-y-3">
            <div className="text-xs text-muted-foreground space-y-1">
              <p>• Uses OpenAI API</p>
              <p>• Fast and reliable</p>
            </div>

            <div className="space-y-2">
              <label htmlFor="synthesis-openai-model" className="text-sm text-foreground">
                Model
              </label>
              <SimpleSelect
                id="synthesis-openai-model"
                value={sidecarSettings.synthesis_openai.model}
                onValueChange={(value) =>
                  onSidecarChange({
                    ...sidecarSettings,
                    synthesis_openai: {
                      ...sidecarSettings.synthesis_openai,
                      model: value,
                    },
                  })
                }
                options={[
                  { value: "gpt-4o-mini", label: "GPT-4o Mini (Fastest)" },
                  { value: "gpt-4o", label: "GPT-4o" },
                  { value: "gpt-4-turbo", label: "GPT-4 Turbo" },
                ]}
              />
            </div>

            <div className="space-y-2">
              <label htmlFor="synthesis-openai-key" className="text-sm text-foreground">
                API Key
              </label>
              <Input
                id="synthesis-openai-key"
                type="password"
                placeholder="sk-..."
                value={sidecarSettings.synthesis_openai.api_key || ""}
                onChange={(e) =>
                  onSidecarChange({
                    ...sidecarSettings,
                    synthesis_openai: {
                      ...sidecarSettings.synthesis_openai,
                      api_key: e.target.value || null,
                    },
                  })
                }
                className="bg-background border-border text-foreground"
              />
            </div>
          </div>
        )}

        {sidecarSettings.synthesis_backend === "grok" && (
          <div className="space-y-3">
            <div className="text-xs text-muted-foreground space-y-1">
              <p>• Uses xAI Grok API</p>
            </div>

            <div className="space-y-2">
              <label htmlFor="synthesis-grok-model" className="text-sm text-foreground">
                Model
              </label>
              <SimpleSelect
                id="synthesis-grok-model"
                value={sidecarSettings.synthesis_grok.model}
                onValueChange={(value) =>
                  onSidecarChange({
                    ...sidecarSettings,
                    synthesis_grok: {
                      ...sidecarSettings.synthesis_grok,
                      model: value,
                    },
                  })
                }
                options={[
                  { value: "grok-2", label: "Grok 2" },
                  { value: "grok-2-mini", label: "Grok 2 Mini (Faster)" },
                ]}
              />
            </div>

            <div className="space-y-2">
              <label htmlFor="synthesis-grok-key" className="text-sm text-foreground">
                API Key
              </label>
              <Input
                id="synthesis-grok-key"
                type="password"
                placeholder="xai-..."
                value={sidecarSettings.synthesis_grok.api_key || ""}
                onChange={(e) =>
                  onSidecarChange({
                    ...sidecarSettings,
                    synthesis_grok: {
                      ...sidecarSettings.synthesis_grok,
                      api_key: e.target.value || null,
                    },
                  })
                }
                className="bg-background border-border text-foreground"
              />
            </div>
          </div>
        )}

        {sidecarSettings.synthesis_backend === "template" && (
          <div className="text-xs text-muted-foreground space-y-1">
            <p>• {t("aiSettings.templateFacts.simple")}</p>
            <p>• {t("aiSettings.templateFacts.offline")}</p>
            <p>• {t("aiSettings.templateFacts.basic")}</p>
          </div>
        )}
      </div>
    </div>
  );
}
