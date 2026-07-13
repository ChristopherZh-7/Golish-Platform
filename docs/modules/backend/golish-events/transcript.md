# golish-events / transcript

> **一句话职责**：transcript 写入器——把 AI 事件以 JSONL 落盘到 `{base}/{session_id}/transcript.json`（支持 replay/调试/分析），并提供 `should_transcript` 过滤（剔除流式与 sub-agent 内部事件）+ summarizer 输入/输出读写。

- **类型**：目录模块（属于 crate [`golish-events`](../golish-events.md)）
- **路径**：`backend/crates/golish-events/src/transcript/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 transcript 落盘（JSONL append）、路径构造、读取（含 legacy 数组兼容）时
- 改 `should_transcript` 过滤规则（哪些事件不进 transcript）时
- 改 summarizer 输入构建/保存（压缩摘要链路）时

## 职责

`TranscriptWriter` 把 `AiEvent` 以 JSONL append 到 `{base}/{session_id}/transcript.json`（底层 `EventTranscriptWriter`）。`read_transcript` 读回（兼容 legacy 整文件数组 + 跳过不可解析行）。`should_transcript` 过滤掉流式/sub-agent 内部事件（这些在别处捕获）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TranscriptWriter`（`new` / `append` / `path`） | JSONL 事件写入器 |
| `TranscriptEvent` | 带 timestamp 的事件（读出类型） |
| `transcript_path(base, session)` | `{base}/{session}/transcript.json` |
| `read_transcript(base, session)` | 读回事件（时间序，容错） |
| `should_transcript(&AiEvent) -> bool` | 是否该写入（剔流式/sub-agent 内部） |
| `build_summarizer_input` / `format_for_summarizer` / `save_summarizer_input` / `save_summary` | summarizer 输入/输出读写 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | writer/reader + `should_transcript` + 路径 |
| `summarizer.rs` | summarizer 输入构建 + 保存 |

## 依赖

- `golish_core::events::AiEvent`、`golish_core::jsonl`（`EventTranscriptWriter`/`TimestampedEntry`）、`tokio::fs`、`serde_json`、`chrono`

## 注意事项 / 坑

- `should_transcript` 故意剔除 `TextDelta`/`Reasoning`/`ToolOutputChunk`/`SubAgent*` 等高频/内部事件——加新流式事件时考虑是否该加进过滤。
- `summarizer.rs` 对 `HarnessTraceKind` 做可读摘要；新增 trace kind 要同步 match 分支。`StageRefinerDecision` 和 `RuntimeSupervisorDecision` 都会进入 summarizer 输入；RuntimeSupervisor 可在 soft/hard 模式改变 agent 下一步策略，但不决定 gate。Candidate Attempt terminal / Wave consolidation 摘要只保留 id、状态/decision、聚合 counts 与 replay，测试显式拒绝 `payload|lease|plan|exploit` 泄漏。
- 读取容忍 legacy 整文件 JSON 数组 + JSONL 两种格式，且跳过坏行（不整体失败）。
- 与 `op_trace` 共用同一 transcript 目录与格式；改路径/格式要两边同步。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-events transcript
```
