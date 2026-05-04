# httpx 基础用法

httpx（ProjectDiscovery 的）是个**并发 HTTP 探活+指纹**工具，每秒能跑几百到上千个目标。注意：和 Python 的 `httpx` HTTP 客户端不是一回事，安装时别装混。

## 安装

```bash
brew install httpx                              # macOS
go install -v github.com/projectdiscovery/httpx/cmd/httpx@latest
```

确认装的是 PD 版：

```bash
httpx -version       # 应输出 ProjectDiscovery 版本
```

## 最小用法 —— 把存活的 URL 挑出来

```bash
cat hosts.txt | httpx -silent
# 或
httpx -l hosts.txt -silent -o live.txt
```

## 加状态/标题/技术栈

```bash
httpx -l hosts.txt -sc -title -td -server
```

输出例：

```
https://target.com [200] [Welcome] [nginx] [WordPress, jQuery]
```

| 参数 | 含义 |
|---|---|
| `-sc` | 状态码 |
| `-title` | 页面标题 |
| `-td` | tech detect（类似 wappalyzer） |
| `-server` | Server 头 |
| `-cl` | content-length |
| `-cdn` | 是否 CDN |
| `-ip` | 解析 IP |
| `-cname` | CNAME 链 |
| `-asn` | ASN 信息 |

## 过滤状态码

```bash
httpx -l hosts.txt -mc 200,301,302              # 只看 200/301/302
httpx -l hosts.txt -fc 404,500                  # 排除 404/500
```

## 多端口探测

```bash
httpx -l hosts.txt -ports 80,443,8080,8443,8000,8888
```

## JSON 输出（喂下游工具）

```bash
httpx -l hosts.txt -json -o httpx.jsonl
```

每行包含 url / status / title / tech / cdn / ip / hash 等丰富字段。

## 截图

```bash
httpx -l hosts.txt -ss -silent
```

会启 chromium 抓图（需先装 headless chrome），保存到 `output/screenshot/`。

## 相似度去重

跑 fuzzer 后会有上千个看起来一样的 200，httpx 可按 hash 去重：

```bash
httpx -l urls.txt -hash sha256 -title | sort -u -k2
```

## 跟在 subfinder 后面

```bash
subfinder -d target.com -silent | httpx -silent | tee alive.txt
subfinder -d target.com -silent | httpx -td -title -ip
```

## 跟 nuclei 串

```bash
subfinder -d target.com -silent \
  | httpx -silent \
  | nuclei -tags cve -severity high,critical
```

## 速率与并发

```bash
httpx -l hosts.txt -t 100 -rl 200 -timeout 5
```

| 参数 | 含义 |
|---|---|
| `-t` | 并发线程 |
| `-rl` | 每秒请求上限 |
| `-timeout` | 请求超时秒 |

## 自定义 header

```bash
httpx -l hosts.txt -H "X-Forwarded-For: 127.0.0.1" -H "Cookie: a=b"
```

## 常见坑

- 安装混了 Python 的 `pip install httpx` —— 那个是异步 HTTP 客户端，不是这个
- 默认只探 80/443，记得加 `-ports`
- 大量 wildcard 子域会出现一堆假 200 → 加 `-fl` 按 line count 过滤同质响应
