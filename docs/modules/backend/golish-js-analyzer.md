# golish-js-analyzer

> **一句话职责**：JS bundle 静态分析器——从 JS 源码抽取 API 端点调用点、脱敏后的敏感/config/framework 候选和 rule-based signal 命中，供下游 pentest 工具和 AI 复核直接消费，不用花 LLM token「读」每个 bundle。

- **类型**：crate（Layer 2/3，叶子）
- **路径**：`backend/crates/golish-js-analyzer/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 从前端 JS 抽取 API 端点、识别 fetch/axios/$.ajax/Request 调用、API 攻击面发现时
- 从已落地 JS 中提取 secret/config/framework/library/rule_matches 候选并给 AI 局部复核线索时
- 调整端点抽取的识别模式/置信度时

## 职责

用**正则 + AST-grep call-site filter** 从 JS 抽取端点调用点。识别 `fetch` / `axios.<verb>` / 自定义客户端 `client.<verb>`（如 `Wr.post('/system/auth/login')`）/ `axios(config)` / `$.ajax` / `new Request`。结果带 `confidence`，调用方可按置信度过滤。另有 `signals` 扫描器提取 JWT/API key/token/private key/internal URL、API base/runtime config、常见框架和库，并加载 `resources/js-analysis/js-signal-rules.yml` 的 rule-based signal 命中。输出只含 preview/hash/source_file/line，避免把完整 secret 放进 prompt。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `extract_endpoints(...)` | 抽取端点 |
| `analyze_signals_from_files(...)` / `analyze_signals_from_source(...)` | 抽取脱敏后的 secret/config/framework/library 候选与 rule-based signal 命中 |
| `Endpoint` | `{method, path, params, auth, ...}` |
| `AuthHint` / `CallSiteKind` / `UrlKind` | 端点元信息 |
| `SecretCandidate` / `ConfigCandidate` / `FrameworkCandidate` / `LibraryCandidate` | JS 分析候选，带 source_file/line/confidence |
| `RuleMatchCandidate` / `RuleMatchKind` | 第一层规则候选，带 rule_name/source_rule/group/kind/preview/hash/line/ai_review |

## 依赖

- **内部**：无（叶子）

## 被谁依赖 / 改动影响面

`golish`、`golish-pentest-app`、`golish-auth-probe`（消费 `Endpoint`）。

## 关键文件（无目录子模块）

`lib.rs`（端点抽取公开接口）、`signals.rs`（secret/config/framework/library/rule_matches 候选）、`patterns.rs`（端点模式）、`resources/js-analysis/js-signal-rules.yml`（Rust-regex-compatible signal 规则集）。

## 注意事项 / 坑

- 端点抽取仍不覆盖所有变量 URL / 无 HTTP 动词的 opaque wrapper（这些回退给 AI 局部复核）；`client.<verb>` 属于低一档置信度的确定性规则；P1 计划上 swc AST extractor（同 `extract_endpoints` 签名）。
- `signals` 输出必须保持脱敏：完整 secret 不进入 tool result / prompt；需要人工或模型确认时用 source_file + line 通过文件工具局部查看。`rule_matches` 只是第一层候选，不等于真实漏洞或真实 secret。
- `#![forbid(unsafe_code)]` + `#![deny(warnings)]`：改动不能引入 warning。
- 低置信度行由 `Endpoint::confidence` 标记，别当成确定端点。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-js-analyzer
```
