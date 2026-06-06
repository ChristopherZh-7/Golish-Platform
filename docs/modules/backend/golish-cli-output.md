# golish-cli-output

> **一句话职责**：CLI 输出处理——事件接收循环，按输出模式（terminal / JSON / quiet）渲染 agent 事件。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-cli-output/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 CLI 渲染、输出模式、JSONL 程序化输出、截断策略时
- `golish` 命令行/headless 输出不对、JSON 被截断时

## 职责

从运行时 channel 接收 agent 事件并按模式渲染。这是 CLI 能工作的关键模块。三种模式：terminal（人类可读、box 绘制）、JSON（标准 JSONL、**不截断**）、quiet（只最终响应）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `run_event_loop` | 事件接收主循环 |
| `convert_to_cli_json` / `CliJsonEvent` | 转 JSONL 事件 |
| `truncate`（via `formatting`） | 截断工具 |

## 依赖

- **内部**：`golish-core`

## 被谁依赖 / 改动影响面

`golish`（主程序 CLI/headless 路径）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `cli_json/` | CLI 事件 → JSON 转换 | [→](golish-cli-output/cli_json.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `event_loop.rs` | 接收事件并渲染 |
| `terminal.rs` | 终端模式格式化 |
| `formatting.rs` | 截断/格式化 helper |

## 注意事项 / 坑

- **截断契约**：terminal 模式截断（工具输出 500、reasoning 2000 字符），**JSON 模式不截断**（程序化解析依赖完整输出），quiet 只出最终响应。改截断别动 JSON 模式的「不截断」。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-cli-output
```
