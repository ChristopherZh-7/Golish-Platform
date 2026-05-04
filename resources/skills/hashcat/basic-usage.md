# hashcat 基础用法

hashcat 是号称世界最快的 GPU 加速哈希破解器，OpenCL/CUDA 后端，支持 200+ 种 hash mode。比 john 难学一点，但速度差距能到 10×~100×。

## 安装

```bash
brew install hashcat                  # macOS
sudo apt install hashcat              # Debian / Ubuntu
```

需要 GPU 驱动：

| 平台 | 驱动 |
|---|---|
| macOS | 系统自带 Metal/OpenCL |
| Linux NVIDIA | `nvidia-driver` + CUDA |
| Linux AMD | ROCm |
| Windows NVIDIA | NVIDIA drivers |

验证：

```bash
hashcat --version
hashcat -I              # 列出可用计算设备（GPU/CPU）
```

## 命令模板

```
hashcat -m <hash-mode> -a <attack-mode> [options] <hashfile> [wordlist|mask]
```

最简单：MD5 + rockyou：

```bash
hashcat -m 0 -a 0 hashes.txt /usr/share/wordlists/rockyou.txt
```

## 常用 -m 哈希模式

| -m | 类型 | 备注 |
|---|---|---|
| 0 | MD5 | 32 位 hex |
| 100 | SHA1 | 40 位 |
| 1400 | SHA256 | 64 位 |
| 1700 | SHA512 | 128 位 |
| 1000 | NTLM | Windows 本地账户 |
| 5500 | NetNTLMv1 | 老 SMB |
| 5600 | NetNTLMv2 | Responder 抓的 |
| 13100 | Kerberos 5 TGS-REP | Kerberoast |
| 18200 | Kerberos 5 AS-REP | AS-REP roast |
| 500 | md5crypt ($1$) | 老 Linux shadow |
| 1800 | sha512crypt ($6$) | 现代 Linux shadow |
| 3200 | bcrypt ($2$) | Web 应用常见 |
| 22000 | WPA-PBKDF2-PMKID+EAPOL | Wi-Fi |
| 13400 | KeePass | KeePass 1/2 |
| 9600 | MS Office 2013 | docx 等 |

完整：`hashcat --help | grep -i ntlm`（按关键字搜）

## 5 种攻击模式 -a

| -a | 名称 | 说明 |
|---|---|---|
| 0 | Straight | 字典 |
| 1 | Combination | 两份字典两两组合 |
| 3 | Brute / Mask | 掩码 / 暴力 |
| 6 | Hybrid Wordlist + Mask | 字典词 + 掩码尾巴 |
| 7 | Hybrid Mask + Wordlist | 掩码头 + 字典词 |
| 9 | Association | 用户名/盐做提示词 |

## 字典 + 规则

```bash
hashcat -m 0 -a 0 hashes.txt rockyou.txt -r rules/best64.rule
```

著名规则集：
- `best64.rule`（hashcat 自带）
- `OneRuleToRuleThemAll.rule`（社区，覆盖广）
- `dive.rule`（更狠的变形）

## 掩码暴力

```bash
hashcat -m 0 -a 3 hashes.txt ?l?l?l?l?l?l?d?d
```

| 占位 | 字符集 |
|---|---|
| `?l` | a-z |
| `?u` | A-Z |
| `?d` | 0-9 |
| `?s` | 特殊符号 |
| `?a` | 全可打印 |
| `?h` | 0-9, a-f |
| `?H` | 0-9, A-F |

自定义字符集：`-1 ?l?u?d` 然后用 `?1`。

## Hybrid

字典词 + 4 位数字（最常见的密码模式）：

```bash
hashcat -m 0 -a 6 hashes.txt rockyou.txt ?d?d?d?d
```

## 速度调优

| 参数 | 含义 |
|---|---|
| `-w 3` | workload 0-4，default 2，挖矿用 4 |
| `-O` | 优化内核（限定 pwd 长度 ≤ 32 但快 5x） |
| `-d 1,2` | 选指定设备（多 GPU） |
| `--force` | 忽略警告 |

跑前先看预估速度：

```bash
hashcat -m 0 -b               # benchmark MD5
hashcat -m 1000 -b            # NTLM benchmark
```

## 暂停 & 恢复

`s` 键：状态；`p` 键：暂停；`r` 键：恢复；`q` 键：保存退出。

```bash
hashcat -m 1000 -a 0 hashes.txt rockyou.txt --session=run1 -O
# Ctrl-C 中断
hashcat --session=run1 --restore
```

## 查看 cracked

```bash
hashcat -m 1000 --show hashes.txt > cracked.txt
```

破解结果默认保存在 `~/.local/share/hashcat/hashcat.potfile`。

## 实战工作流

```bash
# Responder 抓到 NetNTLMv2
echo 'admin::WORKGROUP:1122334455667788:...' > ntlm.hash
hashcat -m 5600 -a 0 ntlm.hash rockyou.txt -r rules/best64.rule -O

# Kerberoast TGS
hashcat -m 13100 -a 0 tgs.hash rockyou.txt -r rules/OneRuleToRuleThemAll.rule

# WPA PMKID（hcxdumptool 抓到）
hashcat -m 22000 -a 0 wpa.22000 rockyou.txt
```

## 常见坑

- macOS 的 GPU 速度有限，跑长任务用 Linux + NVIDIA
- 加 `-O` 限制密码长度但能快 5×，不知情时先试
- 同一目录跑多个 session 时一定要 `--session=name`，不然会互相覆盖
- bcrypt / scrypt 等慢哈希 GPU 也救不了，准备等几小时
