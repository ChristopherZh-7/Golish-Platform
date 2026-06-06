# golish-events / op_trace

> **一句话职责**：operation 级、自发现的统一 trace——把主 agent transcript + 每个 sub-agent transcript 合并成单条时间序时间线（每行带 `agent_path`），并派生一眼可读的 `OperationManifest`；**lazy 读时计算**，不在运行期写、不阻塞 agent loop。

- **类型**：目录模块（属于 crate [`golish-events`](../golish-events.md)）
- **路径**：`backend/crates/golish-events/src/op_trace/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `harness_trace` 工具 / `golish --replay` 读取的合并时间线、manifest 派生时
- 改 transcript base 解析（`VT_TRANSCRIPT_DIR` > `{workspace}/.golish/transcripts` > `~/.golish/transcripts`）时
- agent「跑了但没日志」——读写两侧 base 解析不同步时

## 职责

把一次 run 的所有 transcript（main + subagents）合并成时间序 `TraceRecord` 列表，每行打 `agent_path`（`main` / `main>pentester` …），并 lazy 派生 `OperationManifest`（status/stages/agents/last_decision）。设计见 `docs/design/2026-06-05-unified-ai-harness-observability.md` §4.C。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TraceRecord` / `OperationManifest` | 合并时间线行 / 一眼概览 |
| `resolve_transcript_base` / `resolve_transcript_base_for_session` / `default_transcript_base` | transcript base 解析（**须与写侧 lockstep**） |
| `session_dir` | `{base}/{session_id}/` |
| `collect_records` / `build_manifest` | 合并记录 / 派生 manifest |
| `render_timeline` / `decision_records_json` | 人/AI 可读时间线 / 决策记录 JSON（`harness_trace` 工具用） |
| `write_trace_artifacts` | 写 `timeline.jsonl` + `manifest.json`（`just replay` 副作用，原子写 manifest） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 全部解析/合并/manifest/渲染逻辑 |
| `tests.rs` | 单测 |

## 依赖

- `golish_core::events`（`AiEvent`/`HarnessTraceKind`/`build_agent_path`）、`golish_core::jsonl`、`serde_json`、`chrono`

## 注意事项 / 坑

- **读写 base 必须 lockstep**：写侧（`golish-agent-app` session init）与读侧（本模块）用同一解析顺序——home-only 默认会漏掉从真实 workspace 启动的 run（"没日志"症状根因）。
- **lazy 读时计算**：不在 run 期写，故不阻塞 agent loop；`HarnessTrace` 决策经正常事件路径落 `transcript.json`。
- 容忍 legacy 整文件 JSON 数组 + 跳过不可解析行（截断标记/半写尾行），不整体失败。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-events op_trace
```
