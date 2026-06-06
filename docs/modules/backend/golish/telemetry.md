# golish / telemetry

> **一句话职责**：OpenTelemetry / Langfuse tracing 集成——`init_tracing` + `TelemetryGuard` + Langfuse 配置 + counting processor + filter + 统计快照。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/telemetry/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 tracing 初始化、Langfuse 上报、span 过滤、telemetry 统计时

## 职责

初始化 tracing-subscriber + OpenTelemetry/Langfuse 导出。`init`（`init_tracing`）、`guard`（`TelemetryGuard` 持有导出器生命周期）、`langfuse`（`LangfuseConfig`）、`counting_processor`/`filter`（span 计数/过滤）、`stats`（统计快照）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `init_tracing` | tracing + OTel 初始化 |
| `TelemetryGuard` | 导出器生命周期 guard |
| `LangfuseConfig` | Langfuse 配置 |
| `TelemetryStats` / `TelemetryStatsSnapshot` | 统计 / 快照 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `init.rs` / `guard.rs` | 初始化 / guard |
| `langfuse.rs` / `counting_processor.rs` / `filter.rs` / `stats.rs` | Langfuse / 计数 / 过滤 / 统计 |

## 依赖

- `opentelemetry*`、`tracing-opentelemetry`、`opentelemetry-langfuse`、`tracing-subscriber/appender`

## 注意事项 / 坑

- `TelemetryGuard` 必须存活到进程结束（drop 即 flush 导出）；启动时 `init_telemetry_and_app_state` 返回它，别提前 drop。
- Langfuse 上报涉及外部网络；配置缺失应优雅降级（仅本地 log）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish telemetry
```
