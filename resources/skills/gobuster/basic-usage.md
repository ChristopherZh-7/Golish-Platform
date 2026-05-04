# gobuster 基础用法

gobuster 是 Go 写的目录 / DNS / vhost / S3 爆破工具，比 dirb 快、比 ffuf 简单。

## 安装

```bash
brew install gobuster              # macOS
sudo apt install gobuster          # Debian / Ubuntu
go install github.com/OJ/gobuster/v3@latest
```

验证：

```bash
gobuster version
```

## 7 种模式

| 模式 | 作用 |
|---|---|
| `dir` | 目录 / 文件爆破 |
| `dns` | 子域名爆破 |
| `vhost` | 虚拟主机爆破 |
| `fuzz` | HTTP 请求 fuzz（FUZZ 关键字） |
| `s3` | AWS S3 桶爆破 |
| `gcs` | GCS 桶爆破 |
| `tftp` | TFTP 文件爆破 |

## 目录爆破

```bash
gobuster dir -u https://target.com -w /usr/share/wordlists/dirb/common.txt -t 30
```

加扩展名：

```bash
gobuster dir -u https://target.com -w common.txt -x php,html,txt,bak,zip
```

只看特定状态码：

```bash
gobuster dir -u https://target.com -w common.txt -s 200,301,302,401,403
```

跳过 404（默认就跳）：

```bash
gobuster dir -u https://target.com -w common.txt -b 404
```

## 子域名爆破

```bash
gobuster dns -d target.com \
  -w /usr/share/wordlists/seclists/Discovery/DNS/subdomains-top1million-5000.txt \
  -t 50
```

显示 IP：

```bash
gobuster dns -d target.com -w subs.txt --show-ips
```

## vhost 爆破

发现绑定到同一 IP 的不同站点：

```bash
gobuster vhost -u https://target.com \
  -w /usr/share/wordlists/seclists/Discovery/DNS/subdomains-top1million-5000.txt \
  -t 30
```

## 自定义请求

加 cookie / header / UA：

```bash
gobuster dir -u https://target.com -w common.txt \
  -c "session=abcd; remember=1" \
  -a "Mozilla/5.0 (custom)" \
  -H "X-Forwarded-For: 127.0.0.1"
```

## 跳过 TLS 验证（自签证书）

```bash
gobuster dir -u https://target.com -w common.txt -k
```

## 输出

```bash
gobuster dir -u https://target.com -w common.txt -o gobuster.txt -q
```

`-q` 安静模式，`-o` 写文件。

## 与 ffuf 对比

| 维度 | gobuster | ffuf |
|---|---|---|
| 速度 | 快 | 极快 |
| 配置 | 简单（专用子命令） | 灵活（FUZZ 关键字） |
| 多关键字 | 否 | 是 |
| 过滤 | 状态码 + 黑名单 | 状态/大小/词数/行数 都能过滤 |
| 推荐 | 快速跑路 | 精细 fuzz |

入门用 gobuster，深度 fuzz 用 ffuf。
