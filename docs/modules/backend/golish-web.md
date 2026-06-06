# golish-web

> **一句话职责**：Web 搜索与内容抓取——Tavily / Brave 搜索集成 + 网页抓取/提取，封装成 agent 工具。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-web/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Tavily/Brave 搜索、网页抓取/提取/爬取、web_fetch 时
- agent 联网搜索工具（tavily_*/brave）相关时

## 职责

提供联网搜索与抓取能力，并封装成 `Tool`。由 golish-tools 在配置了对应 API key 时条件注册。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `TavilyState` / `BraveSearchState` | 搜索 provider 状态 |
| `create_tavily_tools` / `create_brave_tools` | 构建工具集 |
| `WebSearchTool` / `WebSearchAnswerTool` / `WebExtractTool` / `WebCrawlTool` / `WebMapTool` / `BraveSearchTool` | 各工具 |
| `WebFetcher` / `FetchResult` | 网页抓取 |

## 依赖

- **内部**：`golish-core`

## 被谁依赖 / 改动影响面

`golish-tools`（条件注册联网搜索工具）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `tavily/` | Tavily 搜索集成 | [→](golish-web/tavily.md) |
| `tool/` | 各 Web `Tool` 实现 | [→](golish-web/tool.md) |

## 关键文件

`brave.rs`（Brave 搜索）、`web_fetch.rs`（网页抓取）。

## 注意事项 / 坑

- 工具是否启用取决于 API key（见 golish-tools 的条件注册），测试别假设它们总在。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-web
```
