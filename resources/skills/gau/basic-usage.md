# gau (Get All URLs) 基础用法

gau 从公开档案（Wayback Machine、AlienVault OTX、Common Crawl、URLScan）拉取一个域名**历史上**被记录的所有 URL。**不向目标发任何包**，纯被动 OSINT，对存量大、历史悠久的目标特别有用。

## 安装

```bash
brew install gau
go install -v github.com/lc/gau/v2/cmd/gau@latest
```

验证：

```bash
gau --version
```

## 最简用法

```bash
gau target.com
```

会输出几百到几十万行 URL，根据目标历史长度而定。

## 包含子域

```bash
gau target.com --subs
```

注意：默认**不**包含子域。

## 指定数据源

```bash
gau target.com --providers wayback,otx
```

四个内建：`wayback / otx / commoncrawl / urlscan`。

## 时间窗口

```bash
gau target.com --from 202301 --to 202412
```

格式 YYYYMM。只看最近一年：

```bash
gau target.com --from 202504 --to 202604
```

## 过滤 URL

按状态码（基于 Wayback 的快照状态）：

```bash
gau target.com --mc 200,301
gau target.com --fc 404,500
```

按文件后缀：

```bash
gau target.com --me php,asp,aspx,jsp                              # 只要动态
gau target.com --fe css,js,jpg,png,svg,gif,woff,woff2,ttf,ico    # 排除静态资源
```

## 多域名批处理

```bash
cat domains.txt | gau --threads 10 -o all-urls.txt
```

或：

```bash
subfinder -d target.com -silent | gau --subs > urls.txt
```

## JSON 输出

```bash
gau target.com --json | jq '.url'
```

JSON 行包含 `url` / `provider` 等字段。

## 经典 pipeline

历史 URL → 现存检查 → XSS：

```bash
gau target.com --subs --fe "css,js,jpg,png,svg,woff" \
  | httpx -silent \
  | dalfox pipe --silence
```

历史 URL → 仅带 ? 参数 → fuzz：

```bash
gau target.com --subs | grep '?' | sort -u > params.txt
ffuf -u "$(head -1 params.txt)" -w payloads.txt
```

历史 URL → 找泄露的 secret：

```bash
gau target.com --subs --me js | httpx -silent | xargs -I {} curl -s {} | grep -E "api[_\-]?key|secret|token"
```

历史 URL → nuclei 扫敏感配置：

```bash
gau target.com --subs --me "git,env,bak,old,sql,zip,backup" | nuclei -tags exposure
```

## gau vs waybackurls

| 维度 | gau | waybackurls |
|---|---|---|
| 数据源 | 4 个 | 1 个（Wayback） |
| 速度 | 较慢 | 快 |
| 过滤功能 | 丰富 | 极简 |
| 推荐 | 全面盘点 | 快速过 |

实战常一起用 + sort -u 去重：

```bash
(gau target.com --subs; waybackurls target.com) | sort -u > urls.txt
```

## API 配置

OTX / URLScan 的速率限制可以加 API key 缓解。`~/.config/gau/.gau.toml`：

```toml
[urlscan]
apikey = "YOUR_KEY"

[otx]
apikey = "YOUR_KEY"
```

## 常见坑

- 拿到的 URL 是历史快照，不代表现在还能访问 → 总是接 `httpx` 探活再用
- Common Crawl 数据延迟几个月，新功能/路径可能没有 → 配合主动爬虫 katana
- 大域可能拉到 100w+ URL → `sort -u` 去重 + 尽早 grep
- Wayback 对带敏感参数的 URL 也会归档 → 偶尔能在历史 URL 里捡到 token / secret
