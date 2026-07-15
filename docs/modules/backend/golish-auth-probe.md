# golish-auth-probe

> Superseded by [Vuln Observation → Candidate → Verification 闭环](../../design/2026-07-14-vuln-observation-candidate-closure.md). The crate and runtime tool were removed on 2026-07-14; this card is retained as historical context.

> **历史职责**：API 授权探测（旧 API 安全流水线 Stage 2）——消费 js-analyzer 的 `Endpoint`，对每个端点跑 3 轮 HTTP 检测：匿名访问 / 跨用户 IDOR / 越权提权。

- **类型**：已移除 crate（历史卡片）
- **历史路径**：`backend/crates/golish-auth-probe/`
- **状态**：🗑️ 已移除

---

## 何时该读这张卡

- 仅在审阅旧提交、旧 transcript 或迁移历史数据时
- 新实现以 superseding design 与 `golish-pentest-app/pentest_bridge` 模块卡为准

## 历史职责

接 js-analyzer 抽出的端点，跑 3 轮确定性 HTTP 检测：匿名访问（无 auth 取数，Critical）、跨用户 IDOR（A 的 token 读 B 的资源，High）、提权（低权 token 触达管理端点，High）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `probe(...)` | HTTP 探测编排 |
| `Scenario` / `Round` / `RoundOutcome` / `Verdict` | 场景/轮次/判定 |
| `ProbeFinding` / `ProbeReport` / `ProbeSummary` / `Evidence` / `Severity` | 结果/证据 |
| `compare_rounds` / `substitute_id` / `SubstituteKind` | 对比/ID 替换 |
| `Endpoint` 等（re-export 自 js-analyzer） | 输入类型 |

## 依赖

- **内部**：`golish-js-analyzer`（消费并 re-export `Endpoint`）

## 被谁依赖 / 改动影响面

无。workspace、组合根与 `golish-pentest-app` 已移除该依赖。

## 关键文件（无目录子模块）

`orchestrator.rs`（`probe`）、`compare.rs`、`request.rs`、`substitute.rs`、`types.rs`。

## 注意事项 / 坑

- 这是直接检 **IDOR/越权** 的核心能力，结果是确定性的（gate 依赖，别把"自信"当通过，I7/I8）。
- 完整契约：`docs/auth-probe-contract.md`。

## 测试入口

无；crate 已移除。旧测试入口仅能在历史提交中运行。
