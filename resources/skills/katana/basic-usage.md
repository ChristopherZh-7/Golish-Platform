# katana 基础用法

katana 是 ProjectDiscovery 出的下一代 Web 爬虫，比传统的 hakrawler / gospider 强：自带 headless Chrome，能解析 JS 文件抽提端点，支持 sitemap / robots / form / wayback 多源融合。

## 安装

```bash
brew install katana
go install -v github.com/projectdiscovery/katana/cmd/katana@latest
```

验证：

```bash
katana -version
```

## 最简单一条

```bash
katana -u https://target.com -silent
```

## 调爬深度

```bash
katana -u https://target.com -d 5 -silent
```

`-d 3` 是默认。深爬要 `-d 10` 起。

## 启 headless（解析 SPA）

```bash
katana -u https://target.com -headless -silent
```

会真正跑 JS 渲染，能看到 Vue / React 路由生出的请求。需要本地 Chrome。

## 抽 JS 文件中的端点

```bash
katana -u https://target.com -jc -silent
```

`-jc` = "javascript crawl"，会下载所有 .js 文件 grep 出形如 `fetch("/api/...")` `axios.get("/v1/...")` 的端点。

## 限定 scope

```bash
katana -u https://target.com \
  -cs "target\\.com|cdn\\.target\\.com" \
  -fr "logout|signout|delete" \
  -silent
```

| 参数 | 含义 |
|---|---|
| `-cs` | crawl scope (regex 在内) |
| `-cos` | crawl out scope (regex 不在内) |
| `-mr` | match regex（输出过滤） |
| `-fr` | filter regex（剔除） |

## 指定输出字段

```bash
katana -u https://target.com -f qurl -silent
# 只输出带 query 的 URL，方便给 dalfox / sqlmap 喂数据
```

| -f 值 | 输出 |
|---|---|
| `url` | 完整 URL |
| `qurl` | 带 ?param 的 URL |
| `path` | 仅路径 |
| `fqdn` | 域名 |
| `dir` | 目录 |
| `key` / `value` / `kv` | 参数键 / 值 / 键值对 |
| `ufile` | URL 末段是文件名的 |

## 添加 cookie / header

```bash
katana -u https://target.com \
  -H "Cookie: session=abcd" \
  -H "X-Auth: token" \
  -silent
```

## 速率控制

```bash
katana -u https://target.com -c 20 -p 30 -silent
```

| 参数 | 含义 |
|---|---|
| `-c` | 并发请求 |
| `-p` | 每秒请求上限 |
| `-rl` | 全局 rate limit |

## 表单提交

```bash
katana -u https://target.com -fx -silent
```

`-fx` 会自动尝试提交 HTML 表单，扩大爬取面。

## JSONL 输出

```bash
katana -u https://target.com -jsonl -o katana.jsonl
```

## 经典 pipeline

子域 → 探活 → 爬端点 → XSS 扫：

```bash
subfinder -d target.com -silent \
  | httpx -silent \
  | xargs -I {} katana -u {} -jc -f qurl -silent \
  | dalfox pipe --silence
```

子域 → 爬端点 → nuclei：

```bash
katana -list live.txt -jc -silent | nuclei -tags exposure,config -silent
```

## 代理 / Burp 联调

```bash
katana -u https://target.com -proxy http://127.0.0.1:8080 -silent
```

## 常见坑

- `-headless` 跑 SPA 时第一次启动很慢 → Chrome 冷启
- `-jc` 抽出的端点有大量噪音（404 / 假阳）→ 加 `-f qurl` 过滤
- 不开 scope 时跑大站会爬到 CDN / 三方资源去 → 总是配合 `-cs`
- 爬过深时内存涨 → 加 `-d 5` 限深度 + `-c 20` 限并发
- 部分站点 WAF 见到 katana UA 直接拦 → 加 `-H "User-Agent: Mozilla/5.0..."`
