# Dynamic Prompt Contribution System

This document explains the dynamic prompt contribution system that allows the system prompt to be composed from multiple contributors at runtime.

## Overview

The Golish agent uses a dynamic prompt composition system that builds the system prompt from:

1. **Base prompt** - Core identity, workflow, tool documentation, and security rules
2. **Agent mode** - Mode-specific instructions (Default, Planning, AutoApprove)
3. **Project instructions** - From CLAUDE.md or configured memory file
4. **Dynamic contributions** - From registered prompt contributors

This ensures the agent's system prompt is composed consistently from the same building blocks, testing real production behavior.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     System Prompt Builder                        │
│  (build_system_prompt_with_contributions)                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────────┐  │
│  │ Base Prompt  │  │ Agent Mode   │  │ Project Instructions  │  │
│  │              │  │ Instructions │  │ (CLAUDE.md)           │  │
│  └──────────────┘  └──────────────┘  └───────────────────────┘  │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │            PromptContributorRegistry                      │   │
│  │  ┌────────────────────┐  ┌─────────────────────────────┐ │   │
│  │  │ SubAgentPrompt     │  │ ProviderBuiltinTools        │ │   │
│  │  │ Contributor        │  │ Contributor                 │ │   │
│  │  └────────────────────┘  └─────────────────────────────┘ │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Key Components

### PromptContext (`golish-core/src/prompt.rs`)

Context passed to prompt contributors for conditional generation:

```rust
pub struct PromptContext {
    pub provider: String,           // e.g., "anthropic", "openai"
    pub model: String,              // e.g., "claude-sonnet-4"
    pub available_tools: Vec<String>,
    pub has_web_search: bool,       // Tavily or provider-specific
    pub has_native_web_tools: bool, // Claude's web_search/web_fetch
    pub has_sub_agents: bool,       // Sub-agents available
    pub workspace: Option<String>,
}
```

### PromptContributor Trait (`golish-core/src/prompt.rs`)

Trait for components that contribute to the system prompt:

```rust
pub trait PromptContributor: Send + Sync {
    fn contribute(&self, ctx: &PromptContext) -> Option<Vec<PromptSection>>;
    fn name(&self) -> &str;
}
```

### PromptContributorRegistry (`golish-ai/src/prompt_registry.rs`)

Collects and aggregates prompt sections from contributors:

```rust
pub struct PromptContributorRegistry {
    contributors: Vec<Arc<dyn PromptContributor>>,
}

impl PromptContributorRegistry {
    pub fn register(&mut self, contributor: Arc<dyn PromptContributor>);
    pub fn build_prompt(&self, ctx: &PromptContext) -> String;
}
```

### Default Contributors (`golish-ai/src/contributors/`)

Two default contributors are provided:

1. **SubAgentPromptContributor** - Adds documentation for available sub-agents
2. **ProviderBuiltinToolsContributor** - Adds provider-specific tool instructions (e.g., Anthropic web search, OpenAI web search)

## Usage

### Main Agent (agent_bridge.rs)

The main agent builds prompts with contributions in `prepare_execution_context()`:

```rust
// Create prompt contributor registry with default contributors
let contributors = create_default_contributors(self.sub_agent_registry.clone());
let mut registry = PromptContributorRegistry::new();
for contributor in contributors {
    registry.register(contributor);
}

// Create prompt context with provider, model, and feature flags
let has_web_search = self.tavily_state.is_some();
let has_sub_agents = true;
let prompt_context = PromptContext::new(&self.provider_name, &self.model_name)
    .with_web_search(has_web_search)
    .with_sub_agents(has_sub_agents)
    .with_workspace(workspace_path.display().to_string());

// Build the prompt with contributions
let system_prompt = build_system_prompt_with_contributions(
    &workspace_path,
    agent_mode,
    memory_file_path.as_deref(),
    Some(&registry),
    Some(&prompt_context),
);
```

## Prompt Priority

Contributions are ordered by priority (lower values appear first):

| Priority | Value | Description |
|----------|-------|-------------|
| Core | 0 | Identity and environment |
| Workflow | 100 | Workflow and behavior rules |
| Tools | 200 | Tool documentation |
| Features | 300 | Feature-specific instructions |
| Provider | 400 | Provider-specific instructions |
| Context | 500 | Dynamic runtime context |

## Adding a New Contributor

1. Create a struct implementing `PromptContributor`:

```rust
pub struct MyContributor;

impl PromptContributor for MyContributor {
    fn contribute(&self, ctx: &PromptContext) -> Option<Vec<PromptSection>> {
        // Return None if nothing to contribute for this context
        if !ctx.has_web_search {
            return None;
        }

        Some(vec![PromptSection::new(
            "my-section",
            PromptPriority::Features,
            "My custom prompt content...",
        )])
    }

    fn name(&self) -> &str {
        "MyContributor"
    }
}
```

2. Register it in `create_default_contributors()` or add it manually to the registry.

## Related Files

- `golish-core/src/prompt.rs` - Core types (PromptContext, PromptContributor, PromptSection)
- `golish-ai/src/prompt_registry.rs` - PromptContributorRegistry
- `golish-ai/src/contributors/mod.rs` - Default contributors factory
- `golish-ai/src/contributors/sub_agents.rs` - SubAgentPromptContributor
- `golish-ai/src/contributors/provider_tools.rs` - ProviderBuiltinToolsContributor
- `golish-ai/src/system_prompt.rs` - System prompt building functions
- `golish-ai/src/agent_bridge.rs` - Main agent prompt composition
