import { memo, useState, useRef, useEffect, useCallback } from "react";
import { Image, ChevronDown, Cpu, ArrowUp, Gauge, Plus, Clock, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useStore, useSessionAiConfig } from "@/store";
import { PROVIDER_GROUPS, formatModelName } from "@/lib/models";
import { initAiSession, type ProviderConfig } from "@/lib/ai";
import { getSettings } from "@/lib/settings";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  timestamp: number;
}

interface ChatConversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
}

function createConversation(): ChatConversation {
  return {
    id: `chat-${Date.now()}`,
    title: "New Chat",
    messages: [],
    createdAt: Date.now(),
  };
}

function MessageBlock({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";

  return (
    <div className={cn("px-4 py-3", !isUser && "bg-[var(--bg-hover)]")}>
      <div className="text-[11px] text-muted-foreground mb-1.5 font-medium">
        {isUser ? "You" : "Qbit AI"}
      </div>
      <div className="text-[13px] text-foreground leading-[1.6] whitespace-pre-wrap">
        {message.content}
      </div>
    </div>
  );
}

export const AIChatPanel = memo(function AIChatPanel() {
  const [conversations, setConversations] = useState<ChatConversation[]>(() => [createConversation()]);
  const [activeConvId, setActiveConvId] = useState(() => conversations[0].id);
  const [showHistory, setShowHistory] = useState(false);
  const [input, setInput] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const tabsRef = useRef<HTMLDivElement>(null);

  const activeConv = conversations.find((c) => c.id === activeConvId);
  const messages = activeConv?.messages ?? [];

  const setMessages = useCallback(
    (updater: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => {
      setConversations((prev) =>
        prev.map((c) => {
          if (c.id !== activeConvId) return c;
          const newMessages = typeof updater === "function" ? updater(c.messages) : updater;
          const title = newMessages.length > 0 && c.title === "New Chat"
            ? newMessages.find((m) => m.role === "user")?.content.slice(0, 30) || c.title
            : c.title;
          return { ...c, messages: newMessages, title };
        }),
      );
    },
    [activeConvId],
  );

  const handleNewChat = useCallback(() => {
    const conv = createConversation();
    setConversations((prev) => [...prev, conv]);
    setActiveConvId(conv.id);
    setInput("");
    setShowHistory(false);
  }, []);

  const handleCloseTab = useCallback(
    (convId: string, e: React.MouseEvent) => {
      e.stopPropagation();
      setConversations((prev) => {
        const filtered = prev.filter((c) => c.id !== convId);
        if (filtered.length === 0) {
          const fresh = createConversation();
          setActiveConvId(fresh.id);
          return [fresh];
        }
        if (convId === activeConvId) {
          setActiveConvId(filtered[filtered.length - 1].id);
        }
        return filtered;
      });
    },
    [activeConvId],
  );

  const activeSessionId = useStore((s) => s.activeSessionId);
  const setSessionAiConfig = useStore((s) => s.setSessionAiConfig);
  const aiConfig = useSessionAiConfig(activeSessionId ?? "");
  const sessionWorkingDirectory = useStore(
    (s) => (activeSessionId ? s.sessions[activeSessionId]?.workingDirectory : undefined)
  );
  const modelDisplay = aiConfig?.model ? formatModelName(aiConfig.model) : "No Model";

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const handleModelSelect = useCallback(
    async (modelId: string, provider: string) => {
      if (!activeSessionId) return;

      setSessionAiConfig(activeSessionId, { status: "initializing", model: modelId });

      try {
        const settings = await getSettings();
        const workspace = sessionWorkingDirectory ?? ".";
        let config: ProviderConfig;

        switch (provider) {
          case "anthropic": {
            const apiKey = settings.ai.anthropic.api_key;
            if (!apiKey) throw new Error("Anthropic API key not configured");
            config = { provider: "anthropic", workspace, model: modelId, api_key: apiKey };
            break;
          }
          case "openai": {
            const apiKey = settings.ai.openai.api_key;
            if (!apiKey) throw new Error("OpenAI API key not configured");
            config = { provider: "openai", workspace, model: modelId, api_key: apiKey };
            break;
          }
          case "openrouter": {
            const apiKey = settings.ai.openrouter.api_key;
            if (!apiKey) throw new Error("OpenRouter API key not configured");
            config = { provider: "openrouter", workspace, model: modelId, api_key: apiKey };
            break;
          }
          case "vertex_ai": {
            const vc = settings.ai.vertex_ai;
            config = {
              provider: "vertex_ai",
              workspace,
              model: modelId,
              credentials_path: vc.credentials_path ?? "",
              project_id: vc.project_id ?? "",
              location: vc.location ?? "us-east5",
            };
            break;
          }
          case "gemini": {
            const apiKey = settings.ai.gemini.api_key;
            if (!apiKey) throw new Error("Gemini API key not configured");
            config = { provider: "gemini", workspace, model: modelId, api_key: apiKey };
            break;
          }
          case "groq": {
            const apiKey = settings.ai.groq.api_key;
            if (!apiKey) throw new Error("Groq API key not configured");
            config = { provider: "groq", workspace, model: modelId, api_key: apiKey };
            break;
          }
          case "ollama":
            config = { provider: "ollama", workspace, model: modelId };
            break;
          default:
            throw new Error(`Unsupported provider: ${provider}`);
        }

        await initAiSession(activeSessionId, config);
        setSessionAiConfig(activeSessionId, {
          status: "ready",
          provider,
          model: modelId,
        });
      } catch (err) {
        console.error("[AIChatPanel] Model switch failed:", err);
        setSessionAiConfig(activeSessionId, { status: "error" });
      }
    },
    [activeSessionId, setSessionAiConfig, sessionWorkingDirectory],
  );

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text) return;

    setMessages((prev) => [
      ...prev,
      { id: `user-${Date.now()}`, role: "user", content: text, timestamp: Date.now() },
    ]);
    setInput("");
    if (textareaRef.current) textareaRef.current.style.height = "auto";

    setTimeout(() => {
      setMessages((prev) => [
        ...prev,
        {
          id: `ai-${Date.now()}`,
          role: "assistant",
          content: "功能开发中，后续将支持自动化工具调用和智能漏洞分析。",
          timestamp: Date.now(),
        },
      ]);
    }, 600);
  }, [input]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const handleTextareaInput = useCallback(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 160)}px`;
    }
  }, []);

  const currentModel = aiConfig?.model ?? "";
  const currentProvider = aiConfig?.provider ?? "";

  return (
    <div className="flex flex-col h-full">
      {/* Tab Bar */}
      <div className="h-[40px] flex items-center px-2 gap-1.5 flex-shrink-0">
        <div ref={tabsRef} className="flex-1 flex items-center gap-1.5 overflow-x-auto scrollbar-none min-w-0">
          {conversations.map((conv) => (
            <button
              key={conv.id}
              type="button"
              className={cn(
                "group flex items-center gap-1.5 h-[28px] px-3 text-[12px] whitespace-nowrap flex-shrink-0 transition-all rounded-lg",
                conv.id === activeConvId
                  ? "text-foreground bg-[var(--bg-hover)]"
                  : "text-muted-foreground hover:text-foreground/80",
              )}
              onClick={() => { setActiveConvId(conv.id); setShowHistory(false); }}
            >
              <span className="max-w-[120px] truncate">{conv.title}</span>
              <span
                className={cn(
                  "w-4 h-4 flex items-center justify-center rounded-full transition-opacity",
                  conv.id === activeConvId ? "opacity-60 hover:opacity-100" : "opacity-0 group-hover:opacity-60 hover:!opacity-100",
                )}
                onClick={(e) => handleCloseTab(conv.id, e)}
                onKeyDown={() => {}}
                role="button"
                tabIndex={-1}
              >
                <X className="w-2.5 h-2.5" />
              </span>
            </button>
          ))}
        </div>
        <div className="flex items-center gap-0.5 flex-shrink-0">
          <button
            type="button"
            title="新对话"
            className="h-6 w-6 flex items-center justify-center rounded-md text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
            onClick={handleNewChat}
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            title="历史记录"
            className={cn(
              "h-6 w-6 flex items-center justify-center rounded-md transition-colors",
              showHistory ? "text-foreground bg-[var(--bg-hover)]" : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]",
            )}
            onClick={() => setShowHistory((v) => !v)}
          >
            <Clock className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* History panel */}
      {showHistory && (
        <div className="flex-1 overflow-y-auto overflow-x-hidden border-b border-[var(--border-subtle)]">
          <div className="px-3 py-2">
            <span className="text-[11px] text-muted-foreground uppercase tracking-wider font-semibold">历史对话</span>
          </div>
          {conversations.filter((c) => c.messages.length > 0).length === 0 ? (
            <div className="flex items-center justify-center py-8">
              <span className="text-[12px] text-muted-foreground/50">暂无历史对话</span>
            </div>
          ) : (
            conversations
              .filter((c) => c.messages.length > 0)
              .sort((a, b) => b.createdAt - a.createdAt)
              .map((conv) => (
                <button
                  key={conv.id}
                  type="button"
                  className={cn(
                    "w-full text-left px-3 py-2 text-[12px] hover:bg-[var(--bg-hover)] transition-colors",
                    conv.id === activeConvId ? "text-foreground bg-[var(--bg-hover)]" : "text-muted-foreground",
                  )}
                  onClick={() => { setActiveConvId(conv.id); setShowHistory(false); }}
                >
                  <div className="truncate">{conv.title}</div>
                  <div className="text-[10px] text-muted-foreground/50 mt-0.5">
                    {new Date(conv.createdAt).toLocaleDateString()} · {conv.messages.length} 条消息
                  </div>
                </button>
              ))
          )}
        </div>
      )}

      {/* Messages */}
      {!showHistory && <div className="flex-1 overflow-y-auto overflow-x-hidden">
        {messages.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full select-none gap-4">
            <div className="flex items-center gap-1.5">
              {[0, 1, 2].map((i) => (
                <div
                  key={i}
                  className="w-1.5 h-1.5 rounded-full bg-accent/40 typing-dot"
                  style={{ animationDelay: `${i * 0.2}s` }}
                />
              ))}
            </div>
            <p className="text-[13px] text-muted-foreground/50">今天要做点什么呢</p>
          </div>
        ) : (
          <div>
            {messages.map((msg) => (
              <MessageBlock key={msg.id} message={msg} />
            ))}
            <div ref={messagesEndRef} />
          </div>
        )}
      </div>}

      {/* Input Area */}
      <div className="p-3 flex-shrink-0">
        <div className="rounded-lg border border-[var(--border-subtle)] bg-background overflow-hidden focus-within:border-muted-foreground/30 transition-colors">
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            onInput={handleTextareaInput}
            placeholder="Add a follow-up"
            rows={1}
            className={cn(
              "w-full bg-transparent border-none outline-none resize-none",
              "text-[13px] text-foreground placeholder:text-muted-foreground/40",
              "leading-relaxed max-h-[160px] px-3 pt-2.5 pb-1.5",
            )}
          />
          {/* Bottom toolbar */}
          <div className="flex items-center justify-between px-2.5 pb-2">
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                className="flex items-center gap-1 px-2 py-1 rounded-md bg-muted text-[11px] text-foreground font-medium hover:bg-[var(--bg-hover)] transition-colors"
              >
                <span className="text-accent">∞</span>
                Agent
                <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
              </button>

              {/* Model selector dropdown */}
              <DropdownMenu>
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
                  {PROVIDER_GROUPS.map((group, gi) => (
                    <div key={group.provider}>
                      {gi > 0 && <DropdownMenuSeparator />}
                      <div className="px-2 py-1 text-[10px] text-muted-foreground uppercase tracking-wide">
                        {group.providerName}
                      </div>
                      {group.models.map((model) => {
                        const isSelected =
                          currentModel === model.id &&
                          (currentProvider === group.provider ||
                            currentProvider === `anthropic_vertex`);
                        return (
                          <DropdownMenuItem
                            key={`${group.provider}-${model.id}-${model.reasoningEffort ?? ""}`}
                            onClick={() => handleModelSelect(model.id, group.provider)}
                            className={cn(
                              "text-xs cursor-pointer",
                              isSelected
                                ? "text-accent bg-[var(--accent-dim)]"
                                : "text-foreground hover:text-accent",
                            )}
                          >
                            {model.name}
                          </DropdownMenuItem>
                        );
                      })}
                    </div>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <div className="flex items-center gap-1">
              <button
                type="button"
                title="用量"
                className="h-6 w-6 flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
              >
                <Gauge className="w-3.5 h-3.5" />
              </button>
              <button
                type="button"
                title="上传图片"
                className="h-6 w-6 flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
              >
                <Image className="w-3.5 h-3.5" />
              </button>
              <button
                type="button"
                title={input.trim() ? "发送" : ""}
                onClick={handleSend}
                disabled={!input.trim()}
                className={cn(
                  "h-6 w-6 flex items-center justify-center rounded transition-colors",
                  input.trim()
                    ? "bg-accent text-accent-foreground hover:bg-accent/80 cursor-pointer"
                    : "bg-muted text-muted-foreground cursor-default",
                )}
              >
                <ArrowUp className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
});
