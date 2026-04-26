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
  logicalTerminalId?: string,
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

  // Load configured providers from settings
  useEffect(() => {
    const loadProviders = () => {
      getSettings()
        .then((settings) => {
          const configured = new Set<string>();
          const ai = settings.ai;
          if (ai.anthropic?.api_key) configured.add("anthropic");
          if (ai.openai?.api_key) configured.add("openai");
          if (ai.openrouter?.api_key) configured.add("openrouter");
          if (ai.gemini?.api_key) configured.add("gemini");
          if (ai.groq?.api_key) configured.add("groq");
          if (ai.xai?.api_key) configured.add("xai");
          if (ai.zai_sdk?.api_key) configured.add("zai_sdk");
          if (ai.nvidia?.api_key) configured.add("nvidia");
          if (ai.vertex_ai?.credentials_path || ai.vertex_ai?.project_id)
            configured.add("vertex_ai");
          if (ai.vertex_gemini?.credentials_path || ai.vertex_gemini?.project_id)
            configured.add("vertex_gemini");
          configured.add("ollama");
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
