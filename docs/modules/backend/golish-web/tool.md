# golish-web / tool

> **一句话职责**：把 web 能力封装成 `golish_core::Tool`——`WebSearchTool` / `BraveSearchTool` / `WebCrawlTool` / `WebMapTool`，并提供 `create_tavily_tools` / `create_brave_tools` 工厂注册进工具 registry。

- **类型**：目录模块（属于 crate [`golish-web`](../golish-web.md)）
- **路径**：`backend/crates/golish-web/src/tool/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 web 搜索/抓取的 agent 工具行为、参数 schema、结果格式时
- 加新 web 工具或改 Tavily/Brave 工具工厂时

## 职责

实现 `golish_core::Tool` trait 把 web 能力暴露给 LLM：`WebSearchTool`（Tavily 搜索）、`BraveSearchTool`（Brave 搜索）、`WebCrawlTool`、`WebMapTool`。工厂 `create_tavily_tools` / `create_brave_tools` 批量构造供 registry 注册。

## 公开接口

| 符号 | 说明 |
|---|---|
| `WebSearchTool` | Tavily 搜索工具 |
| `BraveSearchTool` | Brave 搜索工具 |
| `WebCrawlTool` / `WebMapTool` | 抓取 / 站点地图工具 |
| `create_tavily_tools` / `create_brave_tools` | 工具工厂 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `WebSearchTool` 等工具实现 |
| `crawl.rs` | crawl/map/brave 工具 + 工厂 |

## 依赖

- `golish_core::Tool` + `utils`、crate 内 `tavily::TavilyState`、`serde_json`

## 注意事项 / 坑

- 工具持 `Arc<TavilyState>`：key 未配置时返回错误 JSON（带 `error`），别 panic。
- 工具 schema 与 `tavily/` 的实际 API 参数要一致；改参数两边同步。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-web tool
```
