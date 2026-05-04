# nikto 基础用法

nikto 是一款老牌的 Web 服务器漏洞扫描器，主要查危险文件、过期版本、配置错误。它**不隐蔽**，会被 WAF 和 IDS 立刻识别。

## 安装

```bash
brew install nikto                # macOS
sudo apt install nikto             # Debian / Ubuntu
```

验证：

```bash
nikto -Version
```

## 最简单一条命令

```bash
nikto -h https://target.com
```

## 显式端口与 SSL

```bash
nikto -h target.com -p 443 -ssl
nikto -h target.com -p 80,443,8080,8443
```

## 调整测试范围（Tuning）

| Tuning | 含义 |
|---|---|
| 1 | 有意思的文件 / 日志 |
| 2 | 配置错误 / 默认文件 |
| 3 | 信息泄漏 |
| 4 | 注入（XSS / Script / HTML） |
| 5 | 远程文件读取 |
| 6 | DOS |
| 7 | 远程文件读取（服务端） |
| 8 | 命令执行 / shell |
| 9 | SQL 注入 |
| x | 反向调优（排除某项） |

例如只跑信息泄漏 + 注入 + SQLi：

```bash
nikto -h target.com -Tuning 349
```

## 输出格式

```bash
nikto -h target.com -Format json -output report.json
nikto -h target.com -Format htm -output report.html
nikto -h target.com -Format csv -output report.csv
```

## 跑认证后的扫描

```bash
nikto -h target.com -id admin:admin                          # Basic Auth
nikto -h target.com -Cookies "PHPSESSID=abcd; remember=1"   # Cookie 鉴权
```

## 常见坑

- nikto 会被 WAF 拦截 —— 真实环境可加 `-evasion 1` 试编码绕过，但效果有限
- 默认会探测大量 HTTP 路径（5000+），容易被告警
- 单线程，不快；建议先用 `nmap -sV` 圈定 Web 端口，再跑 nikto
- 要做正经 Web 测试，nikto + ffuf + nuclei 组合更专业

## 更新规则库

```bash
nikto -update
```
