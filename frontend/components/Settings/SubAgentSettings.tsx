import { useCallback, useEffect, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Edit3,
  Loader2,
  Lock,
  Plus,
  Save,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { CustomSelect } from "@/components/ui/custom-select";
import { Switch } from "@/components/ui/switch";
import type { AiProvider, SubAgentModelConfig } from "@/lib/settings";
import {
  type AgentFileInfo,
  deleteAgentDefinition,
  listAgentDefinitions,
  readAgentPrompt,
  saveAgentDefinition,
  seedAgents,
} from "@/lib/ai";
import { notify } from "@/lib/notify";

interface SubAgentSettingsProps {
  subAgentModels: Record<string, SubAgentModelConfig>;
  onChange: (models: Record<string, SubAgentModelConfig>) => void;
}

const PROVIDER_OPTIONS: { value: AiProvider; label: string }[] = [
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
];

const MODEL_SUGGESTIONS: Record<AiProvider, string[]> = {
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
    "deepseek-ai/deepseek-v3.2",
    "google/gemma-4-31b-it",
  ],
};

interface EditingAgent {
  id: string;
  name: string;
  description: string;
  systemPrompt: string;
  allowedTools: string[];
  maxIterations: number;
  timeoutSecs: number | null;
  idleTimeoutSecs: number | null;
  readonly: boolean;
  isBackground: boolean;
  model: string;
  temperature: number | null;
  maxTokens: number | null;
  topP: number | null;
  isNew: boolean;
}

function emptyAgent(): EditingAgent {
  return {
    id: "",
    name: "",
    description: "",
    systemPrompt: "",
    allowedTools: [],
    maxIterations: 50,
    timeoutSecs: 600,
    idleTimeoutSecs: 180,
    readonly: false,
    isBackground: false,
    model: "inherit",
    temperature: null,
    maxTokens: null,
    topP: null,
    isNew: true,
  };
}

export function SubAgentSettings({ subAgentModels, onChange }: SubAgentSettingsProps) {
  const [agents, setAgents] = useState<AgentFileInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  const [editingAgent, setEditingAgent] = useState<EditingAgent | null>(null);
  const [savingId, setSavingId] = useState<string | null>(null);
  const [toolInput, setToolInput] = useState("");

  const loadAgents = useCallback(async () => {
    try {
      setLoading(true);
      await seedAgents();
      const list = await listAgentDefinitions();
      setAgents(list);
    } catch (err) {
      console.error("Failed to load agents:", err);
      notify.error("Failed to load agent definitions");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadAgents();
  }, [loadAgents]);

  const toggleExpand = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const startEditing = async (agent: AgentFileInfo) => {
    try {
      const prompt = await readAgentPrompt(agent.id);
      setEditingAgent({
        id: agent.id,
        name: agent.name,
        description: agent.description,
        systemPrompt: prompt,
        allowedTools: [...agent.allowed_tools],
        maxIterations: agent.max_iterations,
        timeoutSecs: agent.timeout_secs,
        idleTimeoutSecs: agent.idle_timeout_secs,
        readonly: agent.readonly,
        isBackground: agent.is_background,
        model: agent.model || "inherit",
        temperature: agent.temperature,
        maxTokens: agent.max_tokens,
        topP: agent.top_p,
        isNew: false,
      });
    } catch (err) {
      console.error("Failed to read agent prompt:", err);
      notify.error("Failed to load agent prompt");
    }
  };

  const startCreating = () => {
    setEditingAgent(emptyAgent());
  };

  const cancelEditing = () => {
    setEditingAgent(null);
    setToolInput("");
  };

  const addTool = () => {
    if (!editingAgent || !toolInput.trim()) return;
    const tool = toolInput.trim();
    if (!editingAgent.allowedTools.includes(tool)) {
      setEditingAgent({ ...editingAgent, allowedTools: [...editingAgent.allowedTools, tool] });
    }
    setToolInput("");
  };

  const removeTool = (tool: string) => {
    if (!editingAgent) return;
    setEditingAgent({
      ...editingAgent,
      allowedTools: editingAgent.allowedTools.filter((t) => t !== tool),
    });
  };

  const handleSave = async () => {
    if (!editingAgent) return;
    if (!editingAgent.id.trim()) {
      notify.error("Agent ID is required");
      return;
    }
    if (!editingAgent.name.trim()) {
      notify.error("Agent name is required");
      return;
    }

    try {
      setSavingId(editingAgent.id);
      await saveAgentDefinition({
        agentId: editingAgent.id.trim(),
        name: editingAgent.name.trim(),
        description: editingAgent.description.trim(),
        systemPrompt: editingAgent.systemPrompt,
        allowedTools: editingAgent.allowedTools,
        maxIterations: editingAgent.maxIterations,
        timeoutSecs: editingAgent.timeoutSecs ?? undefined,
        idleTimeoutSecs: editingAgent.idleTimeoutSecs ?? undefined,
        readonly: editingAgent.readonly,
        isBackground: editingAgent.isBackground,
        model: editingAgent.model !== "inherit" ? editingAgent.model : undefined,
        temperature: editingAgent.temperature ?? undefined,
        maxTokens: editingAgent.maxTokens ?? undefined,
        topP: editingAgent.topP ?? undefined,
      });
      notify.success(`Agent "${editingAgent.name}" saved`);
      setEditingAgent(null);
      setToolInput("");
      await loadAgents();
    } catch (err) {
      console.error("Failed to save agent:", err);
      notify.error(`Failed to save agent: ${err}`);
    } finally {
      setSavingId(null);
    }
  };

  const handleDelete = async (agent: AgentFileInfo) => {
    if (agent.is_system) {
      notify.error("System agents cannot be deleted");
      return;
    }
    try {
      await deleteAgentDefinition(agent.id);
      notify.success(`Agent "${agent.name}" deleted`);
      await loadAgents();
    } catch (err) {
      console.error("Failed to delete agent:", err);
      notify.error(`Failed to delete: ${err}`);
    }
  };

  // Model override helpers (settings-based, separate from file-based config)
  const updateModelOverride = (agentId: string, config: SubAgentModelConfig | null) => {
    if (config === null) {
      const { [agentId]: _, ...rest } = subAgentModels;
      onChange(rest);
    } else {
      onChange({ ...subAgentModels, [agentId]: config });
    }
  };

  const getModelConfig = (agentId: string): SubAgentModelConfig => {
    return subAgentModels[agentId] || {};
  };

  const hasModelOverride = (agentId: string): boolean => {
    const config = subAgentModels[agentId];
    return Boolean(config?.provider && config?.model);
  };

  // ── Editing form ──

  if (editingAgent) {
    return (
      <div className="space-y-6">
        <div className="flex items-center justify-between">
          <h4 className="text-sm font-medium text-accent">
            {editingAgent.isNew ? "Create New Agent" : `Edit: ${editingAgent.name}`}
          </h4>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={cancelEditing}>
              <X className="w-4 h-4 mr-1" /> Cancel
            </Button>
            <Button
              size="sm"
              onClick={handleSave}
              disabled={savingId === editingAgent.id}
              className="bg-accent text-accent-foreground hover:bg-accent/90"
            >
              {savingId === editingAgent.id ? (
                <Loader2 className="w-4 h-4 mr-1 animate-spin" />
              ) : (
                <Save className="w-4 h-4 mr-1" />
              )}
              Save
            </Button>
          </div>
        </div>

        {/* Basic Info */}
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label className="text-xs text-muted-foreground">
                ID {!editingAgent.isNew && <span className="text-[10px]">(read-only)</span>}
              </label>
              <Input
                value={editingAgent.id}
                onChange={(e) =>
                  editingAgent.isNew && setEditingAgent({ ...editingAgent, id: e.target.value.replace(/[^a-z0-9_-]/g, "") })
                }
                readOnly={!editingAgent.isNew}
                placeholder="my-agent"
                className="bg-background border-border text-foreground h-9 font-mono text-xs"
              />
            </div>
            <div className="space-y-1">
              <label className="text-xs text-muted-foreground">Name</label>
              <Input
                value={editingAgent.name}
                onChange={(e) => setEditingAgent({ ...editingAgent, name: e.target.value })}
                placeholder="My Agent"
                className="bg-background border-border text-foreground h-9"
              />
            </div>
          </div>

          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">Description</label>
            <Input
              value={editingAgent.description}
              onChange={(e) => setEditingAgent({ ...editingAgent, description: e.target.value })}
              placeholder="What this agent specializes in..."
              className="bg-background border-border text-foreground h-9"
            />
          </div>
        </div>

        {/* System Prompt */}
        <div className="space-y-1">
          <label className="text-xs text-muted-foreground">System Prompt</label>
          <Textarea
            value={editingAgent.systemPrompt}
            onChange={(e) => setEditingAgent({ ...editingAgent, systemPrompt: e.target.value })}
            placeholder="You are a specialized agent for..."
            className="bg-background border-border text-foreground font-mono text-xs min-h-[200px] resize-y"
          />
          <p className="text-[10px] text-muted-foreground">
            {editingAgent.systemPrompt.length} characters
          </p>
        </div>

        {/* Allowed Tools */}
        <div className="space-y-2">
          <label className="text-xs text-muted-foreground">
            Allowed Tools ({editingAgent.allowedTools.length})
          </label>
          <div className="flex flex-wrap gap-1.5">
            {editingAgent.allowedTools.map((tool) => (
              <Badge
                key={tool}
                variant="secondary"
                className="text-xs flex items-center gap-1 px-2 py-0.5"
              >
                {tool}
                <button
                  type="button"
                  onClick={() => removeTool(tool)}
                  className="hover:text-destructive"
                >
                  <X className="w-3 h-3" />
                </button>
              </Badge>
            ))}
            {editingAgent.allowedTools.length === 0 && (
              <span className="text-xs text-muted-foreground italic">
                Empty = all tools allowed
              </span>
            )}
          </div>
          <div className="flex gap-2">
            <Input
              value={toolInput}
              onChange={(e) => setToolInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), addTool())}
              placeholder="run_pty_cmd"
              className="bg-background border-border text-foreground h-8 text-xs font-mono flex-1"
            />
            <Button variant="outline" size="sm" onClick={addTool} className="h-8 px-3 text-xs">
              Add
            </Button>
          </div>
        </div>

        {/* Execution Config */}
        <div className="space-y-3">
          <label className="text-xs text-muted-foreground font-medium">Execution</label>
          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-1">
              <label className="text-[10px] text-muted-foreground">Max Iterations</label>
              <Input
                type="number"
                min={1}
                max={200}
                value={editingAgent.maxIterations}
                onChange={(e) =>
                  setEditingAgent({ ...editingAgent, maxIterations: parseInt(e.target.value) || 50 })
                }
                className="bg-background border-border h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] text-muted-foreground">Timeout (sec)</label>
              <Input
                type="number"
                min={0}
                value={editingAgent.timeoutSecs ?? ""}
                onChange={(e) =>
                  setEditingAgent({
                    ...editingAgent,
                    timeoutSecs: e.target.value ? parseInt(e.target.value) : null,
                  })
                }
                placeholder="600"
                className="bg-background border-border h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] text-muted-foreground">Idle Timeout (sec)</label>
              <Input
                type="number"
                min={0}
                value={editingAgent.idleTimeoutSecs ?? ""}
                onChange={(e) =>
                  setEditingAgent({
                    ...editingAgent,
                    idleTimeoutSecs: e.target.value ? parseInt(e.target.value) : null,
                  })
                }
                placeholder="180"
                className="bg-background border-border h-8 text-xs"
              />
            </div>
          </div>

          <div className="flex gap-6">
            <div className="flex items-center gap-2">
              <Switch
                checked={editingAgent.readonly}
                onCheckedChange={(v) => setEditingAgent({ ...editingAgent, readonly: v })}
              />
              <label className="text-xs text-muted-foreground">Read-only</label>
            </div>
            <div className="flex items-center gap-2">
              <Switch
                checked={editingAgent.isBackground}
                onCheckedChange={(v) => setEditingAgent({ ...editingAgent, isBackground: v })}
              />
              <label className="text-xs text-muted-foreground">Background</label>
            </div>
          </div>
        </div>

        {/* LLM Parameters */}
        <div className="space-y-3">
          <label className="text-xs text-muted-foreground font-medium">LLM Parameters</label>
          <div className="space-y-1">
            <label className="text-[10px] text-muted-foreground">Model</label>
            <Input
              value={editingAgent.model}
              onChange={(e) => setEditingAgent({ ...editingAgent, model: e.target.value })}
              placeholder="inherit (use main model)"
              className="bg-background border-border h-8 text-xs font-mono"
            />
            <p className="text-[10px] text-muted-foreground">
              &quot;inherit&quot; = use main agent model, &quot;fast&quot; = auto-pick fast model
            </p>
          </div>

          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-1">
              <label className="text-[10px] text-muted-foreground">Temperature</label>
              <Input
                type="number"
                min={0}
                max={2}
                step={0.1}
                value={editingAgent.temperature ?? ""}
                onChange={(e) =>
                  setEditingAgent({
                    ...editingAgent,
                    temperature: e.target.value ? parseFloat(e.target.value) : null,
                  })
                }
                placeholder="default"
                className="bg-background border-border h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] text-muted-foreground">Max Tokens</label>
              <Input
                type="number"
                min={256}
                max={200000}
                step={256}
                value={editingAgent.maxTokens ?? ""}
                onChange={(e) =>
                  setEditingAgent({
                    ...editingAgent,
                    maxTokens: e.target.value ? parseInt(e.target.value) : null,
                  })
                }
                placeholder="default"
                className="bg-background border-border h-8 text-xs"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] text-muted-foreground">Top P</label>
              <Input
                type="number"
                min={0}
                max={1}
                step={0.05}
                value={editingAgent.topP ?? ""}
                onChange={(e) =>
                  setEditingAgent({
                    ...editingAgent,
                    topP: e.target.value ? parseFloat(e.target.value) : null,
                  })
                }
                placeholder="default"
                className="bg-background border-border h-8 text-xs"
              />
            </div>
          </div>
        </div>
      </div>
    );
  }

  // ── Agent List ──

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="w-5 h-5 text-muted-foreground animate-spin" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <h4 className="text-sm font-medium text-accent">Agents</h4>
          <p className="text-xs text-muted-foreground">
            Manage sub-agent definitions. Agents are stored as <code>.md</code> files in{" "}
            <code>~/.golish/agents/</code>.
          </p>
        </div>
        <Button
          size="sm"
          onClick={startCreating}
          className="bg-accent text-accent-foreground hover:bg-accent/90"
        >
          <Plus className="w-4 h-4 mr-1" /> New Agent
        </Button>
      </div>

      <div className="space-y-2">
        {agents.map((agent) => {
          const isExpanded = expandedIds.has(agent.id);
          const modelConfig = getModelConfig(agent.id);
          const hasOverride = hasModelOverride(agent.id);

          return (
            <div
              key={agent.id}
              className="rounded-lg bg-muted border border-[var(--border-medium)] overflow-hidden"
            >
              {/* Header row */}
              <button
                type="button"
                onClick={() => toggleExpand(agent.id)}
                className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-[var(--bg-hover)] transition-colors"
              >
                {isExpanded ? (
                  <ChevronDown className="w-4 h-4 text-muted-foreground flex-shrink-0" />
                ) : (
                  <ChevronRight className="w-4 h-4 text-muted-foreground flex-shrink-0" />
                )}

                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-foreground">{agent.name}</span>
                    <span className="text-[10px] font-mono text-muted-foreground/60">
                      {agent.id}
                    </span>
                    {agent.is_system && (
                      <Badge variant="outline" className="text-[9px] px-1.5 py-0 h-4">
                        <Lock className="w-2.5 h-2.5 mr-0.5" /> system
                      </Badge>
                    )}
                    {agent.source === "file" && (
                      <Badge variant="secondary" className="text-[9px] px-1.5 py-0 h-4">
                        file
                      </Badge>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground truncate">{agent.description}</p>
                </div>

                <div className="flex items-center gap-2 flex-shrink-0">
                  <span className="text-[10px] text-muted-foreground">
                    {agent.allowed_tools.length} tools
                  </span>
                  {hasOverride && (
                    <Badge variant="secondary" className="text-[9px]">
                      model override
                    </Badge>
                  )}
                </div>
              </button>

              {/* Expanded detail */}
              {isExpanded && (
                <div className="px-4 pb-4 pt-1 border-t border-[var(--border-medium)] space-y-4">
                  {/* Quick info */}
                  <div className="grid grid-cols-4 gap-3 text-xs">
                    <div>
                      <span className="text-muted-foreground">Model:</span>{" "}
                      <span className="font-mono">{agent.model || "inherit"}</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground">Max iter:</span>{" "}
                      {agent.max_iterations}
                    </div>
                    <div>
                      <span className="text-muted-foreground">Timeout:</span>{" "}
                      {agent.timeout_secs ? `${agent.timeout_secs}s` : "none"}
                    </div>
                    <div>
                      <span className="text-muted-foreground">Idle:</span>{" "}
                      {agent.idle_timeout_secs ? `${agent.idle_timeout_secs}s` : "none"}
                    </div>
                  </div>

                  {/* Tools */}
                  {agent.allowed_tools.length > 0 && (
                    <div className="space-y-1">
                      <span className="text-[10px] text-muted-foreground uppercase tracking-wider">
                        Allowed Tools
                      </span>
                      <div className="flex flex-wrap gap-1">
                        {agent.allowed_tools.map((tool) => (
                          <Badge
                            key={tool}
                            variant="secondary"
                            className="text-[10px] font-mono px-1.5 py-0"
                          >
                            {tool}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* Runtime model override (settings-based) */}
                  <div className="space-y-2 p-3 rounded bg-background border border-[var(--border-medium)]">
                    <div className="flex items-center justify-between">
                      <span className="text-[10px] text-muted-foreground uppercase tracking-wider">
                        Runtime Model Override
                      </span>
                      {hasOverride && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => updateModelOverride(agent.id, null)}
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
                          updateModelOverride(agent.id, {
                            ...modelConfig,
                            provider: value as AiProvider,
                            model: value !== modelConfig.provider ? undefined : modelConfig.model,
                          })
                        }
                        options={PROVIDER_OPTIONS}
                        placeholder="Use default"
                      />
                      {modelConfig.provider ? (
                        <div className="relative">
                          <Input
                            value={modelConfig.model || ""}
                            onChange={(e) =>
                              updateModelOverride(agent.id, {
                                ...modelConfig,
                                model: e.target.value,
                              })
                            }
                            placeholder="Enter model name"
                            list={`override-${agent.id}-models`}
                            className="bg-background border-border h-9 text-xs"
                          />
                          <datalist id={`override-${agent.id}-models`}>
                            {(MODEL_SUGGESTIONS[modelConfig.provider] || []).map((m) => (
                              <option key={m} value={m} />
                            ))}
                          </datalist>
                        </div>
                      ) : (
                        <Input
                          disabled
                          placeholder="Select provider first"
                          className="bg-muted border-border h-9 text-xs"
                        />
                      )}
                    </div>
                    {hasOverride && (
                      <p className="text-[10px] text-[var(--success)]">
                        Runtime override: {modelConfig.provider} / {modelConfig.model}
                      </p>
                    )}
                  </div>

                  {/* LLM params if set */}
                  {(agent.temperature != null || agent.max_tokens != null || agent.top_p != null) && (
                    <div className="flex gap-4 text-xs">
                      {agent.temperature != null && (
                        <span>
                          <span className="text-muted-foreground">temp:</span> {agent.temperature}
                        </span>
                      )}
                      {agent.max_tokens != null && (
                        <span>
                          <span className="text-muted-foreground">max_tokens:</span>{" "}
                          {agent.max_tokens}
                        </span>
                      )}
                      {agent.top_p != null && (
                        <span>
                          <span className="text-muted-foreground">top_p:</span> {agent.top_p}
                        </span>
                      )}
                    </div>
                  )}

                  {/* File path */}
                  {agent.path && (
                    <p className="text-[10px] text-muted-foreground/50 font-mono truncate">
                      {agent.path}
                    </p>
                  )}

                  {/* Action buttons */}
                  <div className="flex gap-2 pt-1">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => startEditing(agent)}
                      className="h-7 text-xs"
                    >
                      <Edit3 className="w-3 h-3 mr-1" /> Edit Definition
                    </Button>
                    {!agent.is_system && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDelete(agent)}
                        className="h-7 text-xs text-muted-foreground hover:text-destructive"
                      >
                        <Trash2 className="w-3 h-3 mr-1" /> Delete
                      </Button>
                    )}
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>

      {agents.length === 0 && (
        <div className="text-center py-8">
          <p className="text-sm text-muted-foreground">No agents found.</p>
          <p className="text-xs text-muted-foreground mt-1">
            Click &quot;New Agent&quot; to create one or restart to seed defaults.
          </p>
        </div>
      )}

      <div className="text-xs text-muted-foreground border-t border-[var(--border-medium)] pt-4">
        <p>
          <strong>Tip:</strong> Agent definitions are stored as Markdown files with YAML frontmatter.
          System agents (Worker, Memorist, Reflector) can be edited but not deleted.
          Changes take effect on the next session.
        </p>
      </div>
    </div>
  );
}
