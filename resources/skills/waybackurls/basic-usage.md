# waybackurls 基础用法

waybackurls 是 tomnomnom 写的最小化 Wayback Machine URL 拉取器。比 gau 简单，只查一个数据源（archive.org），但快、零依赖、流式输出。和 gau 互补。

## 安装

```bash
brew install waybackurls
go install -v github.com/tomnomnom/waybackurls@latest
```

验证：

```bash
which waybackurls       # 看路径
waybackurls --help      # 简短的帮助
```

## 最简

```bash
waybackurls target.com
```

默认包含子域。

## 不要子域

```bash
waybackurls -no-subs target.com
```

## 带日期信息

```bash
waybackurls -dates target.com
```

每行多两个字段：first seen / last seen 时间戳。

## 从 stdin 读多个域

```bash
cat domains.txt | waybackurls > urls.txt
subfinder -d target.com -silent | waybackurls > urls.txt
```

## 一个 URL 的归档版本列表

```bash
echo "https://target.com/old-page" | waybackurls -get-versions
```

返回该 URL 在 Wayback 的所有历史快照 URL。

## 经典 pipeline

```bash
# 1. 历史 URL → httpx 探活 → 落库
waybackurls target.com | httpx -silent | tee live.txt

# 2. 历史 URL → dalfox 直 XSS
waybackurls target.com | dalfox pipe --silence

# 3. 历史 URL → 找参数 fuzz 目标
waybackurls target.com | grep -E '\?[a-zA-Z]+=' | sort -u > params.txt

# 4. 历史 URL → 找泄露文件
waybackurls target.com | grep -E '\.(env|bak|old|backup|sql|git|zip|tar\.gz)$'

# 5. 历史 URL → 找 admin / debug 路径
waybackurls target.com | grep -iE 'admin|debug|test|dev|backup|swagger|api'

# 6. 联合 gau + 去重
(waybackurls target.com; gau target.com --subs) | sort -u > all-urls.txt
```

## 与 katana 互补

| 工具 | 来源 | 时效 |
|---|---|---|
| `waybackurls` | 历史归档 | 历史 URL（含已下线） |
| `katana` | 主动爬当前站 | 现存 URL |

最佳实践：先 wayback / gau 吃历史，再 katana 主动爬现存，sort -u 合并去重。

## waybackurls vs gau

| 维度 | waybackurls | gau |
|---|---|---|
| 数据源 | 1（Wayback） | 4（Wayback + OTX + CC + URLScan） |
| 配置 | 几乎没有 | 多过滤选项 |
| 速度 | 快 | 略慢 |
| 推荐 | 一行命令小测 | 大盘点 |

## 常见坑

- Wayback 拉到的 URL 90% 已 404 → 总是配合 httpx 探活
- 大域时 archive.org 限流 → 拆批处理
- 没有 `--silent` 参数 → 默认就只打 URL，没有 banner
- 命令行参数极少 → 复杂过滤靠 grep / awk 自己接
