# golish-agent-kit / loop_detection

> **一句话职责**：循环检测与执行监督——硬性 loop guard 防无限循环/失控，`ExecutionMonitor` 负责 PentAGI-style RuntimeSupervisor 触发（shadow/soft/hard）。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/loop_detection/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改循环保护阈值（max turn / max tool loops / max repeated tool calls）或检测逻辑时
- 改 RuntimeSupervisor 触发阈值、shadow/soft/hard 注入模式时
- agent 报 LoopWarning/LoopBlocked/MaxIterationsReached 相关时

## 职责

硬性 `LoopDetector` 跟踪并拦截失控行为：总 turn 数、每 turn 工具调用数、相同参数重复调用次数。超限发 `AiEvent::LoopWarning`/`LoopBlocked`/`MaxIterationsReached`（见 `golish-core::events`）。

`ExecutionMonitor` 是较软的执行监督触发器：跟踪同工具重复/总工具调用阈值，触发 RuntimeSupervisor。bridge 默认启用 hard supervisor（普通 `just dev` 直接生效）；优先读 `GOLISH_RUNTIME_SUPERVISOR`，兼容旧 `GOLISH_EXECUTION_MENTOR`；`shadow` 只记录结构化决策，`soft`/`on` 注入策略指令，`hard` 还会阻止同一批剩余工具继续跑偏，`off`/`false`/`0` 关闭。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ExecutionMonitor` / `ExecutionMonitorMode` | RuntimeSupervisor 触发监视器；`Shadow` 只记录、`SoftInject` 注入策略指令、`HardInject` 注入更强纠偏 |
| `LoopProtectionConfig`（`max_tool_loops` / `max_repeated_tool_calls` / max turns…） | 保护配置 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `LoopProtectionConfig` + re-export |
| `monitor.rs` | `ExecutionMonitor` 检测逻辑 |

## 依赖

- crate 内；`serde`（config）

## 注意事项 / 坑

- 阈值过松会放任失控（烧 token/钱）、过紧会误杀合法长任务；改阈值要兼顾 Task 模式的多 turn。
- `ExecutionMonitor::new()` 保留 soft-inject 语义；生产默认由 bridge 创建 hard-inject monitor，`GOLISH_RUNTIME_SUPERVISOR` / 旧 `GOLISH_EXECUTION_MENTOR` 作为 shadow/soft/off 调试覆盖。
- 对应前端的 LoopWarning/LoopBlocked 事件（`golish-core::events` + `cli_json::loop_guard`）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit loop_detection
```
