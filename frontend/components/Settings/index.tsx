import {
  Bell,
  Bot,
  Cog,
  FileCode,
  FolderCode,
  Globe,
  Loader2,
  Paintbrush,
  Puzzle,
  Server,
  Shield,
  Terminal,
  Wrench,
  X,
} from "lucide-react";
import { lazy, Suspense, useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { useSettingsNavigation, type SettingsSection } from "./hooks/useSettingsNavigation";

const AdvancedSettings = lazy(() =>
  import("./AdvancedSettings").then((m) => ({ default: m.AdvancedSettings }))
);
const AgentSettings = lazy(() =>
  import("./AgentSettings").then((m) => ({ default: m.AgentSettings }))
);
const AiSettings = lazy(() => import("./AiSettings").then((m) => ({ default: m.AiSettings })));
const CodebasesSettings = lazy(() =>
  import("./CodebasesSettings").then((m) => ({ default: m.CodebasesSettings }))
);
const EditorSettings = lazy(() =>
  import("./EditorSettings").then((m) => ({ default: m.EditorSettings }))
);
const NotificationsSettings = lazy(() =>
  import("./NotificationsSettings").then((m) => ({ default: m.NotificationsSettings }))
);
const ProviderSettings = lazy(() =>
  import("./ProviderSettings").then((m) => ({ default: m.ProviderSettings }))
);
const TerminalSettings = lazy(() =>
  import("./TerminalSettings").then((m) => ({ default: m.TerminalSettings }))
);
const AppearanceSettings = lazy(() =>
  import("./AppearanceSettings").then((m) => ({ default: m.AppearanceSettings }))
);
const McpSettings = lazy(() => import("./McpSettings").then((m) => ({ default: m.McpSettings })));
const NetworkSettings = lazy(() =>
  import("./NetworkSettings").then((m) => ({ default: m.NetworkSettings }))
);
const PentestEnvSettings = lazy(() =>
  import("./PentestEnvSettings").then((m) => ({ default: m.PentestEnvSettings }))
);

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

interface NavItemDef {
  id: SettingsSection;
  labelKey: string;
  icon: React.ReactNode;
  descKey: string;
}

const NAV_ITEM_DEFS: NavItemDef[] = [
  { id: "pentest", labelKey: "settings.environment", icon: <Wrench className="w-4 h-4" />, descKey: "settings.envDescription" },
  { id: "providers", labelKey: "settings.providers", icon: <Server className="w-4 h-4" />, descKey: "settings.providersDesc" },
  { id: "ai", labelKey: "settings.aiModels", icon: <Bot className="w-4 h-4" />, descKey: "settings.aiModelsDesc" },
  { id: "terminal", labelKey: "settings.terminal", icon: <Terminal className="w-4 h-4" />, descKey: "settings.terminal" },
  { id: "editor", labelKey: "settings.editor", icon: <FileCode className="w-4 h-4" />, descKey: "settings.editor" },
  { id: "agent", labelKey: "settings.agent", icon: <Cog className="w-4 h-4" />, descKey: "settings.agent" },
  { id: "mcp", labelKey: "settings.mcp", icon: <Puzzle className="w-4 h-4" />, descKey: "settings.mcp" },
  { id: "codebases", labelKey: "settings.codebases", icon: <FolderCode className="w-4 h-4" />, descKey: "settings.codebases" },
  { id: "network", labelKey: "settings.network", icon: <Globe className="w-4 h-4" />, descKey: "settings.network" },
  { id: "notifications", labelKey: "settings.notifications", icon: <Bell className="w-4 h-4" />, descKey: "settings.notifications" },
  { id: "appearance", labelKey: "settings.appearance", icon: <Paintbrush className="w-4 h-4" />, descKey: "settings.appearance" },
  { id: "advanced", labelKey: "settings.advanced", icon: <Shield className="w-4 h-4" />, descKey: "settings.advanced" },
];

export function SettingsNav({
  activeSection,
  onSectionChange,
}: {
  activeSection: string;
  onSectionChange: (section: SettingsSection) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col h-full">
      <div className="h-[34px] flex items-center px-3 flex-shrink-0">
        <span className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">{t("settings.title")}</span>
      </div>
      <div className="flex-1 overflow-y-auto px-1.5 py-1">
        {NAV_ITEM_DEFS.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => onSectionChange(item.id)}
            className={cn(
              "w-full flex items-center gap-2.5 px-3 py-2 text-left transition-all rounded-lg mb-0.5",
              activeSection === item.id
                ? "bg-[var(--bg-hover)] text-foreground"
                : "text-muted-foreground hover:bg-[var(--bg-hover)] hover:text-foreground"
            )}
          >
            <span className={cn(activeSection === item.id ? "text-accent" : "")}>
              {item.icon}
            </span>
            <span className="text-[12px] font-medium">{t(item.labelKey)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

export function SettingsContent({
  activeSection: activeSectionProp,
}: {
  activeSection?: string;
}) {
  const {
    settings, activeSection, isLoading, loadSettings,
    updateSection, handleNetworkChange,
  } = useSettingsNavigation(activeSectionProp);

  useEffect(() => { loadSettings(); }, [loadSettings]);

  const renderContent = useCallback(() => {
    if (activeSection === "pentest") {
      return <PentestEnvSettings />;
    }
    if (!settings) return null;
    switch (activeSection) {
      case "providers":
        return <ProviderSettings settings={settings.ai} onChange={(ai) => updateSection("ai", ai)} />;
      case "ai":
        return (
          <AiSettings
            apiKeys={settings.api_keys}
            sidecarSettings={settings.sidecar}
            onApiKeysChange={(keys) => updateSection("api_keys", keys)}
            onSidecarChange={(sidecar) => updateSection("sidecar", sidecar)}
          />
        );
      case "terminal":
        return <TerminalSettings settings={settings.terminal} onChange={(terminal) => updateSection("terminal", terminal)} />;
      case "editor":
        return <EditorSettings />;
      case "agent":
        return (
          <AgentSettings
            settings={settings.agent}
            toolsSettings={settings.tools}
            subAgentModels={settings.ai.sub_agent_models || {}}
            onChange={(agent) => updateSection("agent", agent)}
            onToolsChange={(tools) => updateSection("tools", tools)}
            onSubAgentModelsChange={(models) => updateSection("ai", { ...settings.ai, sub_agent_models: models })}
          />
        );
      case "mcp":
        return <McpSettings />;
      case "codebases":
        return <CodebasesSettings />;
      case "network":
        return <NetworkSettings settings={settings.network} onChange={handleNetworkChange} />;
      case "notifications":
        return <NotificationsSettings settings={settings.notifications} onChange={(notifications) => updateSection("notifications", notifications)} />;
      case "appearance":
        return <AppearanceSettings terminalSettings={settings.terminal} onTerminalChange={(terminal) => updateSection("terminal", terminal)} />;
      case "advanced":
        return (
          <AdvancedSettings
            settings={settings.advanced}
            privacy={settings.privacy}
            onChange={(advanced) => updateSection("advanced", advanced)}
            onPrivacyChange={(privacy) => updateSection("privacy", privacy)}
          />
        );
    }
  }, [activeSection, settings, updateSection, handleNetworkChange]);

  if (isLoading) {
    return (
      <div className="flex-1 flex items-center justify-center h-full">
        <Loader2 className="w-6 h-6 text-muted-foreground animate-spin" />
      </div>
    );
  }

  if (!settings) {
    return (
      <div className="flex-1 flex items-center justify-center h-full">
        <span className="text-destructive text-[13px]">Failed to load settings</span>
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className={cn("p-6", (activeSection === "pentest" || activeSection === "providers") ? "" : "max-w-3xl")}>
        <Suspense
          fallback={
            <div className="flex items-center justify-center py-8">
              <Loader2 className="w-6 h-6 text-muted-foreground animate-spin" />
            </div>
          }
        >
          {renderContent()}
        </Suspense>
      </div>
    </ScrollArea>
  );
}

function SettingsDialogNav({ activeSection, onSectionChange }: { activeSection: SettingsSection; onSectionChange: (s: SettingsSection) => void }) {
  const { t } = useTranslation();
  return (
    <nav className="w-52 bg-card rounded-xl flex flex-col flex-shrink-0 panel-float overflow-hidden">
      <div className="flex-1 py-2 px-1.5 overflow-y-auto">
        {NAV_ITEM_DEFS.map((item) => (
          <button
            key={item.id}
            type="button"
            onClick={() => onSectionChange(item.id)}
            className={cn(
              "w-full flex items-start gap-2.5 px-3 py-2.5 text-left transition-all rounded-lg mb-0.5",
              activeSection === item.id
                ? "bg-[var(--bg-hover)] text-foreground"
                : "text-muted-foreground hover:bg-[var(--bg-hover)] hover:text-foreground"
            )}
          >
            <span className={cn("mt-0.5", activeSection === item.id ? "text-accent" : "")}>
              {item.icon}
            </span>
            <div className="flex-1 min-w-0">
              <div className="text-[13px] font-medium">{t(item.labelKey)}</div>
              <div className="text-[11px] text-muted-foreground/60 mt-0.5">{t(item.descKey)}</div>
            </div>
          </button>
        ))}
      </div>
    </nav>
  );
}

export function SettingsDialog({ open, onOpenChange }: SettingsDialogProps) {
  const {
    settings, activeSection, setActiveSection, isLoading,
    loadSettings, updateSection, handleNetworkChange,
  } = useSettingsNavigation();

  useEffect(() => {
    if (open) loadSettings();
  }, [open, loadSettings]);

  const handleClose = useCallback(() => {
    onOpenChange(false);
  }, [onOpenChange]);

  const renderContent = useCallback(() => {
    if (activeSection === "pentest") {
      return <PentestEnvSettings />;
    }
    if (!settings) return null;

    switch (activeSection) {
      case "providers":
        return <ProviderSettings settings={settings.ai} onChange={(ai) => updateSection("ai", ai)} />;
      case "ai":
        return (
          <AiSettings
            apiKeys={settings.api_keys}
            sidecarSettings={settings.sidecar}
            onApiKeysChange={(keys) => updateSection("api_keys", keys)}
            onSidecarChange={(sidecar) => updateSection("sidecar", sidecar)}
          />
        );
      case "terminal":
        return <TerminalSettings settings={settings.terminal} onChange={(terminal) => updateSection("terminal", terminal)} />;
      case "editor":
        return <EditorSettings />;
      case "agent":
        return (
          <AgentSettings
            settings={settings.agent}
            toolsSettings={settings.tools}
            subAgentModels={settings.ai.sub_agent_models || {}}
            onChange={(agent) => updateSection("agent", agent)}
            onToolsChange={(tools) => updateSection("tools", tools)}
            onSubAgentModelsChange={(models) => updateSection("ai", { ...settings.ai, sub_agent_models: models })}
          />
        );
      case "mcp":
        return <McpSettings />;
      case "codebases":
        return <CodebasesSettings />;
      case "network":
        return <NetworkSettings settings={settings.network} onChange={handleNetworkChange} />;
      case "notifications":
        return <NotificationsSettings settings={settings.notifications} onChange={(notifications) => updateSection("notifications", notifications)} />;
      case "appearance":
        return <AppearanceSettings terminalSettings={settings.terminal} onTerminalChange={(terminal) => updateSection("terminal", terminal)} />;
      case "advanced":
        return (
          <AdvancedSettings
            settings={settings.advanced}
            privacy={settings.privacy}
            onChange={(advanced) => updateSection("advanced", advanced)}
            onPrivacyChange={(privacy) => updateSection("privacy", privacy)}
          />
        );
    }
  }, [activeSection, settings, updateSection, handleNetworkChange]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="!max-w-none !inset-0 !translate-x-0 !translate-y-0 !w-screen !h-screen p-0 bg-background border-0 rounded-none text-foreground flex flex-col overflow-hidden"
      >
        <DialogTitle className="sr-only">Settings</DialogTitle>

        {/* Header - macOS traffic lights + title */}
        <div className="flex items-center justify-between px-6 h-[38px] flex-shrink-0 titlebar-drag" data-tauri-drag-region>
          <h2 className="text-[14px] font-semibold text-foreground titlebar-no-drag">Settings</h2>
          <button
            type="button"
            onClick={handleClose}
            className="p-1.5 rounded-lg hover:bg-[var(--bg-hover)] text-muted-foreground hover:text-foreground transition-colors titlebar-no-drag"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {isLoading ? (
          <div className="flex-1 flex items-center justify-center">
            <Loader2 className="w-6 h-6 text-muted-foreground animate-spin" />
          </div>
        ) : settings ? (
          <div className="flex-1 flex gap-1.5 px-1.5 pb-1.5 min-h-0 overflow-hidden">
            {/* Sidebar Navigation */}
            <SettingsDialogNav activeSection={activeSection} onSectionChange={setActiveSection} />

            {/* Main Content */}
            <div className="flex-1 flex flex-col min-w-0 min-h-0 overflow-hidden bg-card rounded-xl panel-float">
              <ScrollArea className="h-full">
                <div className={cn("p-6", (activeSection === "pentest" || activeSection === "providers") ? "" : "max-w-3xl")}>
                  <Suspense
                    fallback={
                      <div className="flex items-center justify-center py-8">
                        <Loader2 className="w-6 h-6 text-muted-foreground animate-spin" />
                      </div>
                    }
                  >
                    {renderContent()}
                  </Suspense>
                </div>
              </ScrollArea>
            </div>
          </div>
        ) : (
          <div className="flex-1 flex items-center justify-center">
            <span className="text-destructive">Failed to load settings</span>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
