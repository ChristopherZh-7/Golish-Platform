import {
  Bot,
  Code2,
  FileText,
  Search,
  Settings2,
  Shield,
  Terminal,
} from "lucide-react";

const AGENT_COLORS: Record<string, string> = {
  planner: "var(--ansi-blue)",
  coder: "var(--ansi-green)",
  researcher: "var(--ansi-yellow)",
  reviewer: "var(--ansi-cyan)",
  explorer: "var(--ansi-yellow)",
  analyst: "var(--ansi-cyan)",
  adviser: "var(--ansi-cyan)",
  reporter: "#10b981",
  pentester: "var(--ansi-red)",
  memorist: "var(--ansi-blue)",
  reflector: "var(--ansi-magenta)",
};

const AGENT_ICONS: Record<string, typeof Bot> = {
  coder: Code2,
  researcher: Search,
  explorer: Search,
  planner: Settings2,
  adviser: Shield,
  reporter: FileText,
  pentester: Terminal,
};

export function getAgentColor(agentName: string): string {
  const lower = agentName.toLowerCase();
  for (const [key, color] of Object.entries(AGENT_COLORS)) {
    if (lower.includes(key)) return color;
  }
  return "var(--ansi-magenta)";
}

export function getAgentIcon(agentName: string): typeof Bot {
  const lower = agentName.toLowerCase();
  for (const [key, icon] of Object.entries(AGENT_ICONS)) {
    if (lower.includes(key)) return icon;
  }
  return Bot;
}
