# golish-pty / parser

> **一句话职责**：`TerminalParser`——基于 `vte` 解析 PTY 输出抽取 OSC 事件，检测 alt-screen，并把输出按区域过滤（只留 Output 区，剔 Prompt/Input）；alt-screen（TUI）模式下禁用过滤透传全部转义序列。

- **类型**：目录模块（属于 crate [`golish-pty`](../golish-pty.md)）
- **路径**：`backend/crates/golish-pty/src/parser/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 OSC 序列抽取（OSC 133 shell 集成等）、终端区域分割（Prompt/Input/Output）时
- 改 alt-screen 检测或 TUI 透传逻辑时
- agent 抓的命令输出夹带 prompt/转义噪声时

## 职责

`TerminalParser` 用 `vte::Parser` + `OscPerformer` 解析输出：`parse` 抽 OSC 事件；`parse_filtered` 额外把可见字节按区域过滤（只留 Output），但若处于/进入 alt-screen 则**透传原始字节**（TUI 需要全转义序列）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TerminalParser`（`parse` / `parse_filtered` / `in_alternate_screen`） | 解析器主体 |
| `OscEvent` / `ParseResult` / `TerminalRegion` | OSC 事件 / 解析结果（events+output+prompt_visible）/ 区域 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `TerminalParser` 本体 + alt-screen 透传决策 |
| `performer.rs` | `OscPerformer`（vte Perform 实现，区域跟踪） |
| `types.rs` | `OscEvent`/`ParseResult`/`TerminalRegion` |

## 依赖

- `vte`（VT 解析）

## 注意事项 / 坑

- 模块标 `#![allow(dead_code)]`（仅经 Tauri feature 集成）。
- **alt-screen 必须透传**：TUI（vim/htop）下别过滤，否则丢转义序列；`parse_filtered` 已据 `alternate_screen_active` 决策，改逻辑要保此分支。
- `prompt_visible` 始终独立 drain（与 alt-screen 无关），前端 fullterm 模式忽略 stdin_wait。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-pty parser
```
