import { Loader2, Save, X, Globe, FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";

export interface EditingAgent {
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
  scope: "global" | "project";
  isNew: boolean;
}

export function emptyAgent(scope: "global" | "project" = "global"): EditingAgent {
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
    scope,
    isNew: true,
  };
}

interface AgentEditorProps {
  editingAgent: EditingAgent;
  setEditingAgent: (agent: EditingAgent) => void;
  onSave: () => void;
  onCancel: () => void;
  savingId: string | null;
  toolInput: string;
  setToolInput: (v: string) => void;
}

export function AgentEditor({
  editingAgent, setEditingAgent, onSave, onCancel, savingId, toolInput, setToolInput,
}: AgentEditorProps) {
  const addTool = () => {
    if (!toolInput.trim()) return;
    const tool = toolInput.trim();
    if (!editingAgent.allowedTools.includes(tool)) {
      setEditingAgent({ ...editingAgent, allowedTools: [...editingAgent.allowedTools, tool] });
    }
    setToolInput("");
  };

  const removeTool = (tool: string) => {
    setEditingAgent({
      ...editingAgent,
      allowedTools: editingAgent.allowedTools.filter((t) => t !== tool),
    });
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium text-accent">
          {editingAgent.isNew ? "Create New Agent" : `Edit: ${editingAgent.name}`}
        </h4>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            <X className="w-4 h-4 mr-1" /> Cancel
          </Button>
          <Button
            size="sm"
            onClick={onSave}
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

        <div className="grid grid-cols-[1fr_auto] gap-3">
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">Description</label>
            <Input
              value={editingAgent.description}
              onChange={(e) => setEditingAgent({ ...editingAgent, description: e.target.value })}
              placeholder="What this agent specializes in..."
              className="bg-background border-border text-foreground h-9"
            />
          </div>
          <div className="space-y-1">
            <label className="text-xs text-muted-foreground">Scope</label>
            <div className="flex gap-1 h-9 items-center">
              <Button
                variant={editingAgent.scope === "global" ? "default" : "outline"}
                size="sm"
                onClick={() => setEditingAgent({ ...editingAgent, scope: "global" })}
                className={`h-8 px-3 text-xs gap-1 ${editingAgent.scope === "global" ? "bg-accent text-accent-foreground" : ""}`}
              >
                <Globe className="w-3 h-3" /> Global
              </Button>
              <Button
                variant={editingAgent.scope === "project" ? "default" : "outline"}
                size="sm"
                onClick={() => setEditingAgent({ ...editingAgent, scope: "project" })}
                className={`h-8 px-3 text-xs gap-1 ${editingAgent.scope === "project" ? "bg-accent text-accent-foreground" : ""}`}
              >
                <FolderOpen className="w-3 h-3" /> Project
              </Button>
            </div>
          </div>
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
