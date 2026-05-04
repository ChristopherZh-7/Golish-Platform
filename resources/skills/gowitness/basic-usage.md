# gowitness 基础用法

gowitness 是 Sensepost 出的 Web 截图工具，用 headless Chrome 批量给目标网站拍照、写入 SQLite、生成 HTML 报告。在外部资产 / 大量子域名场景里特别有用——比看 1000 行 status code 直观得多。

## 安装

```bash
brew install gowitness
go install github.com/sensepost/gowitness@latest
```

需要本地有 Chrome / Chromium。

验证：

```bash
gowitness version
```

## 5 个子命令

| 子命令 | 用途 |
|---|---|
| `single` | 一个 URL |
| `file` | URL 列表文件 |
| `scan` | CIDR 探测 + 截图 |
| `report` | 启 HTTP server 浏览结果 |
| `server` | REST API 模式（CI/CD 用） |

## 1. 单个 URL

```bash
gowitness single -u https://target.com
```

输出 PNG 在 `screenshots/`，DB 在 `gowitness.sqlite3`。

## 2. URL 列表

```bash
gowitness file -f urls.txt -t 8 --timeout 15
```

`-t` 并发，`--timeout` 单 URL 超时秒。

## 3. CIDR 扫描 + 截图

```bash
gowitness scan --cidr 10.0.0.0/24 --ports 80,443,8080,8443
```

会先 TCP 探活，再对开放端口 HTTP 请求 + 截图。

## 4. 浏览结果

```bash
gowitness report serve --address localhost:7171
```

打开 `http://localhost:7171` 看缩略图墙 + 按 status / title / tech 过滤。

## 经典 pipeline

外部资产盘点：

```bash
subfinder -d target.com -all -silent | httpx -silent | tee live.txt
gowitness file -f live.txt -t 8
gowitness report serve
```

CIDR 扫资产：

```bash
nmap -sn 10.0.0.0/24 -oG - | awk '/Up/{print $2}' > alive.txt
httpx -l alive.txt -p 80,443,8080,8443 -silent > web.txt
gowitness file -f web.txt
```

## 常用参数

| 参数 | 含义 |
|---|---|
| `-t 8` | 并发数 |
| `--timeout 15` | 单 URL 超时 |
| `--resolution-x 1920` | 截图宽 |
| `--resolution-y 1080` | 截图高 |
| `--screenshot-path shots/` | 输出目录 |
| `--user-agent "Mozilla/5.0"` | 自定义 UA |
| `--ignore-cert-errors` | 跳过 SSL 验证 |
| `--db-uri custom.db` | 指定 SQLite |
| `--disable-db` | 不写 DB |

## SQLite 查询（不用 GUI 也能查）

```bash
sqlite3 gowitness.sqlite3 \
  "SELECT url, response_code, title FROM urls WHERE response_code = 200 ORDER BY title"
```

或导出：

```bash
sqlite3 -header -csv gowitness.sqlite3 "SELECT * FROM urls" > all.csv
```

## 自定义 header / cookie

```bash
gowitness file -f urls.txt \
  -H "Cookie: session=abcd" \
  -H "X-Forwarded-For: 127.0.0.1"
```

## REST API 模式

```bash
gowitness server --address 0.0.0.0:7171
```

`POST http://0.0.0.0:7171/submit` 带 JSON `{"url": "..."}` 即可触发截图，CI/CD 用。

## 常见坑

- 第一次跑没装 Chrome → 报 `chromedriver: not found`，安装 `brew install --cask chromium`
- 大量 URL 时 SQLite 锁竞争 → 加 `--disable-db` 然后只看 PNG，速度快很多
- 自签证书站要加 `--ignore-cert-errors`
- 截图小不清楚 → 加大 `--resolution-x 1920 --resolution-y 1080`
- 跑 1000+ URL 时记得磁盘空间，每张 ~200KB
