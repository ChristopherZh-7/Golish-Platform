# golish-js-analyzer

> **一句话职责**：JS bundle 静态分析器——从 JS 源码抽取 API 端点调用点（`{method, path, params, auth, body_schema, …}`），供下游 pentest 工具直接消费，不用花 LLM token「读」每个 bundle。

- **类型**：crate（Layer 2/3，叶子）
- **路径**：`backend/crates/golish-js-analyzer/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 从前端 JS 抽取 API 端点、识别 fetch/axios/$.ajax/Request 调用、API 攻击面发现时
- 调整端点抽取的识别模式/置信度时

## 职责

用**正则**（P0，非完整 AST）从 JS 抽取端点调用点。识别 `fetch` / `axios.<verb>` / 自定义客户端 `client.<verb>`（如 `Wr.post('/system/auth/login')`）/ `axios(config)` / `$.ajax` / `new Request`。结果带 `confidence`，调用方可按置信度过滤。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `extract_endpoints(...)` | 抽取端点 |
| `Endpoint` | `{method, path, params, auth, ...}` |
| `AuthHint` / `CallSiteKind` / `UrlKind` | 端点元信息 |

## 依赖

- **内部**：无（叶子）

## 被谁依赖 / 改动影响面

`golish`、`golish-pentest-app`、`golish-auth-probe`（消费 `Endpoint`）。

## 关键文件（无目录子模块）

`lib.rs`（含全部抽取逻辑与类型）。

## 注意事项 / 坑

- P0 是**正则**抽取，不覆盖变量 URL / 无 HTTP 动词的 opaque wrapper（这些回退给 LLM）；`client.<verb>` 属于低一档置信度的确定性规则；P1 计划上 swc AST extractor（同 `extract_endpoints` 签名）。
- `#![forbid(unsafe_code)]` + `#![deny(warnings)]`：改动不能引入 warning。
- 低置信度行由 `Endpoint::confidence` 标记，别当成确定端点。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-js-analyzer
```
