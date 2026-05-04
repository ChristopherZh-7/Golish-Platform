# nmap 基础用法

## 安装

```bash
brew install nmap         # macOS
sudo apt install nmap     # Debian / Ubuntu
sudo dnf install nmap     # RHEL / Fedora
```

验证：

```bash
nmap --version
```

## 最小命令

```bash
nmap scanme.nmap.org
```

默认会扫描 1000 个最常见的 TCP 端口。

## 指定端口范围

```bash
nmap -p 22,80,443 192.168.1.10        # 特定端口
nmap -p 1-1000 192.168.1.10           # 端口区间
nmap -p- 192.168.1.10                  # 全部 65535 端口
nmap --top-ports 100 192.168.1.10     # Top 100 常见端口
```

## 三种扫描技术

| 参数 | 说明 | 是否需要 root |
|---|---|---|
| `-sS` | TCP SYN（半开扫描，最常用） | 是 |
| `-sT` | TCP Connect（完整三次握手） | 否 |
| `-sU` | UDP 扫描 | 是 |
| `-sn` | Ping sweep（不扫端口，仅探测存活） | 否 |

## 服务版本探测

```bash
nmap -sV scanme.nmap.org              # 探测开放端口的服务版本
nmap -sV --version-intensity 5 target # 加大探测强度
```

## 操作系统识别

```bash
sudo nmap -O 192.168.1.10
```

## 全功能侦察（OS + 版本 + 脚本 + traceroute）

```bash
sudo nmap -A -T4 192.168.1.10
```

## 时序模板

`-T0`（最慢，规避 IDS）→ `-T5`（最快，吵闹）。日常用 `-T4`。

## 输出三件套

```bash
nmap -A -oA scan_result 192.168.1.10
# 同时生成 scan_result.nmap / .gnmap / .xml
```

## 常见坑

- 不加 `sudo` 时 `-sS` / `-O` / `-sU` 会失败
- 网段大时不要直接用 `-p-`，会很慢
- `--script vuln` 跑全集慢且容易触发 WAF，先用 `-sV` 缩小目标
