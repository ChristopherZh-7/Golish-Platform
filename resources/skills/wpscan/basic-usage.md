# wpscan 基础用法

wpscan 是 Ruby 写的 WordPress 漏洞扫描器，能枚举插件 / 主题 / 用户 / 配置备份，并与 wpscan.com 漏洞库联动给出 CVE。

## 安装

```bash
gem install wpscan          # 通用
brew install wpscan         # macOS（自带 Ruby 依赖）
sudo apt install wpscan     # Debian / Ubuntu
```

验证：

```bash
wpscan --version
```

## API Token

去 [wpscan.com](https://wpscan.com) 注册免费账号拿 token（每天 25 次 API），不传也能跑，但只会标"潜在漏洞"，不带 CVE 详情。

放在 `~/.wpscan/scan.yml`：

```yaml
cli_options:
  api_token: YOUR_TOKEN
  random_user_agent: true
```

或每次命令行 `--api-token YOUR_TOKEN`。

## 最小命令

```bash
wpscan --url https://target.com
```

## 标准枚举

```bash
wpscan --url https://target.com -e vp,vt,u --random-user-agent
```

| 标识 | 含义 |
|---|---|
| `vp` | vulnerable plugins |
| `ap` | all plugins |
| `vt` | vulnerable themes |
| `at` | all themes |
| `u` | users |
| `tt` | timthumb 文件 |
| `cb` | config backups |
| `dbe` | DB exports |

## 探测模式

| `--detection-mode` | 含义 |
|---|---|
| `passive` | 仅看 HTML / 资源链接 |
| `aggressive` | 扫已知插件路径（吵闹） |
| `mixed` | 默认，权衡 |

## 用户名枚举

```bash
wpscan --url https://target.com -e u --random-user-agent
```

会通过 `/?author=N` 重定向枚举出真实 username。

## 登录爆破

```bash
wpscan --url https://target.com \
  -U usernames.txt \
  -P /usr/share/wordlists/rockyou.txt \
  --max-threads 2 --throttle 500
```

`--throttle 500` 每次请求间隔 500ms，避免触发限速 / 封号。

## 走代理

```bash
wpscan --url https://target.com --proxy http://127.0.0.1:8080
```

可与 Burp 联动调试请求。

## JSON 报告

```bash
wpscan --url https://target.com -e vp,vt,u -f json -o wpscan.json
```

可用 `jq` 抽取漏洞列表：

```bash
jq '.plugins[].vulnerabilities[] | {plugin: .references.cve, title}' wpscan.json
```

## 跳过 SSL 验证

自签证书：

```bash
wpscan --url https://target.com --disable-tls-checks
```

## 常见坑

- 不带 API token 跑出来"version disclosed but no vulnerabilities" 是正常的，不是没漏洞，是看不到漏洞数据库
- 默认 5 线程对小站够，对 CDN 站会被识别 → 加 `--throttle` + `--random-user-agent`
- 国内网络拉 wpscan 数据库慢 → 第一次跑加 `--no-update`，等晚上让它自己更新
