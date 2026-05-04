# nmap NSE 脚本进阶

NSE（Nmap Scripting Engine）让 nmap 不只是端口扫描器，还能做漏洞检测、暴力破解、信息搜集。

## 脚本分类

| 类别 | 用途 |
|---|---|
| `auth` | 认证类 |
| `broadcast` | 广播探测 |
| `brute` | 暴力破解 |
| `default` | 默认脚本（`-sC` 启用） |
| `discovery` | 服务发现 |
| `dos` | 拒绝服务（慎用） |
| `exploit` | 利用漏洞（慎用） |
| `fuzzer` | 模糊测试 |
| `intrusive` | 侵入式 |
| `malware` | 恶意软件检测 |
| `safe` | 安全只读 |
| `version` | 版本探测 |
| `vuln` | 漏洞识别 |

## 默认脚本

```bash
nmap -sC -sV target
# 等价于 --script=default --script-args 配合
```

## 跑特定脚本

```bash
nmap --script smb-vuln-ms17-010 -p 445 192.168.1.10    # 永恒之蓝
nmap --script http-enum -p 80,443 target               # Web 路径枚举
nmap --script ssl-enum-ciphers -p 443 target           # SSL 加密套件
nmap --script ssh-auth-methods -p 22 target            # SSH 认证方式
```

## 批量跑漏洞类

```bash
nmap -sV --script vuln target
```

慢且吵闹，建议结合 `-p` 缩端口范围。

## 脚本组合

```bash
nmap --script "http-* and not http-brute" -p 80,443 target
nmap --script "default,vuln,safe" target
```

## 自带常用脚本速查

| 脚本 | 用途 |
|---|---|
| `dns-brute` | DNS 子域名爆破 |
| `http-title` | 网站标题 |
| `http-headers` | 响应头 |
| `http-methods` | 支持的 HTTP 方法 |
| `ftp-anon` | 匿名 FTP 检测 |
| `mysql-info` | MySQL 信息 |
| `redis-info` | Redis 信息 |
| `smb-os-discovery` | SMB OS 信息 |
| `vnc-info` | VNC 协议信息 |

## 脚本参数

```bash
nmap --script http-form-brute --script-args 'userdb=users.txt,passdb=pass.txt' -p 80 target
```

## 自定义脚本路径

NSE 脚本默认在 `/usr/share/nmap/scripts/`（macOS Homebrew 在 `/opt/homebrew/share/nmap/scripts/`）。可放自己的 `.nse` 文件然后用绝对路径调用。

## 与其它工具对接

输出 `-oX scan.xml` 后可以丢给 `searchsploit --nmap scan.xml` 查 EDB 利用，或丢给 metasploit 的 `db_import`。
