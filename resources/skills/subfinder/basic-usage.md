# subfinder 基础用法

subfinder 是 ProjectDiscovery 出的**被动**子域名收集工具。它只查公开数据源（CT 日志、搜索引擎、漏洞情报站），不主动 DNS 爆破，因此不会被目标察觉。

## 安装

```bash
brew install subfinder                         # macOS
go install -v github.com/projectdiscovery/subfinder/v2/cmd/subfinder@latest
```

验证：

```bash
subfinder -version
```

## 最简单一条

```bash
subfinder -d target.com -silent
```

`-silent` 关掉横幅，只输出子域名（适合管道）。

## 用所有数据源

```bash
subfinder -d target.com -all -silent
```

`-all` 会用上所有源（其中一些需要 API Key）。第一次运行后会生成 `~/.config/subfinder/provider-config.yaml`，把 API key 写进去：

```yaml
shodan:
  - YOUR_SHODAN_KEY
virustotal:
  - YOUR_VT_KEY
```

支持的数据源（节选）：
crtsh / virustotal / shodan / chaos / securitytrails / fofa / hunterio / passivetotal / threatcrowd 等。

## 批量域名

```bash
subfinder -dL domains.txt -all -silent -o all-subs.txt
```

## 输出 JSONL（含来源元数据）

```bash
subfinder -d target.com -all -oJ -o subs.jsonl
```

每行带 `source` 字段，方便分析哪个源出的最多。

## 经典 pipeline

```bash
# 收集子域 → 探活 → 截图 → CVE 扫
subfinder -d target.com -silent \
  | httpx -silent \
  | tee alive.txt \
  | nuclei -tags cve -severity high,critical
```

## 加自定义 DNS 解析器

公共 DNS 偶尔被污染，可换：

```bash
subfinder -d target.com -rL resolvers.txt
```

`resolvers.txt` 一行一个 IP（如 1.1.1.1 / 8.8.8.8）。

## 排除子域

`-eS` 排除特定源：

```bash
subfinder -d target.com -eS commoncrawl -silent
```

## 限制时间

防止某个源拖死整个流程：

```bash
subfinder -d target.com -all -timeout 30 -max-time 5 -silent
```

`-timeout` 是单源超时秒，`-max-time` 是总超时分钟。

## 与主动 brute 配合

被动收集 + 主动 brute 才完整。subfinder 拿底，再用：

```bash
gobuster dns -d target.com -w subs-top1m.txt -t 100
# 或 amass -d target.com -active
```

## 常见坑

- 不配 API key → 只有 4-5 个免费源能用，结果偏少
- 用 `-all` 加 API key 才能挖到 70%+ 的子域
- 结果里有 wildcard 子域（*.target.com 解到同一个 IP）→ 用 `httpx` 探活去掉
