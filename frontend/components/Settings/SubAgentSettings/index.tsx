import {
  ChevronDown,
  ChevronRight,
  Edit3,
  FolderOpen,
  Globe,
  Loader2,
  Lock,
  Plus,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  type AgentFileInfo,
  deleteAgentDefinition,
  listAgentDefinitions,
  readAgentPrompt,
  saveAgentDefinition,
  seedAgents,
} from "@/lib/ai";
import { notify } from "@/lib/notify";
import type { SubAgentModelConfig } from "@/lib/settings";
import { AgentEditor, type EditingAgent, emptyAgent } from "./AgentEditor";
import { ModelOverridePanel } from "./ModelOverrides";

interface SubAgentSettingsProps {
  subAgentModels: Record<string, SubAgentModelConfig>;
  onChange: (models: Record<string, SubAgentModelConfig>) => void;
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
        scope: agent.scope === "project" ? "project" : "global",
        isNew: false,
      });
    } catch (err) {
      console.error("Failed to read agent prompt:", err);
      notify.error("Failed to load agent prompt");
    }
  };

  const startCreating = (scope: "global" | "project" = "global") => {
    setEditingAgent(emptyAgent(scope));
  };

  const globalAgents = useMemo(() => agents.filter((a) => a.scope !== "project"), [agents]);
  const projectAgents = useMemo(() => agents.filter((a) => a.scope === "project"), [agents]);

  const cancelEditing = () => {
    setEditingAgent(null);
    setToolInput("");
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
        scope: editingAgent.scope,
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

  if (editingAgent) {
    return (
      <AgentEditor
        editingAgent={editingAgent}
        setEditingAgent={setEditingAgent}
        onSave={handleSave}
        onCancel={cancelEditing}
        savingId={savingId}
        toolInput={toolInput}
        setToolInput={setToolInput}
      />
    );
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="w-5 h-5 text-muted-foreground animate-spin" />
      </div>
    );
  }

  const renderAgentCard = (agent: AgentFileInfo) => {
    const isExpanded = expandedIds.has(agent.id);
    const modelConfig = getModelConfig(agent.id);
    const hasOverride = hasModelOverride(agent.id);

    return (
      <div
        key={agent.id}
        className="rounded-lg bg-muted border border-[var(--border-medium)] overflow-hidden"
      >
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
              <span className="text-[10px] font-mono text-muted-foreground/60">{agent.id}</span>
              {agent.is_system && (
                <Badge variant="outline" className="text-[9px] px-1.5 py-0 h-4">
                  <Lock className="w-2.5 h-2.5 mr-0.5" /> system
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

        {isExpanded && (
          <div className="px-4 pb-4 pt-1 border-t border-[var(--border-medium)] space-y-4">
            <div className="grid grid-cols-4 gap-3 text-xs">
              <div>
                <span className="text-muted-foreground">Model:</span>{" "}
                <span className="font-mono">{agent.model || "inherit"}</span>
              </div>
              <div>
                <span className="text-muted-foreground">Max iter:</span> {agent.max_iterations}
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

            <ModelOverridePanel
              agentId={agent.id}
              modelConfig={modelConfig}
              hasOverride={hasOverride}
              onUpdate={(config) => updateModelOverride(agent.id, config)}
            />

            {(agent.temperature != null || agent.max_tokens != null || agent.top_p != null) && (
              <div className="flex gap-4 text-xs">
                {agent.temperature != null && (
                  <span>
                    <span className="text-muted-foreground">temp:</span> {agent.temperature}
                  </span>
                )}
                {agent.max_tokens != null && (
                  <span>
                    <span className="text-muted-foreground">max_tokens:</span> {agent.max_tokens}
                  </span>
                )}
                {agent.top_p != null && (
                  <span>
                    <span className="text-muted-foreground">top_p:</span> {agent.top_p}
                  </span>
                )}
              </div>
            )}

            {agent.path && (
              <p className="text-[10px] text-muted-foreground/50 font-mono truncate">
                {agent.path}
              </p>
            )}

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
  };

  return (
    <div className="space-y-6">
      {/* Global Agents */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-accent" />
            <h4 className="text-sm font-medium text-accent">Global Agents</h4>
            <span className="text-[10px] text-muted-foreground">~/.golish/agents/</span>
          </div>
          <Button
            size="sm"
            onClick={() => startCreating("global")}
            className="bg-accent text-accent-foreground hover:bg-accent/90 h-7"
          >
            <Plus className="w-3.5 h-3.5 mr-1" /> New
          </Button>
        </div>
        <div className="space-y-2">
          {globalAgents.map(renderAgentCard)}
          {globalAgents.length === 0 && (
            <p className="text-xs text-muted-foreground italic py-3 text-center">
              No global agents. Click &quot;New&quot; or restart to seed defaults.
            </p>
          )}
        </div>
      </div>

      <div className="border-t border-[var(--border-medium)]" />

      {/* Project Agents */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FolderOpen className="w-4 h-4 text-accent" />
            <h4 className="text-sm font-medium text-accent">Project Agents</h4>
            <span className="text-[10px] text-muted-foreground">.golish/agents/</span>
          </div>
          <Button
            size="sm"
            onClick={() => startCreating("project")}
            className="bg-accent text-accent-foreground hover:bg-accent/90 h-7"
          >
            <Plus className="w-3.5 h-3.5 mr-1" /> New
          </Button>
        </div>
        <div className="space-y-2">
          {projectAgents.map(renderAgentCard)}
          {projectAgents.length === 0 && (
            <p className="text-xs text-muted-foreground italic py-3 text-center">
              No project-specific agents. These are stored in your project&apos;s{" "}
              <code>.golish/agents/</code> directory.
            </p>
          )}
        </div>
      </div>

      <div className="text-xs text-muted-foreground border-t border-[var(--border-medium)] pt-4">
        <p>
          <strong>Global</strong> agents are available across all projects. <strong>Project</strong>{" "}
          agents are scoped to the current workspace and override global agents with the same ID.
          System agents (Worker, Memorist, Reflector) can be edited but not deleted.
        </p>
      </div>
    </div>
  );
}
