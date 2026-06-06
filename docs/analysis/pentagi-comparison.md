# Golish Platform vs PentAGI — Detailed Comparison

## 1. Overview

| Aspect | Golish Platform | PentAGI |
|--------|----------------|---------|
| **Purpose** | AI-powered terminal emulator with security testing capabilities | Automated penetration testing platform driven by AI agents |
| **Architecture** | Desktop app (Tauri 2: Rust backend + React 19 frontend) | Web app (Go backend + React/TypeScript frontend, Docker-deployed) |
| **Backend Language** | Rust (28 workspace crates, 4-layer architecture) | Go (single module, ~18 packages) |
| **Frontend Framework** | React 19 + Vite + Tailwind v4 + shadcn/ui | React + Vite + Tailwind + shadcn/ui |
| **State Management** | Zustand + Immer (single store) | Apollo Client (GraphQL cache) + React Context providers |
| **API Layer** | Tauri IPC (`invoke` / `listen`) | REST + GraphQL + WebSocket subscriptions |
| **Database** | PostgreSQL (via sqlx, compile-time checked queries) | PostgreSQL + pgvector (via GORM + SQLC auto-generated queries) |
| **Deployment** | Native desktop binary (macOS/Windows/Linux) | Docker Compose (multi-container, cloud-ready) |
| **Source Files** | ~763 (.rs/.ts/.tsx) | ~670 (.go/.ts/.tsx) |
| **DB Migrations** | 30 | 25 |
| **LLM Framework** | rig-core (Rust) with custom provider crates | langchaingo (Go fork) with provider adapters |

---

## 2. Architecture Comparison

### 2.1 Backend Structure

**Golish** uses a modular **4-layer crate architecture**:
- **Layer 1 (Foundation)**: `golish-core` — zero internal deps, core types/traits
- **Layer 2 (Infrastructure)**: 18 crates — PTY, session, tools, settings, context, MCP, web, etc.
- **Layer 3 (Domain)**: `golish-ai` — agent orchestration, planning, HITL, loop detection
- **Layer 4 (Application)**: `golish` — Tauri commands, CLI entry, runtime

**PentAGI** uses a flat **package-based architecture**:
- `cmd/pentagi/` — entry point
- `pkg/config/` — environment-based config
- `pkg/server/` — Gin HTTP router, middleware, auth
- `pkg/graph/` — GraphQL schema + gqlgen resolvers
- `pkg/database/` — SQLC-generated DB queries + GORM models
- `pkg/providers/` — LLM provider adapters (12+ providers)
- `pkg/tools/` — tool integrations (terminal, browser, search engines, memory)
- `pkg/docker/` — Docker SDK for sandboxed execution
- `pkg/observability/` — OpenTelemetry, Langfuse integration
- `pkg/graphiti/` — Neo4j knowledge graph client

### 2.2 Frontend Structure

**Golish** has a rich desktop UI with 65+ component directories:
- Terminal emulation (xterm.js with portal architecture for state persistence)
- Activity-based navigation (Home, Security Testing, AI Chat, etc.)
- Complex split-pane layout system
- Inline AI chat with streaming, tool cards, and approval dialogs

**PentAGI** has a web-based UI focused on flow management:
- ~7 page routes (Dashboard, Flows, Settings, Templates)
- Sidebar-based navigation with flow list
- Real-time updates via GraphQL subscriptions
- Provider management and prompt customization UIs

---

## 3. AI Agent System

### 3.1 Multi-Agent Architecture

**PentAGI** has a sophisticated **role-based multi-agent system** with 12 specialized agent types:

| Agent | Role |
|-------|------|
| **Primary** | Orchestrator — delegates to other agents |
| **Assistant** | Direct user interaction (chat mode) |
| **Generator** | Creates task/subtask plans |
| **Refiner** | Patches and adjusts subtask lists |
| **Pentester** | Performs penetration testing |
| **Coder** | Writes exploit code and scripts |
| **Installer** (Maintenance) | Sets up environments and tools in Docker |
| **Searcher** | Searches the internet via multiple engines |
| **Memorist** | Retrieves from long-term vector memory |
| **Enricher** | Enriches user questions with context |
| **Reporter** | Generates detailed vulnerability reports |
| **Adviser** | Provides expert guidance on complex issues |

Each agent has a dedicated **executor config** that specifies its available tools and inter-agent delegation handlers. The Primary agent delegates tasks to specialists, who can further delegate to each other (e.g., Pentester can call Coder, Searcher, Memorist).

**Golish** has a **single-agent architecture** with sub-agent support:
- One main agentic loop (`golish-ai/src/agentic_loop.rs`)
- Sub-agents defined in `golish-sub-agents` crate
- Tool system via `golish-tools` (file ops, directory ops, AST search, shell)
- Context compaction via dedicated summarizer

### 3.2 Memory System

**PentAGI**: pgvector-based long-term memory with:
- Semantic similarity search
- Per-flow, per-task, per-subtask scoping
- Automatic tool result storage
- Multi-query search with merge/dedup
- Graphiti knowledge graph (Neo4j) for temporal relationship tracking

**Golish**: Session-based persistence with:
- Transcript recording and compaction
- Context budget tracking
- Session synthesis

### 3.3 Context Management

**PentAGI**: Chain summarization (`pkg/csum`) with configurable parameters:
- QA section-based summarization
- Byte limits for sections
- Preserve-last-N configuration

**Golish**: Token-budget-driven compaction:
- `ContextManager::should_compact()` monitors token usage
- Dedicated summarizer LLM call with XML extraction
- Continuation summary injected into system prompt

---

## 4. Tool System

### 4.1 PentAGI Tools

| Tool | Description |
|------|-------------|
| `terminal` | Docker container command execution (blocking, 1200s hard limit) |
| `file` | File read/write in Docker containers |
| `browser` | Web scraping via isolated Scraper container |
| `google` | Google Custom Search |
| `duckduckgo` | DuckDuckGo search |
| `tavily` | Tavily AI search (with summarization) |
| `traversaal` | Traversaal search |
| `perplexity` | Perplexity AI search |
| `searxng` | Meta search engine |
| `sploitus` | Exploit database search |
| `search_in_memory` | Vector store semantic search |
| `search_guide/code/answer` | Domain-specific vector search |
| `store_guide/code/answer` | Store to vector DB with anonymization |
| `graphiti_search` | Knowledge graph temporal search (7 search types) |

All tools follow a unified `Tool` interface with `Handle(ctx, name, args)` and `IsAvailable()` methods. Each tool execution is tracked via Langfuse observability.

### 4.2 Golish Tools

| Tool Area | Tools |
|-----------|-------|
| **Security Pipeline** | naabu (port scan), httpx (HTTP probe), whatweb (fingerprint), katana (crawl) |
| **Vulnerability Scanning** | Nuclei (template-based), ZAP (DAST), feroxbuster (directory brute) |
| **Intelligence** | RSS/feed vuln intel, wiki KB, PoC library |
| **AI/Agent** | File operations, shell execution, AST search, web search, MCP client |
| **Terminal** | Full PTY management with xterm.js |
| **Data Management** | Targets, findings, fingerprints, audit log, scan timeline |

---

## 5. Security Testing Approach

### 5.1 PentAGI's Approach: AI-Driven Autonomous Pentesting
- AI agents **decide** which tools to run and in what order
- All tool execution happens inside **Docker sandboxed containers**
- The system is fully autonomous — the AI plans, executes, and reports
- Strong emphasis on **multi-agent collaboration** (Pentester consults Coder, Searcher)
- Results stored in vector memory for future reference
- Knowledge graph tracks relationships between discovered entities

### 5.2 Golish's Approach: Pipeline-Based Manual + AI Hybrid
- Predefined **reconnaissance pipelines** (naabu → httpx → whatweb → katana)
- Manual security tool configuration (Nuclei, ZAP, feroxbuster)
- **Target Manager** with graph visualization
- **Fingerprint → PoC matching** for Nuclei template selection
- AI agent assists with analysis and context, but tools are user-initiated
- Scan timeline and audit log for tracking

---

## 6. Key Differentiators

### What PentAGI Does Better

1. **Multi-Agent Orchestration**: The role-based agent system with inter-agent delegation is architecturally elegant and allows complex autonomous workflows. Each agent has well-defined responsibilities and tools.

2. **Docker Sandboxing**: All tool execution in isolated Docker containers provides security and reproducibility. The system can run actual pentesting tools (nmap, metasploit, sqlmap) safely.

3. **Observability Stack**: Full OpenTelemetry integration with Grafana, Jaeger, Loki, VictoriaMetrics, plus Langfuse for LLM analytics. Every tool call, agent interaction, and search query is traced.

4. **Knowledge Graph (Graphiti)**: Temporal knowledge tracking via Neo4j enables relationship-aware retrieval and avoids repeating failed approaches.

5. **Vector Memory**: pgvector-based long-term memory with automatic tool result storage and multi-query dedup search provides persistent learning across tasks.

6. **GraphQL + Real-time Subscriptions**: Type-safe API layer with auto-generated types and real-time push to the frontend.

7. **Authentication & Multi-User**: JWT/OAuth2/API tokens support enables team collaboration and programmatic API access.

8. **Browser Tool**: Dedicated containerized browser (Scraper) for web intelligence gathering with screenshot support.

9. **12+ LLM Providers**: Comprehensive provider support including Chinese providers (GLM, Kimi, Qwen) and local models (Ollama, vLLM).

10. **Docker Compose Deployment**: Production-ready containerized deployment with optional monitoring, analytics, and knowledge graph stacks.

### What Golish Does Better

1. **Native Desktop Performance**: Tauri 2 gives native performance and OS integration that a web app cannot match. No network latency for tool execution.

2. **Terminal Emulation**: Full xterm.js-based terminal with PTY, alternate screen detection, fullterm mode, and portal architecture for state persistence. This is the core UX pillar.

3. **Rust Type Safety**: 28-crate modular architecture with compile-time checked SQL queries (sqlx), strong ownership semantics, and zero-cost abstractions.

4. **Reconnaissance Pipeline**: The naabu → httpx → whatweb → katana pipeline with unified fingerprint storage is well-integrated for asset discovery.

5. **Rich Security UI**: Target Manager with graph view, vulnerability dashboard, scan timeline, audit log, sitemap tree, and credential vault — all in a polished desktop app.

6. **Nuclei PoC Matching**: Automatic fingerprint-to-PoC template matching via the `fingerprints` table is a novel approach for targeted scanning.

7. **ZAP Integration**: Deep OWASP ZAP integration with sitemap sync, scan queue management, passive scanning, and policy selection.

---

## 7. Lessons and Borrowable Ideas for Golish

### 7.1 High Priority — Multi-Agent System
PentAGI's multi-agent architecture is the most impactful idea to borrow:
- **Split the single agentic loop** into specialized agents (Researcher, Pentester, Coder, Reporter)
- **Agent delegation**: Allow the Pentester agent to call the Researcher agent for intelligence gathering
- **Task decomposition**: Generator agent creates subtask plans, Refiner adjusts them based on results
- This would enable autonomous pentest workflows where the AI plans and executes end-to-end

### 7.2 High Priority — Long-Term Vector Memory
Add a pgvector-based memory system:
- **Store tool results** in vector embeddings for semantic retrieval
- **Cross-session learning**: Pentesting insights from one target can inform work on similar targets
- **Deduplication-aware search** with multi-query merging
- Implementation: Add a `golish-memory` crate using pgvector, create `SearchInMemory` and `StoreToMemory` tools

### 7.3 High Priority — Observability
Add structured tracing and LLM analytics:
- **OpenTelemetry** integration for distributed tracing of tool execution
- **Langfuse or equivalent** for tracking LLM costs, latency, and quality
- **Tool call tracking**: Every tool invocation logged with input/output, duration, status
- Currently Golish uses `tracing` crate but lacks structured observability

### 7.4 Medium Priority — Docker Sandboxing for Tool Execution
Run security tools in Docker containers instead of directly on the host:
- **Isolation**: Prevents accidental host damage from aggressive scanning
- **Reproducibility**: Consistent tool versions across environments
- **Security**: Tools can't access the host filesystem
- Could be implemented as an optional mode: local (current) vs sandboxed (Docker)

### 7.5 Medium Priority — Knowledge Graph
Add a knowledge graph for relationship tracking:
- **Entity relationships**: Target → Service → Vulnerability → Exploit
- **Temporal tracking**: When was something discovered? What approaches failed?
- **Cross-target intelligence**: Reuse successful techniques
- Could use a simpler graph than Neo4j (e.g., in-PostgreSQL adjacency lists)

### 7.6 Medium Priority — Report Generation Agent
Add an automated report generation system:
- **Dedicated Reporter agent** that synthesizes findings into professional pentest reports
- **Template system** for different report formats (executive summary, technical detail)
- **Screenshot integration** for visual evidence

### 7.7 Medium Priority — Flow/Task Model
PentAGI's Flow → Task → Subtask hierarchy is useful for long-running engagements:
- **Flow** = Pentest engagement (has scope, target, timeline)
- **Task** = High-level objective (e.g., "Enumerate web application")
- **Subtask** = Specific action (e.g., "Run Nuclei templates against port 8080")
- This is more structured than Golish's current ad-hoc pipeline execution

### 7.8 Lower Priority — Tool Result Summarization
PentAGI automatically summarizes large tool results (>16KB) using an LLM:
- Prevents context window overflow
- Preserves critical information while reducing noise
- Could be added to Golish's pipeline output parsing

### 7.9 Lower Priority — Anonymization / Secret Detection
PentAGI uses pattern-based anonymization to sanitize sensitive data before storing to vector memory:
- IP addresses, domains, credentials are replaced with descriptive placeholders
- Prevents accidental credential leakage in long-term storage
- Important for team/shared environments

### 7.10 Lower Priority — External Search Engine Integration
PentAGI integrates 7+ search engines (Google, DuckDuckGo, Tavily, Traversaal, Perplexity, Searxng, Sploitus):
- **Sploitus** is particularly useful — an exploit aggregator for finding PoCs
- Could add Sploitus integration to Golish's vulnerability intelligence system

---

## 8. Architecture Diagrams

### PentAGI Data Flow
```
User → Web UI → GraphQL/REST API → Go Server
                                       ├── LLM Providers (10+)
                                       ├── Agent Goroutines
                                       │    ├── Primary Agent (orchestrator)
                                       │    ├── Pentester Agent → Docker Container
                                       │    ├── Coder Agent → Docker Container
                                       │    ├── Searcher Agent → Search APIs
                                       │    ├── Memorist Agent → pgvector
                                       │    └── Reporter Agent
                                       ├── PostgreSQL + pgvector
                                       ├── Neo4j (Knowledge Graph)
                                       └── Observability (OTEL, Langfuse, Grafana)
```

### Golish Data Flow
```
User → Tauri Desktop App
         ├── React Frontend (xterm.js terminals, security panels)
         │    └── Zustand Store ←→ Tauri IPC
         └── Rust Backend (28 crates)
              ├── golish-ai (single agent loop, context compaction)
              ├── golish-pty (terminal sessions)
              ├── golish-tools (file, shell, AST, web)
              ├── golish/tools (pipeline, scan_runner, vuln_intel, ZAP)
              ├── golish-db (PostgreSQL via sqlx)
              ├── golish-mcp (external MCP servers)
              └── Local tool binaries (naabu, httpx, nuclei, katana, etc.)
```

---

## 9. Summary Table

| Feature | Golish | PentAGI | Winner |
|---------|--------|---------|--------|
| Multi-agent orchestration | Single agent + sub-agents | 12 specialized agents | PentAGI |
| Terminal emulation | Full xterm.js PTY | Docker exec (no interactive term) | Golish |
| Tool sandboxing | Local execution | Docker containers | PentAGI |
| Recon pipeline | naabu→httpx→whatweb→katana | AI-decided tool sequence | Golish (structured) |
| Vulnerability scanning | Nuclei + ZAP + feroxbuster | AI-driven (any Docker tool) | Golish (deeper integration) |
| Long-term memory | Session-based | pgvector + knowledge graph | PentAGI |
| Observability | tracing crate (basic) | OTEL + Langfuse + Grafana | PentAGI |
| Deployment | Native binary | Docker Compose | Depends on use case |
| LLM providers | 5 custom crates | 12+ via langchaingo | PentAGI |
| UI richness | 65+ components, desktop-native | ~20 components, web-based | Golish |
| Report generation | Manual findings view | AI-generated reports | PentAGI |
| Auth / Multi-user | Single-user desktop | JWT/OAuth2/API tokens | PentAGI |
| Type safety | Rust (compile-time) | Go (runtime) | Golish |
| Search integrations | Tavily | 7+ search engines + Sploitus | PentAGI |
| Fingerprint matching | fingerprints → Nuclei PoC | N/A (AI-driven selection) | Golish |
