export interface CodebaseConfig {
  path: string;
  memory_file?: string;
}

export interface GolishSettings {
  version: number;
  ai: AiSettings;
  api_keys: ApiKeysSettings;
  ui: UiSettings;
  terminal: TerminalSettings;
  agent: AgentSettings;
  tools: ToolsSettings;
  mcp_servers: Record<string, McpServerConfig>;
  trust: TrustSettings;
  privacy: PrivacySettings;
  advanced: AdvancedSettings;
  sidecar: SidecarSettings;
  network: NetworkSettings;
  notifications: NotificationsSettings;
  /** @deprecated Use `codebases` instead */
  indexed_codebases: string[];
  codebases: CodebaseConfig[];
}

export interface ToolsSettings {
  web_search: boolean;
}

export type ReasoningEffort = "low" | "medium" | "high" | "extra_high";

export interface SubAgentModelConfig {
  provider?: AiProvider;
  model?: string;
  temperature?: number;
  max_tokens?: number;
  top_p?: number;
}

export interface AiSettings {
  default_provider: AiProvider;
  default_model: string;
  default_reasoning_effort?: ReasoningEffort;
  sub_agent_models: Record<string, SubAgentModelConfig>;
  research_provider?: AiProvider;
  research_model?: string;
  vertex_ai: VertexAiSettings;
  vertex_gemini: VertexGeminiSettings;
  openrouter: OpenRouterSettings;
  anthropic: AnthropicSettings;
  openai: OpenAiSettings;
  ollama: OllamaSettings;
  gemini: GeminiSettings;
  groq: GroqSettings;
  xai: XaiSettings;
  zai_sdk: ZaiSdkSettings;
  nvidia: NvidiaSettings;
}

export type AiProvider =
  | "vertex_ai"
  | "vertex_gemini"
  | "openrouter"
  | "anthropic"
  | "openai"
  | "ollama"
  | "gemini"
  | "groq"
  | "xai"
  | "zai_sdk"
  | "nvidia";

export interface VertexAiSettings {
  credentials_path: string | null;
  project_id: string | null;
  location: string | null;
  show_in_selector: boolean;
}

export interface VertexGeminiSettings {
  credentials_path: string | null;
  project_id: string | null;
  location: string | null;
  show_in_selector: boolean;
}

export interface OpenRouterSettings {
  api_key: string | null;
  show_in_selector: boolean;
  provider_preferences?: OpenRouterProviderPreferences | null;
}

export interface OpenRouterProviderPreferences {
  order?: string[] | null;
  only?: string[] | null;
  ignore?: string[] | null;
  allow_fallbacks?: boolean | null;
  require_parameters?: boolean | null;
  data_collection?: string | null;
  zdr?: boolean | null;
  sort?: string | null;
  preferred_min_throughput?: number | null;
  preferred_max_latency?: number | null;
  max_price_prompt?: number | null;
  max_price_completion?: number | null;
  quantizations?: string[] | null;
}

export interface AnthropicSettings {
  api_key: string | null;
  show_in_selector: boolean;
}

export type WebSearchContextSize = "low" | "medium" | "high";

export interface OpenAiSettings {
  api_key: string | null;
  base_url: string | null;
  show_in_selector: boolean;
  enable_web_search: boolean;
  web_search_context_size: WebSearchContextSize;
}

export interface OllamaSettings {
  base_url: string;
  show_in_selector: boolean;
}

export interface GeminiSettings {
  api_key: string | null;
  show_in_selector: boolean;
}

export interface GroqSettings {
  api_key: string | null;
  show_in_selector: boolean;
}

export interface XaiSettings {
  api_key: string | null;
  show_in_selector: boolean;
}

export interface ZaiSdkSettings {
  api_key: string | null;
  base_url: string | null;
  model: string | null;
  show_in_selector: boolean;
}

export interface NvidiaSettings {
  api_key: string | null;
  base_url: string | null;
  show_in_selector: boolean;
}

export interface ApiKeysSettings {
  tavily: string | null;
  github: string | null;
  brave: string | null;
}

export interface UiSettings {
  theme: "dark" | "light" | "system";
  show_tips: boolean;
  hide_banner: boolean;
  window: WindowSettings;
}

export interface WindowSettings {
  width: number;
  height: number;
  x: number | null;
  y: number | null;
  maximized: boolean;
}

export interface CaretSettings {
  style: "block" | "default";
  width: number;
  color: string | null;
  blink_speed: number;
  opacity: number;
}

export const DEFAULT_CARET_SETTINGS: CaretSettings = {
  style: "default",
  width: 1.0,
  color: null,
  blink_speed: 530,
  opacity: 1.0,
};

export interface TerminalSettings {
  shell: string | null;
  font_family: string;
  font_size: number;
  scrollback: number;
  fullterm_commands: string[];
  caret: CaretSettings;
}

export interface AgentSettings {
  session_persistence: boolean;
  session_retention_days: number;
  pattern_learning: boolean;
  min_approvals_for_auto: number;
  approval_threshold: number;
}

export interface McpServerConfig {
  command: string | null;
  args: string[];
  env: Record<string, string>;
  url: string | null;
}

export interface TrustSettings {
  full_trust: string[];
  read_only_trust: string[];
  never_trust: string[];
}

export interface PrivacySettings {
  usage_statistics: boolean;
  log_prompts: boolean;
}

export interface AdvancedSettings {
  enable_experimental: boolean;
  log_level: "error" | "warn" | "info" | "debug" | "trace";
  enable_llm_api_logs: boolean;
  extract_raw_sse: boolean;
}

export interface SidecarSettings {
  enabled: boolean;
  synthesis_enabled: boolean;
  synthesis_backend: SynthesisBackendType;
  synthesis_vertex: SynthesisVertexSettings;
  synthesis_openai: SynthesisOpenAiSettings;
  synthesis_grok: SynthesisGrokSettings;
  retention_days: number;
  capture_tool_calls: boolean;
  capture_reasoning: boolean;
}

export type SynthesisBackendType = "local" | "vertex_anthropic" | "openai" | "grok" | "template";

export interface SynthesisVertexSettings {
  project_id: string | null;
  location: string | null;
  model: string;
  credentials_path: string | null;
}

export interface SynthesisOpenAiSettings {
  api_key: string | null;
  model: string;
  base_url: string | null;
}

export interface SynthesisGrokSettings {
  api_key: string | null;
  model: string;
}

export interface NetworkSettings {
  proxy_url: string | null;
  no_proxy: string | null;
  github_token: string | null;
}

export interface NotificationsSettings {
  native_enabled: boolean;
  sound_enabled: boolean;
  sound: string | null;
}

export interface TelemetryStats {
  spans_started: number;
  spans_ended: number;
  started_at: number;
}

export interface ProviderVisibility {
  vertex_ai: boolean;
  vertex_gemini: boolean;
  openrouter: boolean;
  openai: boolean;
  anthropic: boolean;
  ollama: boolean;
  gemini: boolean;
  groq: boolean;
  xai: boolean;
  zai_sdk: boolean;
  nvidia: boolean;
}
