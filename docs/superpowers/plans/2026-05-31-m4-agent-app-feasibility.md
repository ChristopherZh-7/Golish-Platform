# M4 可行性评估：抽 `golish-agent-app`（agent 服务 crate）

> 父计划：`docs/superpowers/plans/2026-05-30-crate-per-service-split.md` §M4。
> **结论先行：M4 不能像 M2/M3 那样直接干净抽（层次 A）——存在真实架构 blocker：agent 命令面与 god-crate `AppState` 互锁。** 本文记录 2026-05-31 的实证调查与可选路径，避免下一会话重复调查。

---

## 1. 实证调查（2026-05-31 · grep/read 证据）

### 1.1 `ai/` 结构（39 文件）
- **命令面** `ai/commands/*`（19 文件）：analytics/chat/config/context/core/{chat,lifecycle,mod,session,tools}/debug/dispatch/graph/hitl/loop_detection/mode/plan/policy/session/summarizer/workflow/agents/bridge_config。
- **桥接/持久化层**：`ai/{db_bridge,tracking_bridge,session_bridge,embedder_bridge,graph_bridge,sidecar_bridge}` + `tools/conversation_store`。
- `ai/mod.rs`：薄 facade，**glob 重导出** golish-agent-kit/runtime/bridge/prompts/events（L4 crate）。

### 1.2 AppState 互锁（决定性 blocker）
- `ai/commands/*` **几乎全部** take `tauri::State<'_, AppState>`（实证计数：session 11 / context 9 / hitl 8 / policy 8 / loop_detection 7 / analytics 7 / chat 6 / tools 6 / …，19 文件全中）。
- `AppState`（`golish/src/state/mod.rs:39`）聚合 `ai_state: AiState` + indexer_state + settings_manager + sidecar + mcp_manager + command_index + pentest_config_manager + pty_* 等 **golish 内部子系统**（M0 决策：AppState 故意留 golish）。
- `AiState` 定义在 **`ai/commands/mod.rs:61`**，经 `crate::ai::AiState` 被 AppState 引用。
- **三者互锁**：`ai/commands` → `AppState` → `AiState`(在 `ai/commands`)。把 `ai/` 搬到 `golish-agent-app` ⇒ AppState（golish）需 agent-app 的 AiState + agent-app 命令需 golish 的 AppState ⇒ **golish ↔ agent-app 循环依赖**。这是 M2/M3 没有的（那三个域命令 take 窄 `DbState`）。

### 1.3 可搬 vs 不可搬
| 部分 | AppState 耦合 | 层次 A 可搬? |
|---|---|---|
| `ai/db_bridge`（GolishDbRepoProvider，跨服务读 recon/vuln/pentest）| **无**（实证 0）| ✅ 可搬（take pool/DbState）|
| `ai/tracking_bridge`（chain/memory/records/rows/ready_gate）| **无** | ✅ 可搬 |
| `ai/{session,embedder,graph,sidecar}_bridge` | **无** | ✅ 可搬 |
| `tools/conversation_store` | **无** | ✅ 可搬 |
| **`ai/commands/*`（命令面 + AiState）** | **重度**（19 文件）| ❌ **搬不动**（需 AppState 解耦）|

> crate-per-service 的核心价值是**搬命令面**。agent 的命令面恰恰是 AppState 的核心，所以"只搬桥接层"价值有限。

### 1.4 跨服务出向（层次 B 端口前置，与本 blocker 正交）
父计划 §4：agent 出向 8（读 recon 6 + vuln 1 + pentest 1，集中在 `ai/db_bridge/{recon,wiki,orchestration}.rs`，已在 ALLOWLIST）。层次 A 下这些走 `golish_db::repo::<x>` 直读即可（迁 ALLOWLIST 前缀）；层次 B 才需 ReconPort/VulnReadPort/PentestPlanReadPort。**注意**：这是出向耦合，与 §1.2 的 AppState 互锁（结构性 blocker）是两回事——即便端口齐了，AppState 互锁仍挡路。

---

## 2. 可选路径（待用户拍板）

| 选项 | 做法 | 代价 | 评价 |
|---|---|---|---|
| **A · AppState 解耦先行（M4 真前置）** | 定义窄 `AgentState`（ai_state + db_pool + 相关 manager），19 个 ai/commands 改 `State<AppState>`→`State<AgentState>`；`AiState` 下沉到 agent-app 或共享 crate，让 golish `AppState` 不再直接含它（或改持 `Arc<dyn ...>`）；启动接线注入 AgentState。然后 ai/commands 才能搬入 agent-app。 | **中大型重构**（19 命令签名 + AppState 字段/接线 + AiState 归属 + 可能牵动 indexer/settings 引用）| M4 的正解，但工作量与风险显著高于 M2/M3 |
| **B · 部分 M4（只抽桥接层）** | 把 AppState-free 的 `db_bridge/tracking_bridge/*_bridge` + `conversation_store` 抽到 agent-app；命令面留 golish。 | 中 | 价值有限（不含命令面），且 db_bridge 是 agent-kit 的 DbRepoProvider 实现，搬动需理顺 L4 边界 |
| **C · 先抽 M5（platform），M4 留后** | platform（vault/audit/notes/recordings）用 `DbState`，是干净的层次 A 移动（同 M2/M3）。先做完 M5（4/5 域 crate 化），再把 A 的 AppState 解耦当独立里程碑。 | 低（M5 与 M2/M3 同范式）| **推荐**：先拿确定性收益，M4 的硬骨头单独立项 |
| **D · sink AppState 到 app-core** | 把 AppState 整体下沉共享 crate。 | 极大 + 违反 M0 | **不推荐**（把 indexer/settings/sidecar/ai 全拽下来，等于不拆） |

---

## 3. 推荐

**C 然后 A**：先抽 M5（platform，干净），把 crate-per-service 推到 4/5；再把"AppState 解耦 → 抽 agent 命令面"作为专门里程碑（A）推进。M4 的 AppState 解耦本质是"窄状态化"（narrow-state extraction），与 S1-2 端口化同源，值得单独设计文档 + 计划。

> 不在本会话强行 sink AppState（选项 D）或硬拆造环——那是 §2.7 级高风险架构变更，须用户明确拍板。
