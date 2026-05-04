# nuclei 基础用法

nuclei 是 ProjectDiscovery 出品的模板化漏扫器。用 YAML 模板描述请求 + 匹配规则，由它驱动并发扫描。社区已有 8000+ 模板覆盖 CVE / 配置 / 暴露 / 弱口令。

## 安装

```bash
brew install nuclei                                # macOS
go install -v github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest
```

第一次运行会自动拉取模板到 `~/nuclei-templates/`。手动更新：

```bash
nuclei -ut
```

## 最简单一条

```bash
nuclei -u https://target.com
```

## 按严重度过滤

```bash
nuclei -u https://target.com -severity high,critical
```

## 按 tag 过滤

```bash
nuclei -u https://target.com -tags cve,rce
nuclei -u https://target.com -tags exposure,config -severity medium,high,critical
```

排除：

```bash
nuclei -u https://target.com -etags dos,intrusive,fuzz
```

## 跑特定模板 / 目录

```bash
nuclei -u https://target.com -t cves/2024/
nuclei -u https://target.com -t exposures/configs/git-config.yaml
```

## 批量目标

```bash
nuclei -l targets.txt -severity high,critical -o nuclei.txt
```

`targets.txt` 一行一个 URL / 域名。

## 输出 JSONL（喂给其它工具）

```bash
nuclei -l targets.txt -jsonl -o nuclei.jsonl
```

JSONL 每行一个 finding，含 `template-id` / `info.name` / `severity` / `host` / `matched-at` 等字段。

## Markdown 报告

```bash
nuclei -u https://target.com -me report-md/
```

## 加 Header / Cookie

```bash
nuclei -u https://target.com -H "Cookie: session=abcd" -H "X-Auth: xyz"
```

## 走代理（与 Burp 联调）

```bash
nuclei -u https://target.com -proxy http://127.0.0.1:8080
```

## 速率控制

```bash
nuclei -u https://target.com -c 25 -bs 25 -rl 150
```

| 参数 | 含义 |
|---|---|
| `-c` | 并发模板数 |
| `-bs` | 每个模板的目标批量 |
| `-rl` | 每秒请求数上限 |

## 实战工作流

```bash
# 1. 收集子域 + 探活
subfinder -d target.com -silent | httpx -silent > alive.txt

# 2. 跑高危 CVE
nuclei -l alive.txt -tags cve -severity high,critical -o cve.txt

# 3. 跑暴露 / 配置
nuclei -l alive.txt -tags exposure,config -severity medium,high,critical -o exposure.txt
```

## 写自定义模板

模板是 YAML，最简单的 GET-200 检查：

```yaml
id: my-check
info:
  name: My Check
  severity: info
http:
  - method: GET
    path:
      - "{{BaseURL}}/admin/panel"
    matchers:
      - type: status
        status:
          - 200
```

放到 `~/nuclei-templates/my/` 然后 `-t my/my-check.yaml` 即可。

## 常见坑

- 第一次运行很慢 —— 在拉模板，耐心等
- `-tags fuzz` 会触发主动注入测试，授权环境再用
- DOS 类模板默认排除，需要 `-etags ""` 才会被运行
