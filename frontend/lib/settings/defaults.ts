import type { GolishSettings } from "./types";

export const DEFAULT_SETTINGS: GolishSettings = {
  version: 1,
  ai: {
    default_provider: "vertex_ai",
    default_model: "claude-opus-4-5@20251101",
    default_reasoning_effort: undefined,
    sub_agent_models: {},
    vertex_ai: {
      credentials_path: null,
      project_id: null,
      location: null,
      show_in_selector: true,
    },
    vertex_gemini: {
      credentials_path: null,
      project_id: null,
      location: null,
      show_in_selector: true,
    },
    openrouter: {
      api_key: null,
      show_in_selector: true,
    },
    anthropic: {
      api_key: null,
      show_in_selector: true,
    },
    openai: {
      api_key: null,
      base_url: null,
      show_in_selector: true,
      enable_web_search: false,
      web_search_context_size: "medium",
    },
    ollama: {
      base_url: "http://localhost:11434",
      show_in_selector: true,
    },
    gemini: {
      api_key: null,
      show_in_selector: true,
    },
    groq: {
      api_key: null,
      show_in_selector: true,
    },
    xai: {
      api_key: null,
      show_in_selector: true,
    },
    zai_sdk: {
      api_key: null,
      base_url: null,
      model: null,
      show_in_selector: true,
    },
    nvidia: {
      api_key: null,
      base_url: null,
      show_in_selector: true,
    },
  },
  api_keys: {
    tavily: null,
    github: null,
    brave: null,
  },
  ui: {
    theme: "dark",
    show_tips: true,
    hide_banner: false,
    window: {
      width: 1400,
      height: 900,
      x: null,
      y: null,
      maximized: false,
    },
  },
  terminal: {
    shell: null,
    font_family: "SF Mono",
    font_size: 14,
    scrollback: 10000,
    fullterm_commands: [],
    caret: {
      style: "default",
      width: 1.0,
      color: null,
      blink_speed: 530,
      opacity: 1.0,
    },
  },
  agent: {
    session_persistence: true,
    session_retention_days: 30,
    pattern_learning: true,
    min_approvals_for_auto: 3,
    approval_threshold: 0.8,
  },
  tools: {
    web_search: false,
  },
  mcp_servers: {},
  trust: {
    full_trust: [],
    read_only_trust: [],
    never_trust: [],
  },
  privacy: {
    usage_statistics: false,
    log_prompts: false,
  },
  advanced: {
    enable_experimental: false,
    log_level: "info",
    enable_llm_api_logs: false,
    extract_raw_sse: false,
  },
  sidecar: {
    enabled: false,
    synthesis_enabled: true,
    synthesis_backend: "template",
    synthesis_vertex: {
      project_id: null,
      location: null,
      model: "claude-sonnet-4-5-20250514",
      credentials_path: null,
    },
    synthesis_openai: {
      api_key: null,
      model: "gpt-4o-mini",
      base_url: null,
    },
    synthesis_grok: {
      api_key: null,
      model: "grok-2",
    },
    retention_days: 30,
    capture_tool_calls: true,
    capture_reasoning: true,
  },
  network: {
    proxy_url: null,
    no_proxy: null,
    github_token: null,
  },
  notifications: {
    native_enabled: false,
    sound_enabled: true,
    sound: null,
  },
  indexed_codebases: [],
  codebases: [],
};
