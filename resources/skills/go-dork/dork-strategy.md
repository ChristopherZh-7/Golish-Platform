# Google Dork 策略：brave_search 优先，go-dork 兜底

做 Google hacking / dork 时，你有两条路：内置的 `brave_search` 工具（API，用用户配置的 Brave key，稳）和 `go-dork`（CLI，免费爬虫，会被封）。**默认用 `brave_search`，把 `go-dork` 当免费补充。**

## 默认顺序

1. **首选 `brave_search`**：API 驱动、不爬网页、不会被 Google 风控。dork 操作符直接写进 `query` 即可。
2. **`go-dork` 仅作免费备选**：当你想省 API 额度、或要用 `-e shodan/bing` 这类 brave 不覆盖的引擎时再用。

## ⚠️ 关键陷阱：go-dork 静默返回空 ≠ 没结果

Google 经常对爬虫**静默限流**：`go-dork` 这时**不报错、直接返回 0 条**。绝不能把空输出当成「目标很干净 / 没搜到」。

判定规则：

- `go-dork` 返回**空**或**报错** → 视为「被封/被限流」，**不是** checked_empty。
- 立即用**同一条 dork query** 改走 `brave_search` 重试一次。
- 只有 `brave_search` 也返回空，才可记 `checked_empty`（带证据，I8：已检查为空 ≠ 未检查）。

## dork 操作符在 brave_search 里怎么用

Brave 支持常见 Google 操作符，把它们塞进 `brave_search` 的 `query` 参数：

| 目的 | query 示例 |
|---|---|
| 域内暴露文件 | `site:acme.com ext:sql OR ext:env OR ext:bak` |
| 目录列表 | `site:acme.com intitle:index.of` |
| 后台/登录面板 | `site:acme.com inurl:admin OR inurl:login` |
| 配置/备份 | `site:acme.com ext:conf OR ext:config OR ext:old` |
| 子域发散 | `site:*.acme.com -www` |

`brave_search` 参数：`{ "query": "...", "count": 20 }`（count 最大 20；要更多结果就翻 dork 关键词，别指望分页）。

## go-dork 用法（兜底时）

```bash
go-dork -q "site:acme.com ext:sql" -e google -p 2 -x http://127.0.0.1:8080
```

- `-x` 代理强烈建议带上（裸跑几乎必被封）。
- `-e` 可选 `google/bing/duckduckgo/yahoo/shodan`；`brave_search` 不覆盖 `shodan` 引擎时，这是 go-dork 的独有价值。

## 回退备选

如果用户没配 Brave key 但配了 Tavily，则用 `tavily_search` 顶替 `brave_search` 充当稳定源；二者皆无 key 时才退回 `go-dork` 裸爬，并在结果里**显式标注「数据源不稳定，空结果不可信」**。

## 一句话

`brave_search`（稳）打主力，`go-dork`（免费）打辅助；**go-dork 空/错一律重试 brave_search**，别拿静默空结果当结论。
