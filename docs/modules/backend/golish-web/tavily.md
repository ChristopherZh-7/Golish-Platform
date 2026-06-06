# golish-web / tavily

> **一句话职责**：Tavily web 搜索集成——`TavilyState` 持 API key + HTTP client，封装 Tavily 的 search / extract / crawl / map 端点，含结果类型。

- **类型**：目录模块（属于 crate [`golish-web`](../golish-web.md)）
- **路径**：`backend/crates/golish-web/src/tavily/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Tavily API 调用（search/extract/crawl/map）、key 配置（settings + env 回退）时
- 改 Tavily 结果/请求类型时

## 职责

`TavilyState` 管理 Tavily API key 与 `reqwest` client，发 search / extract / crawl / map 请求并解析。结果类型对外暴露给 `tool/`（封装成 agent 工具）。key 支持 settings 配置 + 环境变量回退。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TavilyState`（`from_api_key`） | API key 状态 + HTTP client |
| `SearchResults` / `SearchResult` / `ExtractResults` / `ExtractResult` / `CrawlResults` / `CrawlResult` / `MapResults` / `AnswerResult` | 对外结果类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `TavilyState` + HTTP 调用 + 公开结果类型 |
| `types.rs` | 请求/响应 wrapper + 结果类型 |

## 依赖

- `reqwest`、`serde`、`anyhow`；常量 `TAVILY_BASE_URL = https://api.tavily.com`

## 注意事项 / 坑

- 内部请求/响应 wrapper 不对外，只暴露结果类型（`SearchResults` 等）；agent 工具在 `tool/`。
- key 缺失时 `api_key: None`——工具应优雅报"未配置"而非 panic。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-web tavily
```
