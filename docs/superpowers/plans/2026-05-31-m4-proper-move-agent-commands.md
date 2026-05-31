# M4-proper：把 agent 命令体 + 桥接层搬进 `golish-agent-app`

> 父：`docs/superpowers/plans/2026-05-31-m4a-appstate-decouple.md`（M4-A 已完成：`golish-agent-app`(L5.6) 落地 `AiState`+`AgentState`，19 个 `ai/commands/*` 已 take `AgentState`）。
> **目标**：把 `golish/src/ai/` 整棵子树（命令体 + AppState-free 桥接层 + facade）+ `tools/conversation_store` 搬进 `golish-agent-app`，golish 侧改 `commands_facade` 转发 + 外部消费者改指新 crate——完成 crate-per-service 的 agent 命令面迁移（层次 A）。
> **不在范围**：层次 B 端口（切 agent→recon/vuln/pentest 出向硬依赖、切 platform→agent 入向）留 S1-2b / 后续端口里程碑。

---

## 1. 实证范围（2026-05-31 · Glob/Read/Grep）

### 1.1 待搬文件（`ai/` 39 文件 + conversation_store）
- **命令面** `ai/commands/`（19）：`agents/analytics/config/context/debug/dispatch/graph/hitl/loop_detection/mode/plan/policy/session/summarizer/workflow/bridge_config/mod` + `core/{chat,lifecycle,mod,session,tools}`。已 take `AgentState`（M4-A A2）。
- **AppState-free 桥接层**：`ai/db_bridge/{mod,wiki,recon,orchestration,tasks,convert}`、`ai/tracking_bridge/{mod,ready_gate,memory,rows,chain,records}`、`ai/{session_bridge,graph_bridge,embedder_bridge,sidecar_bridge}.rs`。
- **facade** `ai/mod.rs`（薄 re-export：L4 agent-kit/runtime/bridge/prompts/events + commands）。
- **`tools/conversation_store/`**（agent 自有表 `conversation_store`，父计划 §M4 范围）。

### 1.2 注册路径（M1-M3 已验证范式）
- `commands_facade/ai.rs` = `pub use crate::ai::commands::*;` → 改为 `pub use golish_agent_app::commands::*;`。
- `commands_registry.rs` 扁平 ident 列表 + `use crate::commands_facade::*;` **不动**（facade 转发，`__cmd__` 宏经 glob 解析，M1/M2/M3 已证）。

### 1.3 非命令消费者（5 处，须改指新 crate）
| 文件 | 现引用 | 迁后 |
|---|---|---|
| `state/mod.rs` | `AppState` 含 `ai_state: AiState`（已是 agent-app 类型）；`extract_agent_state()`；构造 `GolishDbRepoProvider`(db_bridge) | `crate::ai::*` → `golish_agent_app::*` |
| `cli/bootstrap/agent_init.rs` | `crate::ai::agent_bridge::AgentBridge`、`crate::ai::sidecar_bridge::SidecarCaptureBackend`、`crate::ai::commands::McpManagerToolExecutor` | 改 `golish_agent_app::...`（或经 golish `ai` shim 再导出，见 §3 决策） |
| `cli/bootstrap/mod.rs` | `crate::ai::...` | 同上 |
| `app/mcp_bootstrap.rs` | `configure_bridge` / `setup_bridge_mcp_tools`（M4-A 已改走 `extract_agent_state()`） | `golish_agent_app::commands::...` |
| `mcp/commands.rs` | `refresh_all_bridge_mcp_tools`（M4-A 已 take `&AgentState`） | `golish_agent_app::commands::...` |

### 1.4 依赖（agent-app Cargo.toml 现有 vs 需补）
- **现有**：app-core / agent-bridge / core / db / settings / sidecar / indexer / pty / mcp / pentest / sqlx / tokio / parking_lot。
- **需补（由 ai/ 子树 import 决定，按 compile 收敛）**：`golish-agent-kit`、`golish-agent-runtime`、`golish-prompts`、`golish-events`、`golish-sub-agents`、`golish-graphiti`（graph_bridge）、`golish-models`、`golish-llm-providers`、`golish-context`、`golish-session`、`golish-projects`、`golish-vuln-intel`(?)、`vtcode-indexer`(?) + `serde`/`serde_json`/`anyhow`/`uuid`/`chrono`/`async-trait`/`rig-core`/`graph-flow`/`futures`/`tracing`/`regex`/`thiserror`。
- **守卫**：`golish-agent-app` 已在 `check_dag` L5.6；新增的内部依赖须全部 ≤ L5.6（agent-kit L4 / runtime L4 / prompts / events / graphiti L2 / models / llm-providers / context / session / projects 均 ≤L4），不破 DAG。

### 1.5 守卫迁移（`check_repo_ownership.py`）
- `SOURCE_ROOTS` 加 `("golish-agent-app", "agent")`。
- `DOMAIN_RULES` 删 `("ai/", "agent")` + `("tools/conversation_store", "agent")`（文件移出 golish/src）。
- `ALLOWLIST` 8 条 `ai/db_bridge/*` → `golish-agent-app/db_bridge/*`：orchestration→execution_plans；recon→{api_endpoints,fingerprints,js_analysis,passive_scans,target_assets,vuln_intel}；wiki→wiki_kb。
- `RAW_SQL_ALLOWLIST`：`ai/session_bridge.rs` + `ai/tracking_bridge/{chain,memory,records}.rs` + `tools/conversation_store/{batch,mod}.rs` → `golish-agent-app/...` 前缀。

---

## 2. 为什么"大一统协调搬迁"（不拆 P1/P2 独立验证）
`ai/commands` → `bridge_config` → `db_bridge/tracking_bridge`，且 `ai/mod.rs` 同时 re-export 两者；命令与桥接互依。若只搬一半，需在 golish↔agent-app 间架临时 shim（命令在 golish 引 `golish_agent_app::db_bridge`，或反之），徒增 churn。**与 M3 的 pipeline↔pentest_bridge 同移同理**：一次性 `git mv` 整棵 `ai/` + conversation_store，再统一 remap + rewire + 驱动 cargo check 至绿。中途不可编译，最终态可编译/可回滚（纯结构搬移）。

---

## 3. 子步（执行顺序；中途不单独编译，末尾统一验证）
- **P0 依赖**：agent-app Cargo.toml 补 §1.4 deps（+ backend workspace.deps 若缺）。
- **P1 桥接层 + conversation_store**：`git mv ai/{db_bridge,tracking_bridge,session_bridge,graph_bridge,embedder_bridge,sidecar_bridge}` + `tools/conversation_store` → agent-app/src/；导入重映射（`crate::error`→`golish_app_core`、`crate::state::DbState`→`golish_app_core::DbState`、`crate::ai::*`→`crate::*`、跨服务 `golish_db::repo::<x>` 保留直读）。
- **P2 命令体 + facade**：`git mv ai/commands` + `ai/mod.rs` → agent-app/src/；`ai/mod.rs` 内 `pub mod commands; pub mod db_bridge; ...` 保持；命令导入重映射（`State<AgentState>` 已就位；`crate::tools::*`→对应 crate；`crate::ai::*`→`crate::*`）。agent-app `lib.rs` 挂 `pub mod ai;`（或直接 `pub mod commands;` 等，决定对外路径）。
- **P3 rewire**：`commands_facade/ai.rs`→`pub use golish_agent_app::commands::*;`；golish 删 `mod ai;`（或留薄 shim `pub use golish_agent_app::ai::*;` 以兼容 `crate::ai::*` 现存引用，**推荐留 shim** 降 churn）；`tools/mod.rs` 删 `pub mod conversation_store;`（或 shim）；改 5 处非命令消费者（§1.3）。
- **P4 守卫**：按 §1.5 改 `check_repo_ownership.py`；`check_dag.py` 校验（agent-app 已 L5.6，确认新 deps 不破环）。
- **P5 验证**：见 §4。

> **决策点（§3 P3）**：golish 侧 `crate::ai` 与 `tools::conversation_store` 是否保留再导出 shim。**推荐保留** `golish/src/ai.rs`（薄 `pub use golish_agent_app::ai::*;`）——`state/mod.rs`/`cli/bootstrap`/`app`/`mcp` 大量 `crate::ai::*` 引用可零改动，churn 最小（M3 对 pentest_ai/pentest_bridge 同法）。后续若要彻底删 shim 再单独清理。

---

## 4. 验证矩阵（末尾统一，全 exit 0 才算 P5 完成）
1. `cargo check -p golish-agent-app`（cwd backend）
2. `cargo check -p golish`（无 warning）
3. `cargo nextest run -p golish-agent-app`（迁入的 db_bridge/tracking_bridge/conversation_store 单测全绿）
4. `cargo clippy -p golish-agent-app --all-targets -- -D warnings`（按需补 crate 级 `#![allow(...)]` 镜像 golish lib.rs）
5. `cargo clippy -p golish --lib -- -D warnings`（覆盖全 workspace）
6. `python3 scripts/check_dag.py`（✓ 50 crates，agent-app L5.6 不变）
7. `python3 scripts/check_repo_ownership.py`（OK clean，allowlist 迁移生效）
8. `ReadLints`（接线文件 + agent-app 迁入目录）无错误
9. grep 残留：agent-app 内无 `crate::error|crate::state::AppState|crate::tools|crate::ai::`（已 remap）

> **不在本步**（§2.7 须用户授权）：`just precommit` 全量、`git commit/push`、运行时 invoke 实测。沿用 M1-M4-A 的"工作树叠加、待用户授权统一收口"模式。

---

## 5. 风险 & 回滚
- **风险**：ai/ 子树最厚、依赖最广（agent-kit/runtime/prompts/events/graphiti/models/llm-providers/context/session/...），P0 dep 收敛可能反复；非命令消费者（CLI bootstrap 构造 AgentBridge）易漏。→ 缓解：P3 留 `crate::ai` shim 把消费者改动降到最小；P5 grep 兜底。
- **回滚**：纯 `git mv` + import remap + facade 转发，未 commit 前可 `git checkout -- <paths>` / `git mv` 回原位；M4-A 的 50-crate 绿态是回退基线。

## 6. 自检（writing-plans）
- 规格覆盖：父计划 §M4 范围（ai/ + conversation_store + facade ai）全覆盖；层次 B 端口显式 deferred。✓
- 顺序依赖：P0(dep)→P1(桥)→P2(命令)→P3(rewire)→P4(守卫)→P5(验证)，与 M1-M3 范式一致。✓
- 路径/类型一致：`golish-agent-app` / `AgentState` / `commands_facade/ai.rs` 跨节一致；守卫前缀沿用 M3 crate-prefixed 约定。✓
