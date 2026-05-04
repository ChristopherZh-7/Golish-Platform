# Wireshark 基础用法

Wireshark 是图形化网络协议分析器，行业标准。命令行版叫 `tshark`，功能等价但适合脚本化。

## 安装

```bash
brew install --cask wireshark            # macOS（GUI）
sudo apt install wireshark                # Debian / Ubuntu
```

macOS 装好后还要把当前用户加 `access_bpf` 组才能不用 sudo 抓包；安装包里的 "Install ChmodBPF" 会做这件事。

## 启动

```bash
wireshark                                 # 直接打开 GUI
wireshark -i en0                          # 指定接口启动
wireshark -r capture.pcapng               # 打开已有 pcap
```

## 看可用接口

```bash
wireshark -D
# 或命令行版：
tshark -D
```

## BPF 抓取过滤器（开始抓包前）

写在 `-f` 后面，决定**抓什么进来**。语法和 tcpdump 一样。

| 过滤 | 含义 |
|---|---|
| `tcp port 80` | TCP 80 端口 |
| `host 10.0.0.5` | 与该主机所有流量 |
| `src host 10.0.0.5` | 该主机发出的 |
| `net 10.0.0.0/24` | 该子网 |
| `udp port 53` | DNS 流量 |
| `not port 22` | 排除 SSH |
| `tcp port 80 or tcp port 443` | 多端口 |

```bash
wireshark -i en0 -f "tcp port 80 or tcp port 443"
```

## Display 过滤器（抓完后查询）

写在 GUI 的过滤栏，或命令行 `-Y`。语法**不一样**，更强大：

| 过滤 | 含义 |
|---|---|
| `http` | HTTP 协议 |
| `http.request.method == "POST"` | POST 请求 |
| `http.host contains "example.com"` | 主机匹配 |
| `ip.addr == 10.0.0.5` | 源或目标 IP |
| `ip.src == 10.0.0.5 and ip.dst == 8.8.8.8` | 精确方向 |
| `tcp.flags.syn == 1 and tcp.flags.ack == 0` | TCP SYN |
| `dns.qry.name contains "google"` | DNS 查询名 |
| `tls.handshake.extensions_server_name == "example.com"` | SNI |
| `tcp.stream eq 5` | 第 5 条 TCP 流 |

```bash
tshark -r cap.pcapng -Y 'http.request.method == "POST"' -T fields -e http.host -e http.request.uri
```

## 跟随流（Follow Stream）

GUI：右键某条 TCP/UDP/TLS 包 → Follow → TCP/UDP/TLS Stream。
命令行：

```bash
tshark -r cap.pcapng -q -z follow,tcp,ascii,5
```

## 常用统计

GUI 菜单 Statistics 里的：

- **Conversations**：哪两个 IP 聊了多少
- **Endpoints**：每个 IP 的总流量
- **Protocol Hierarchy**：协议占比
- **HTTP > Requests**：HTTP 请求统计
- **DNS**：DNS 查询统计

命令行：

```bash
tshark -r cap.pcapng -q -z conv,tcp
tshark -r cap.pcapng -q -z http,tree
tshark -r cap.pcapng -q -z dns,tree
```

## 提取文件 / 凭据

Wireshark GUI：File → Export Objects → HTTP / SMB / TFTP / IMF 等可还原传输的文件。

明文凭据：

```bash
tshark -r cap.pcapng -Y 'http.authbasic' -T fields -e http.authbasic
```

## 抓 + 写入文件 + 限制大小

```bash
sudo tshark -i en0 -w out.pcapng -b filesize:50000 -b files:5
# 每个 50MB，循环 5 个文件
```

## 解 TLS（带 SSLKEYLOGFILE 时）

浏览器/curl 设环境变量：

```bash
export SSLKEYLOGFILE=$HOME/sslkey.log
```

Wireshark：Edit → Preferences → Protocols → TLS → "(Pre)-Master-Secret log filename" 指到该文件，就能看到明文 HTTPS。

## 常见坑

- 普通用户看不到接口 → macOS 装 ChmodBPF / Linux 把用户加 `wireshark` 组
- 抓 WiFi 时默认在 monitor mode 才能看到非自己流量；macOS 用 `airportd` 或 GUI 的 "Capture Options → Monitor"
- pcap 大于几 GB 时 GUI 会卡 → 用 tshark 先过滤再用 Wireshark 看
- TLS 1.3 不再有 master secret 字段，必须用 SSLKEYLOGFILE 才能解
