# masscan 基础用法

masscan 是 Robert Graham 写的极速 TCP 端口扫描器，号称能在 6 分钟内扫完整个互联网。原理是异步发包，不维护 TCP 状态机，因此**很快但只能告诉你"端口开"，不像 nmap 还能识别服务**。

## 安装

```bash
brew install masscan                  # macOS
sudo apt install masscan              # Debian / Ubuntu
```

验证：

```bash
masscan --version
```

## 一行扫子网

```bash
sudo masscan 192.168.1.0/24 -p 80,443 --rate 1000
```

`--rate` 是每秒发包数，默认 100；可以加到 100 万（取决于网卡）。

## 常用语法

| 形式 | 含义 |
|---|---|
| `192.168.1.0/24` | CIDR |
| `10.0.0.1-10.0.0.254` | 范围 |
| `1.1.1.1,8.8.8.8` | 列表 |
| `0.0.0.0/0` | 全互联网（务必加 excludefile） |

端口：

```bash
-p 80
-p 80,443
-p 1-65535
-p U:53,123                # UDP
-p T:80,U:53               # TCP + UDP 混合
```

## 必备 excludefile（防止扫到自家关键设备）

```
# exclude.conf
10.0.0.0/8
172.16.0.0/12
192.168.0.0/16
127.0.0.0/8
```

```bash
sudo masscan 0.0.0.0/0 -p 80 --rate 100000 --excludefile exclude.conf -oL web.list
```

## 输出格式

```bash
sudo masscan 10.0.0.0/8 -p 80,443 --rate 5000 -oL scan.list      # 简单列表
sudo masscan 10.0.0.0/8 -p 80,443 --rate 5000 -oG scan.gnmap     # grep 友好
sudo masscan 10.0.0.0/8 -p 80,443 --rate 5000 -oJ scan.json      # JSON
sudo masscan 10.0.0.0/8 -p 80,443 --rate 5000 -oX scan.xml       # XML（喂 metasploit）
```

简单列表格式：

```
open tcp 80 1.2.3.4 1685000000
open tcp 443 1.2.3.4 1685000000
```

## banner 抓取（慢但有用）

```bash
sudo masscan 10.0.0.0/16 -p 80,443,21,22,25 --banners --rate 500
```

带 banner 时 masscan 会维护 TCP 状态，速度大幅下降，但能拿到 HTTP server header 等。

## 暂停 + 恢复

Ctrl-C 后会写入 `paused.conf`，恢复：

```bash
sudo masscan --resume paused.conf
```

## 经典两阶段：masscan + nmap

```bash
# Stage 1: masscan 全端口快速发现
sudo masscan 10.0.0.0/24 -p 1-65535 --rate 10000 -oG masscan.gnmap

# Stage 2: 仅对发现的开放端口跑 nmap 版本探测
nmap -sV -iL <(awk '/Status: Up/{print $2}' masscan.gnmap)
```

## 速率与稳定性

| 网卡 | 安全 rate |
|---|---|
| 1 Gbps | ~1.5 M pps（理论值） |
| 100 Mbps | ~150K pps |
| WiFi | 不要超 1000 |

跑高速度建议：

- 用 PF_RING / DPDK 替代 libpcap
- 关闭目标侧无关 NAT
- 单独网卡专跑 masscan

## 常见坑

- 没 sudo 跑不了原始 socket
- 跑公网时 ISP / 数据中心可能告警 → 一定加 excludefile + 控速度
- 不能识别 service，只能识别"端口开" → 一定接 nmap 二阶段
- macOS 上 `--rate` 上限受 BPF 缓冲限制
