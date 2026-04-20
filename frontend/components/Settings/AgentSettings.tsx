import { useState } from "react";
import { Bot, BookOpen, FileText, Settings2 } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import type {
  AgentSettings as AgentSettingsType,
  SubAgentModelConfig,
  ToolsSettings,
} from "@/lib/settings";
import { SubAgentSettings } from "./SubAgentSettings";
import { SkillsSettings } from "./SkillsSettings";
import { RulesSettings } from "./RulesSettings";
import { cn } from "@/lib/utils";

interface AgentSettingsProps {
  settings: AgentSettingsType;
  toolsSettings: ToolsSettings;
  subAgentModels: Record<string, SubAgentModelConfig>;
  onChange: (settings: AgentSettingsType) => void;
  onToolsChange: (tools: ToolsSettings) => void;
  onSubAgentModelsChange: (models: Record<string, SubAgentModelConfig>) => void;
}

type AgentTab = "general" | "agents" | "skills" | "rules";

const TABS: { id: AgentTab; label: string; icon: React.ReactNode }[] = [
  { id: "general", label: "General", icon: <Settings2 className="w-3.5 h-3.5" /> },
  { id: "agents", label: "Agents", icon: <Bot className="w-3.5 h-3.5" /> },
  { id: "skills", label: "Skills", icon: <BookOpen className="w-3.5 h-3.5" /> },
  { id: "rules", label: "Rules", icon: <FileText className="w-3.5 h-3.5" /> },
];

export function AgentSettings({
  settings,
  toolsSettings,
  subAgentModels,
  onChange,
  onToolsChange,
  onSubAgentModelsChange,
}: AgentSettingsProps) {
  const [activeTab, setActiveTab] = useState<AgentTab>("general");

  const updateField = <K extends keyof AgentSettingsType>(key: K, value: AgentSettingsType[K]) => {
    onChange({ ...settings, [key]: value });
  };

  const updateToolsField = <K extends keyof ToolsSettings>(key: K, value: ToolsSettings[K]) => {
    onToolsChange({ ...toolsSettings, [key]: value });
  };

  return (
    <div className="space-y-6">
      {/* Tab bar */}
      <div className="flex gap-1 p-1 rounded-lg bg-background border border-[var(--border-medium)]">
        {TABS.map((tab) => (
          <button
            key={tab.id}
            type="button"
            onClick={() => setActiveTab(tab.id)}
            className={cn(
              "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-all",
              activeTab === tab.id
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
          >
            {tab.icon}
            {tab.label}
          </button>
        ))}
      </div>

      {/* General tab */}
      {activeTab === "general" && (
        <div className="space-y-8">
          <div className="flex items-center justify-between">
            <div className="space-y-1">
              <label
                htmlFor="agent-session-persistence"
                className="text-sm font-medium text-foreground"
              >
                Session Persistence
              </label>
              <p className="text-xs text-muted-foreground">Auto-save conversations to disk</p>
            </div>
            <Switch
              id="agent-session-persistence"
              checked={settings.session_persistence}
              onCheckedChange={(checked) => updateField("session_persistence", checked)}
            />
          </div>

          <div className="space-y-2">
            <label htmlFor="agent-session-retention" className="text-sm font-medium text-foreground">
              Session Retention (days)
            </label>
            <Input
              id="agent-session-retention"
              type="number"
              min={0}
              max={365}
              value={settings.session_retention_days}
              onChange={(e) => updateField("session_retention_days", parseInt(e.target.value, 10) || 0)}
              className="w-24"
            />
            <p className="text-xs text-muted-foreground">
              How long to keep saved sessions (0 = forever)
            </p>
          </div>

          <div className="flex items-center justify-between">
            <div className="space-y-1">
              <label htmlFor="agent-pattern-learning" className="text-sm font-medium text-foreground">
                Pattern Learning
              </label>
              <p className="text-xs text-muted-foreground">Learn from approvals for auto-approval</p>
            </div>
            <Switch
              id="agent-pattern-learning"
              checked={settings.pattern_learning}
              onCheckedChange={(checked) => updateField("pattern_learning", checked)}
            />
          </div>

          <div className="space-y-2">
            <label htmlFor="agent-min-approvals" className="text-sm font-medium text-foreground">
              Minimum Approvals
            </label>
            <Input
              id="agent-min-approvals"
              type="number"
              min={1}
              max={10}
              value={settings.min_approvals_for_auto}
              onChange={(e) => updateField("min_approvals_for_auto", parseInt(e.target.value, 10) || 3)}
              className="w-24"
            />
            <p className="text-xs text-muted-foreground">
              Minimum approvals before a tool can be auto-approved
            </p>
          </div>

          <div className="space-y-2">
            <label htmlFor="agent-approval-threshold" className="text-sm font-medium text-foreground">
              Approval Threshold: {(settings.approval_threshold * 100).toFixed(0)}%
            </label>
            <input
              id="agent-approval-threshold"
              type="range"
              min={0}
              max={100}
              value={settings.approval_threshold * 100}
              onChange={(e) => updateField("approval_threshold", parseInt(e.target.value, 10) / 100)}
              className="w-full h-2 bg-muted rounded-lg appearance-none cursor-pointer accent-accent"
            />
            <p className="text-xs text-muted-foreground">Required approval rate for auto-approval</p>
          </div>

          <div className="border-t border-[var(--border-medium)]" />

          <div className="space-y-4">
            <h3 className="text-sm font-medium text-foreground">Tools</h3>
            <div className="flex items-center justify-between">
              <div className="space-y-1">
                <label htmlFor="tools-web-search" className="text-sm font-medium text-foreground">
                  Web Search (Tavily)
                </label>
                <p className="text-xs text-muted-foreground">
                  Enable web search, extract, crawl, and map tools. Requires TAVILY_API_KEY.
                </p>
              </div>
              <Switch
                id="tools-web-search"
                checked={toolsSettings.web_search}
                onCheckedChange={(checked) => updateToolsField("web_search", checked)}
              />
            </div>
          </div>
        </div>
      )}

      {/* Agents tab */}
      {activeTab === "agents" && (
        <SubAgentSettings subAgentModels={subAgentModels} onChange={onSubAgentModelsChange} />
      )}

      {/* Skills tab */}
      {activeTab === "skills" && <SkillsSettings />}

      {/* Rules tab */}
      {activeTab === "rules" && <RulesSettings />}
    </div>
  );
}
