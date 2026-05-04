# searchsploit 基础用法

searchsploit 是 [exploit-db.com](https://www.exploit-db.com/) 的离线命令行接口。本地有一份 EDB 全库（5w+ 个 exploit）的 git 仓库，命令查到的就是真实文件，可以直接 mirror 出来跑。

## 安装

```bash
git clone https://gitlab.com/exploit-database/exploitdb.git /opt/exploitdb
sudo ln -s /opt/exploitdb/searchsploit /usr/local/bin/
# Kali / 大部分发行版自带
```

验证：

```bash
searchsploit --version
```

## 基础搜索

```bash
searchsploit windows smb 2017
searchsploit -t apache 2.4.49        # 仅匹配 title
searchsploit -s "openssh 7.7"         # 严格匹配（不切词）
```

## 按 CVE

```bash
searchsploit --cve 2017-0144
searchsploit --cve CVE-2021-44228
```

## 排除噪音

```bash
searchsploit windows smb --exclude="DoS|/poc/"
```

## 看一个 exploit 的本地路径

```bash
searchsploit -p 42315
```

输出：

```
  Exploit: Windows Kernel - SMB Driver
      URL: https://www.exploit-db.com/exploits/42315
     Path: /opt/exploitdb/exploits/windows/remote/42315.py
File Type: ASCII text
Copied EDB-ID #42315 path to clipboard
```

## 拿一个 exploit 到当前目录

```bash
searchsploit -m 42315
# 文件会复制到 ./42315.py
```

## 看一下 exploit 内容（用 $PAGER 打开）

```bash
searchsploit -x 42315
```

## 拿在线链接

```bash
searchsploit -w windows smb 2017
```

每行追加 exploit-db.com 的 URL，方便丢浏览器看。

## JSON 输出

```bash
searchsploit -j -t apache 2.4.49 | jq '.RESULTS_EXPLOIT'
```

## 与 nmap 串起来

```bash
nmap -sV -oX scan.xml target
searchsploit --nmap scan.xml
```

会按 nmap 探测到的服务版本逐个查相关 exploit。

## 更新本地 EDB 库

```bash
searchsploit -u
# 或
cd /opt/exploitdb && git pull
```

建议每周更新。

## 常用 EDB 分类

| 路径 | 含义 |
|---|---|
| `exploits/` | 真正的 exploit |
| `exploits/dos/` | DoS |
| `exploits/remote/` | 远程利用 |
| `exploits/local/` | 本地提权 |
| `exploits/webapps/` | Web 应用 |
| `shellcodes/` | 各平台 shellcode |
| `papers/` | 漏洞分析文档 |

## 实战例

找到 nmap 报告里有 `vsftpd 2.3.4`：

```bash
searchsploit -t vsftpd 2.3.4
# 找到：
# Linux | exploits/unix/remote/17491.rb     (Backdoor Command Execution)
searchsploit -m 17491
ruby 17491.rb
```

## 常见坑

- 自己的 EDB 库太老 → `searchsploit -u` 更新（或 `git pull`）
- `-t` 加 `-w` 组合最实用：title 匹配 + 看链接
- 大量 exploit 是 PoC 不能直接打目标，要看代码改 LHOST/RHOST
- 现代 EDR 见过这些 exploit 几乎都拦截，仅在授权环境用
