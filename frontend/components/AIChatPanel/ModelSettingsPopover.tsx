import { Check, RotateCcw, Settings2 } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Switch } from "@/components/ui/switch";
import { shutdownAiSession } from "@/lib/ai";
import { notifyModelOverrideChanged } from "@/lib/ai/model-overrides";
import type { ModelOverride } from "@/lib/settings";
import { getSettingsCached, updateSettings } from "@/lib/settings";
import { cn } from "@/lib/utils";
import { setStreamDebugEnabled } from "@/services/ai-events";
import { useStore } from "@/store";

import { modelOverrideKey } from "./providerConfig";

interface ModelUiCapabilities {
  supportsThinkingToggle: boolean;
  supportsReasoningEffort: boolean;
  supportsMaxTokens: boolean;
  effortOptions: { id: string; label: string }[];
}

const DEFAULT_EFFORT_OPTIONS = [
  { id: "low", label: "Low" },
  { id: "medium", label: "Medium" },
  { id: "high", label: "High" },
  { id: "max", label: "Max" },
];

function detectUiCapabilities(provider: string, model: string): ModelUiCapabilities {
  const modelLower = model.toLowerCase();

  if (provider === "anthropic" || provider === "vertex_ai" || modelLower.startsWith("claude")) {
    return {
      supportsThinkingToggle: true,
      supportsReasoningEffort: false,
      supportsMaxTokens: true,
      effortOptions: DEFAULT_EFFORT_OPTIONS,
    };
  }

  const isReasoningModel =
    provider === "openai" &&
    (modelLower.startsWith("o") || modelLower.startsWith("gpt-5") || modelLower.includes("codex"));
  if (isReasoningModel) {
    return {
      supportsThinkingToggle: false,
      supportsReasoningEffort: true,
      supportsMaxTokens: true,
      effortOptions: DEFAULT_EFFORT_OPTIONS,
    };
  }

  if (provider === "nvidia" || provider === "openrouter" || provider === "zai_sdk") {
    const isHybrid =
      modelLower.includes("qwen3") ||
      modelLower.includes("qwen-3") ||
      modelLower.includes("glm-4.7") ||
      modelLower.includes("deepseek-v3") ||
      modelLower.includes("kimi-k2");
    return {
      supportsThinkingToggle: isHybrid,
      supportsReasoningEffort: false,
      supportsMaxTokens: true,
      effortOptions: DEFAULT_EFFORT_OPTIONS,
    };
  }

  return {
    supportsThinkingToggle: false,
    supportsReasoningEffort: false,
    supportsMaxTokens: true,
    effortOptions: DEFAULT_EFFORT_OPTIONS,
  };
}

interface ModelSettingsPopoverProps {
  provider: string;
  model: string;
  modelLabel: string;
  trigger?: React.ReactNode;
  align?: "start" | "center" | "end";
  side?: "top" | "right" | "bottom" | "left";
  onApplied?: (key: string, override: ModelOverride | null) => void;
}

/**
 * Popover that lets the user override per-`(provider, model)` parameters
 * (Thinking toggle, Effort, Max Tokens, Stream Debug) inspired by Cursor's
 * model picker side-panel.
 *
 * Toggles auto-save on change; numeric fields commit on blur with a short
 * debounce. Persisted into
 * `settings.ai.model_overrides[<provider>::<model>]` and the active
 * conversation's AI session is torn down so the next prompt rebuilds the
 * bridge with the fresh `ProviderConfig.model_override`.
 */
export const ModelSettingsPopover = memo(function ModelSettingsPopover({
  provider,
  model,
  modelLabel,
  trigger,
  align = "end",
  side = "right",
  onApplied,
}: ModelSettingsPopoverProps) {
  const { t } = useTranslation();
  const overrideKey = useMemo(() => modelOverrideKey(provider, model), [provider, model]);
  const caps = useMemo(() => detectUiCapabilities(provider, model), [provider, model]);

  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<ModelOverride>({});
  const draftRef = useRef<ModelOverride>({});
  draftRef.current = draft;

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    (async () => {
      try {
        const settings = await getSettingsCached();
        const saved = settings.ai.model_overrides?.[overrideKey] ?? {};
        if (cancelled) return;
        setDraft({ ...saved });
      } catch (err) {
        console.error("[ModelSettings] Failed to load settings", err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, overrideKey]);

  const persist = useCallback(
    async (next: ModelOverride, prev: ModelOverride) => {
      try {
        const settings = await getSettingsCached();
        const nextOverrides = { ...(settings.ai.model_overrides ?? {}) };
        const cleaned: ModelOverride = {};
        if (next.thinking !== undefined) cleaned.thinking = next.thinking;
        if (next.reasoning_effort) cleaned.reasoning_effort = next.reasoning_effort;
        if (next.max_tokens) cleaned.max_tokens = next.max_tokens;
        if (next.context_window) cleaned.context_window = next.context_window;
        if (next.stream_debug) cleaned.stream_debug = next.stream_debug;

        if (Object.keys(cleaned).length === 0) {
          delete nextOverrides[overrideKey];
        } else {
          nextOverrides[overrideKey] = cleaned;
        }
        const nextSettings = {
          ...settings,
          ai: { ...settings.ai, model_overrides: nextOverrides },
        };
        await updateSettings(nextSettings);

        if (next.stream_debug !== prev.stream_debug) {
          setStreamDebugEnabled(!!next.stream_debug);
        }

        const store = useStore.getState();
        const activeConvId = store.activeConversationId;
        const activeConv = activeConvId ? store.conversations[activeConvId] : undefined;
        const activeSel = store.selectedAiModel;
        if (activeConv && activeSel?.provider === provider && activeSel?.model === model) {
          try {
            if (activeConv.aiSessionId) {
              await shutdownAiSession(activeConv.aiSessionId);
            }
          } catch (err) {
            console.warn("[ModelSettings] shutdown for reinit failed", err);
          }
          store.updateConversation(activeConv.id, { aiInitialized: false });
        }

        notifyModelOverrideChanged(overrideKey);

        onApplied?.(overrideKey, Object.keys(cleaned).length === 0 ? null : cleaned);
      } catch (err) {
        console.error("[ModelSettings] Failed to persist override", err);
      }
    },
    [provider, model, overrideKey, onApplied]
  );

  const updateField = useCallback(
    (patch: Partial<ModelOverride>) => {
      setDraft((d) => {
        const next = { ...d, ...patch };
        // Persist asynchronously without blocking the UI update; capture
        // the previous snapshot so we can detect stream-debug transitions.
        void persist(next, d);
        return next;
      });
    },
    [persist]
  );

  const handleReset = useCallback(() => {
    setDraft({});
    void persist({}, draftRef.current);
  }, [persist]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {trigger ?? (
          <button
            type="button"
            className="flex items-center justify-center w-6 h-6 rounded-md hover:bg-[var(--bg-hover)] text-muted-foreground/70 hover:text-foreground transition-colors"
            aria-label={t("ai.editModelSettings", "Edit model settings")}
            title={t("ai.editModelSettings", "Edit model settings")}
          >
            <Settings2 className="w-3.5 h-3.5" />
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent
        side={side}
        align={align}
        sideOffset={6}
        className="w-[260px] p-3 space-y-3 bg-card border-[var(--border-medium)]"
      >
        <header className="flex items-center justify-between">
          <div className="text-[11px] font-medium text-foreground truncate">{modelLabel}</div>
          <button
            type="button"
            onClick={handleReset}
            className="text-[10px] text-muted-foreground/70 hover:text-foreground flex items-center gap-1"
            aria-label="Reset to defaults"
          >
            <RotateCcw className="w-3 h-3" />
            {t("ai.resetDefaults", "Reset")}
          </button>
        </header>

        <section className="space-y-2">
          <div className="text-[10px] uppercase tracking-wide text-muted-foreground/70">
            {t("ai.options", "Options")}
          </div>
          {caps.supportsThinkingToggle && (
            <Row
              label={t("ai.thinking", "Thinking")}
              hint={t("ai.thinkingHint", "Disable to suppress chain-of-thought (hybrid models)")}
            >
              <Switch
                checked={draft.thinking ?? true}
                onCheckedChange={(checked) => updateField({ thinking: checked })}
              />
            </Row>
          )}
          <Row
            label={t("ai.streamDebug", "Stream Debug")}
            hint={t("ai.streamDebugHint", "Print per-chunk reasoning/text counts to the console")}
          >
            <Switch
              checked={draft.stream_debug ?? false}
              onCheckedChange={(checked) => updateField({ stream_debug: checked })}
            />
          </Row>
        </section>

        {caps.supportsReasoningEffort && (
          <section className="space-y-2">
            <div className="text-[10px] uppercase tracking-wide text-muted-foreground/70">
              {t("ai.effort", "Effort")}
            </div>
            <div className="grid grid-cols-2 gap-1">
              {caps.effortOptions.map((opt) => {
                const selected = draft.reasoning_effort === opt.id;
                return (
                  <button
                    key={opt.id}
                    type="button"
                    onClick={() => updateField({ reasoning_effort: opt.id })}
                    className={cn(
                      "text-[11px] px-2 py-1.5 rounded-md border transition-colors flex items-center justify-between",
                      selected
                        ? "border-accent bg-[var(--accent-dim)] text-accent"
                        : "border-[var(--border-subtle)] text-muted-foreground hover:bg-[var(--bg-hover)]"
                    )}
                  >
                    {opt.label}
                    {selected && <Check className="w-3 h-3" />}
                  </button>
                );
              })}
            </div>
          </section>
        )}

        {caps.supportsMaxTokens && (
          <section className="space-y-2">
            <div className="text-[10px] uppercase tracking-wide text-muted-foreground/70">
              {t("ai.maxOutputTokens", "Max Output Tokens")}
            </div>
            <input
              type="number"
              min={256}
              max={64_000}
              step={256}
              value={draft.max_tokens ?? ""}
              placeholder={t("ai.providerDefault", "provider default")}
              onChange={(e) => {
                // Defer persistence until blur to avoid spamming Tauri.
                const value = e.target.value;
                setDraft((d) => ({
                  ...d,
                  max_tokens: value ? Number(value) : undefined,
                }));
              }}
              onBlur={() => {
                void persist(draftRef.current, draftRef.current);
              }}
              className="w-full bg-background border border-[var(--border-subtle)] rounded-md px-2 py-1 text-[11px] text-foreground focus:outline-none focus:border-accent"
            />
          </section>
        )}
      </PopoverContent>
    </Popover>
  );
});

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <div className="min-w-0">
        <div className="text-[11px] text-foreground truncate">{label}</div>
        {hint && (
          <div className="text-[10px] text-muted-foreground/60 leading-tight mt-0.5">{hint}</div>
        )}
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  );
}
