import { logger } from "@/lib/logger";
import type { AgentMessage } from "./store-types";
import { useStore } from "./index";

export async function clearConversation(sessionId: string): Promise<void> {
  useStore.getState().clearTimeline(sessionId);

  try {
    const { clearAiConversationSession, clearAiConversation } = await import("@/lib/ai");
    try {
      await clearAiConversationSession(sessionId);
    } catch {
      await clearAiConversation();
    }
  } catch (error) {
    logger.warn("Failed to clear backend conversation history:", error);
  }
}

export async function restoreSession(sessionId: string, identifier: string): Promise<void> {
  const aiModule = await import("@/lib/ai");
  const { loadAiSession, restoreAiSession, initAiSession, buildProviderConfig } = aiModule;
  const { getSettings } = await import("@/lib/settings");

  const session = await loadAiSession(identifier);
  if (!session) {
    throw new Error(`Session '${identifier}' not found`);
  }

  const settings = await getSettings();
  const workspace = session.workspace_path;

  logger.info(
    `Restoring session (original: ${session.provider}/${session.model}, ` +
      `using current: ${settings.ai.default_provider}/${settings.ai.default_model})`
  );

  const config = await buildProviderConfig(settings, workspace);

  await initAiSession(sessionId, config);

  useStore.getState().setSessionAiConfig(sessionId, {
    provider: settings.ai.default_provider,
    model: settings.ai.default_model,
    status: "ready",
  });

  await restoreAiSession(sessionId, identifier);

  const agentMessages: AgentMessage[] = session.messages
    .filter((msg) => msg.role === "user" || msg.role === "assistant")
    .map((msg, index) => ({
      id: `restored-${identifier}-${index}`,
      sessionId,
      role: msg.role as "user" | "assistant",
      content: msg.content,
      timestamp: index === 0 ? session.started_at : session.ended_at,
      isStreaming: false,
    }));

  useStore.getState().clearTimeline(sessionId);
  useStore.getState().restoreAgentMessages(sessionId, agentMessages);
  useStore.getState().setInputMode(sessionId, "agent");
}
