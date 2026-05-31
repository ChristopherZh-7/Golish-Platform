# M4-A：AppState 解耦（narrow `AgentState`）— M4 的真前置

> 父：`docs/superpowers/plans/2026-05-31-m4-agent-app-feasibility.md`（M4 被 AppState 互锁挡住）。
> **目标**：让 `ai/commands/*` 不再 take 单体 `AppState`，改 take 窄 `AgentState`，并把 `AiState` 移到 golish 之外可达处——切断 `golish ↔ agent-app` 循环，为 M4（搬 ai/ 命令面）扫清结构性 blocker。本步**不搬命令体**（那是 M4-proper），只做状态解耦。

---

## 1. 实证结论（2026-05-31 · grep/read）

### 1.1 ai/commands 用到的 AppState 字段（→ AgentState 必含）
`ai_state`（重度，bridges/runtime/legacy_bridge）、`db_pool`、`db_ready`、`settings_manager`、`sidecar_state`、`sidecar_config`、`indexer_state`、`mcp_manager`、`pentest_config_manager`、`pty_manager`、`pty_output_tap`、`active_terminal_session`、`pentest_busy_sessions`。
**不含**（留 golish AppState 的平台位）：`command_index`、`telemetry_stats`、`langfuse_active`。

### 1.2 类型归属（决定可移性）
| 类型 | 位置 | 可被 ≤L5 引用? |
|---|---|---|
| `AiState` | **golish 内部** `ai/commands/mod.rs:61`（持 `AgentBridge`@L4.3 + `GolishRuntime`@L1）| ✅ 可移到 app-core/agent-app |
| `IndexerState` | golish-indexer (L2) | ✅ |
| `SidecarState` | golish-sidecar (L3) | ✅ |
| `PtyManager` | golish-pty (L2) | ✅ |
| `PtyOutputTap` | golish-app-core (L5，M3 已下沉) | ✅ |
| settings/mcp/pentest cfg | golish-settings(L1)/golish-mcp(L1)/golish-pentest(L2) | ✅ |
| db_pool/db_ready | golish-db (L2) | ✅ |

**唯一 golish-内部依赖 = `AiState`，且它只引 L4+L1，可移。** 故 AgentState 可整体定义在 ≤L5 处。

## 2. 设计

### 2.1 待决策（§4 待用户拍板）：AiState + AgentState 的家
- **方案 X（推荐）：新建 `golish-agent-app`(L5.6)，AiState + AgentState 放这里。** A 即 M4 第一阶段；M4-proper 再把命令体/桥接搬进同 crate。golish `AppState` 持 `AgentState`（依赖 agent-app，下行边，合法）。
- 方案 Y：放 `golish-app-core`(L5)。不新建 crate，但 app-core 要加 golish-agent-bridge(L4) 依赖、且承载 agent 运行态（语义上偏重）。

> 两者都切断循环（命令 take 的 AgentState 不在 golish）。X 更贴 M4 北极星，Y 改动面更小。

### 2.2 结构（以方案 X 为例）
1. 新建 `golish-agent-app` crate（Cargo.toml：golish-app-core/db/core/settings/sidecar/indexer/pty/mcp/pentest/golish-agent-bridge + tauri/tokio/...）；check_dag L5.6（与 pentest-app 同层，sibling）。
2. `AiState`（+ `ai_not_initialized_error` 等 helper）从 `ai/commands/mod.rs` `git mv`/搬到 `golish-agent-app/src/state.rs`；`AgentBridge` 引用改 `golish_agent_bridge::AgentBridge`、`GolishError`→`golish_app_core`。
3. 在同处定义 `pub struct AgentState`（§1.1 的 13 字段，类型用各 crate 路径）+ `AgentState::extract_db_state()` 等便捷方法（镜像 AppState）。
4. golish `AppState`：`ai_state: AiState` 改 `agent: AgentState`（或保留字段但类型来自 agent-app）；`AppState::new` 构造 AgentState；加 `extract_agent_state(&self)->AgentState`（克隆共享 Arc）；保留 command_index/telemetry/langfuse。
5. golish `lib.rs` 加 `golish-agent-app` 依赖；`crate::ai::AiState` 重导出改指 `golish_agent_app::AiState`（兼容现有路径）。

### 2.3 命令迁移（19 文件）
`ai/commands/*` 的 `tauri::State<'_, AppState>` → `tauri::State<'_, AgentState>`；字段访问 `state.ai_state`→`state.ai_state`（AgentState 同名字段）、`state.db_pool`→`state.db_pool`、… 多数字段名不变，仅 State 类型换；个别用 `state.extract_db_state()` 等改调 AgentState 方法。

### 2.4 启动接线
`app/tauri_app.rs`：`.manage(app_state.extract_agent_state())`（与现有 `.manage(app_state)` 并存）；AgentState 与 AppState 共享同一批 `Arc`（ai_state/db_pool/...），行为零变。

## 3. 子步（每步可验证；建议每步 cargo check）
- **A1**：scaffold golish-agent-app + 搬 AiState + 定义 AgentState（+ extract）；golish AppState 改用 AgentState；`cargo check -p golish-agent-app && cargo check -p golish`。
- **A2**：批量迁 19 个 ai/commands 签名 State<AppState>→State<AgentState>；`cargo check -p golish`。
- **A3**：startup `.manage(extract_agent_state())`；守卫（check_dag L5.6）；`nextest -p golish` 相关 + `clippy -p golish --lib -D` + 双守卫。
- （后续 M4-proper：把 ai/commands 命令体 + db_bridge/tracking_bridge/* 搬进 agent-app。）

## 4. 待用户拍板
1. **AiState/AgentState 的家**：方案 X（新建 golish-agent-app，推荐）/ 方案 Y（app-core）？
2. 是否本会话执行 A1（scaffold + 状态搬移），还是先只交设计？
3. 高风险点（§2.7）：改 AppState 字段 + 启动接线（影响全 app 状态装配）——需点头。

## 5. 自检
- 解耦后命令不再引 golish 单体 AppState → M4 移命令体时无循环。✓
- AgentState 字段类型全 ≤L5 或可移 AiState；零跨服务语义变更（共享同 Arc）。✓
- 不搬命令体 = 行为零变、可回滚（纯状态装配重构）。✓
