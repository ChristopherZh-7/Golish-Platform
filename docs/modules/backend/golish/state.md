# golish / state

> **一句话职责**：Tauri 命令的 per-domain managed state——窄子状态（`DbState`/`McpManaged`/`PtyState`/`SidecarManaged`/`TelemetryState`）+ 巨石 `AppState`（聚合 golish 内部子系统，`extract_agent_state()` 派生窄 `AgentState`）。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/state/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改全局 `AppState` 聚合的子系统、managed 子状态、`extract_agent_state` 派生时
- 新命令该取窄子状态（`DbState` 等）而非 `AppState` 时

## 职责

定义 Tauri managed state。新命令应取窄子状态（`DbState`/`PtyState`/…）而非巨石 `AppState`；过渡期两者都 `.manage()`。`AppState` 聚合 golish 内部子系统（AI/indexer/settings/sidecar/…），并 `extract_agent_state()` 派生 `golish-agent-app::AgentState`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `AppState`（+ `extract_agent_state()`） | 巨石全局状态 + 派生窄 AgentState |
| `DbState` / `McpManaged` / `PtyState` / `SidecarManaged` / `TelemetryState` | per-domain 窄子状态 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `AppState` + 窄子状态 re-export |
| `db.rs` / `mcp.rs` / `pty.rs` / `sidecar.rs` / `telemetry.rs` | 各子状态 |

## 依赖

- crate 内全部子系统；`golish-app-core`（`DbState` 来源）、`golish-agent-app`（`AgentState`）

## 注意事项 / 坑

- **新命令取窄子状态**（`DbState` 等），别取 `AppState`（巨石，难解耦）——这是 crate-per-service 的核心约束。
- `AgentState` 现在 golish-agent-app；golish 经 `extract_agent_state()` 构造，不再 re-export 该类型。

## 测试入口

```bash
cd backend && cargo nextest run -p golish state
```
