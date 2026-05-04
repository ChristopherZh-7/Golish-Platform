# amass 基础用法

OWASP Amass 是攻击面映射的"重武器"——子域名 / IP / ASN / 关联域名 / 信任关系全在一个工具里。比 subfinder 更全（也更慢，更吵），适合关键目标深挖。

## 安装

```bash
brew install amass
go install -v github.com/owasp-amass/amass/v4/...@master
```

验证：

```bash
amass version
```

## 5 个子命令

| 子命令 | 用途 |
|---|---|
| `enum` | 子域名枚举（最常用） |
| `intel` | 组织 / ASN / whois 关联 |
| `track` | 两次扫描 diff |
| `viz` | 生成可视化图 |
| `db` | 管理本地数据库 |

## 1. 子域名枚举

最快（被动 OSINT）：

```bash
amass enum -d target.com -passive -silent
```

主动模式（含证书 grep / DNS attack）：

```bash
amass enum -d target.com -active -silent
```

主动 + brute：

```bash
amass enum -d target.com -active -brute -w subdomains.txt -silent
```

| 模式 | 含义 |
|---|---|
| `-passive` | 仅查公开数据源，不发包到目标 |
| `-active` | 启 cert grab、DNS zone walk 等会发包 |
| `-brute` | 加字典 brute |
| 默认 | 综合模式 |

## 2. 组织 / ASN 情报

按组织名反查域名：

```bash
amass intel -org "Cloudflare" -whois
```

按 ASN 找资产：

```bash
amass intel -asn 13335
```

按 CIDR 反查：

```bash
amass intel -cidr 104.16.0.0/12
```

## 3. JSON 输出（喂下游）

```bash
amass enum -d target.com -json amass.jsonl
```

每行带 sources / IP / type，能用 jq 抽：

```bash
jq -r '.name' amass.jsonl | sort -u > all-subs.txt
```

## 4. API 配置

`~/.config/amass/config.yaml` 加 API key 能让 passive 模式威力翻倍：

```yaml
options:
  resolvers:
    - 1.1.1.1
    - 8.8.8.8
datasources:
  - name: shodan
    creds:
      account:
        apikey: YOUR_KEY
  - name: virustotal
    creds:
      account:
        apikey: YOUR_KEY
  - name: securitytrails
    creds:
      account:
        apikey: YOUR_KEY
  - name: censys
    creds:
      account:
        apikey: YOUR_KEY
        secret: YOUR_SECRET
```

支持 100+ 数据源。

## 5. 经典 pipeline

```bash
# 全资产盘点
amass enum -d target.com -active -silent | tee subs.txt | httpx -silent | gowitness file -f -

# 跟 nuclei 串
amass enum -d target.com -passive -silent | httpx -silent | nuclei -tags cve,exposure
```

## amass vs subfinder

| 维度 | amass | subfinder |
|---|---|---|
| 速度 | 慢 | 快 |
| 数据源 | 100+ | 30+ |
| 主动模式 | 有（zone walk / cert / brute） | 无 |
| ASN / Org 关联 | 有 | 无 |
| JSON 元数据 | 详尽 | 一般 |
| 推荐 | 关键目标深挖 | 大批量域名快速过 |

实战：先 subfinder 一遍快速过，再对要深挖的目标跑 amass active。

## 常见坑

- 不配 API key 的 passive 模式只能看到 5-10 个源 → 拿到的子域很少
- active 模式会发大量 DNS / TLS 探测 → 容易被 WAF / IDS 标
- 跑大域 (>10K 子域) 时本地 DNS 资源会卡 → `-r` 指定 1.1.1.1 / 8.8.8.8 / Quad9
- macOS 上跑 brute 模式被 mDNS 干扰，加 `-r 1.1.1.1,8.8.8.8`
- 默认会写入本地 SQLite，多次跑同个域会增量；要清空用 `amass db -names -d target.com`
