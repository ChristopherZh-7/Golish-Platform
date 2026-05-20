import { useEffect, useState } from "react";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { restoreBatchTerminals } from "@/lib/terminal-restore";
import { useStore } from "@/store";

type CreateTerminalFn = (
  workingDirectory?: string,
  skipConversationLink?: boolean,
  scrollback?: string,
  logicalTerminalId?: string
) => Promise<string | null>;

interface UseAiChatInitResult {
  pentestTools: ToolConfig[];
  configuredProviders: Set<string>;
}

export function useAiChatInit(createTerminalTab: CreateTerminalFn): UseAiChatInitResult {
  const [pentestTools, setPentestTools] = useState<ToolConfig[]>([]);
  const [configuredProviders, setConfiguredProviders] = useState<Set<string>>(new Set());

  const workspaceDataReady = useStore((s) => s.workspaceDataReady);
  const pendingTermData = useStore((s) => s.pendingTerminalRestoreData);

  // Unified terminal restore: fires on both initial boot and project switch.
  // Clearing the store value synchronously prevents double-processing under React Strict Mode.
  useEffect(() => {
    if (!workspaceDataReady || !pendingTermData) return;
    const data = pendingTermData;
    useStore.getState().setPendingTerminalRestoreData(null);
    void restoreBatchTerminals(data, createTerminalTab);
  }, [pendingTermData, workspaceDataReady, createTerminalTab]);

  // Load available pentest tools on mount
  useEffect(() => {
    scanTools()
      .then((result) => {
        if (result.success) {
          setPentestTools(result.tools.filter((t) => t.installed));
        }
      })
      .catch(() => {});
  }, []);

  // Load configured providers from settings.
  //
  // A provider only appears in the chat model picker when BOTH:
  //   1. It has working credentials (api_key / vertex creds), AND
  //   2. `show_in_selector` is not explicitly disabled.
  //
  // The second clause lets users keep the credentials in Settings (so
  // sub-agents or scripted runs can still call the provider) while hiding
  // the model from the casual chat picker — useful when you have many keys
  // but only want to drive a small subset from chat.
  useEffect(() => {
    const loadProviders = () => {
      getSettings()
        .then((settings) => {
          const configured = new Set<string>();
          const ai = settings.ai;
          // `show_in_selector` is optional and defaults to `true`. Treat
          // `undefined`/`null` as enabled; only `=== false` hides.
          const visible = (v: { show_in_selector?: boolean } | null | undefined): boolean =>
            v?.show_in_selector !== false;

          if (ai.anthropic?.api_key && visible(ai.anthropic)) configured.add("anthropic");
          if (ai.openai?.api_key && visible(ai.openai)) configured.add("openai");
          if (ai.openrouter?.api_key && visible(ai.openrouter)) configured.add("openrouter");
          if (ai.gemini?.api_key && visible(ai.gemini)) configured.add("gemini");
          if (ai.groq?.api_key && visible(ai.groq)) configured.add("groq");
          if (ai.xai?.api_key && visible(ai.xai)) configured.add("xai");
          if (ai.zai_sdk?.api_key && visible(ai.zai_sdk)) configured.add("zai_sdk");
          if (ai.nvidia?.api_key && visible(ai.nvidia)) configured.add("nvidia");
          if (ai.deepseek?.api_key && visible(ai.deepseek)) configured.add("deepseek");
          if ((ai.vertex_ai?.credentials_path || ai.vertex_ai?.project_id) && visible(ai.vertex_ai))
            configured.add("vertex_ai");
          if (
            (ai.vertex_gemini?.credentials_path || ai.vertex_gemini?.project_id) &&
            visible(ai.vertex_gemini)
          )
            configured.add("vertex_gemini");
          if (visible(ai.ollama)) configured.add("ollama");
          setConfiguredProviders(configured);
        })
        .catch(() => {});
    };

    loadProviders();
    window.addEventListener("settings-updated", loadProviders);
    return () => window.removeEventListener("settings-updated", loadProviders);
  }, []);

  return { pentestTools, configuredProviders };
}
