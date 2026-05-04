# John the Ripper 基础用法

john（俗称 JtR）是经典的 CPU 侧密码哈希破解工具，对 GPU 加速依赖小（要 GPU 用 hashcat）。Jumbo 版支持 200+ 种哈希格式。

## 安装

```bash
brew install john-jumbo                # macOS（Jumbo 版，必装）
sudo apt install john                   # Debian / Ubuntu
```

验证：

```bash
john --version
john --list=formats | head             # 查所有支持的哈希格式
```

## 哈希文件格式

每行一个：

```
$1$abc$XYZ...
```

或 `user:hash` 格式：

```
admin:$2y$10$...
```

NTLM dump（pwdump 格式）：

```
admin:1001:LM_HASH:NT_HASH:::
```

## 自动检测 + 字典攻击

```bash
john --wordlist=/usr/share/wordlists/rockyou.txt hashes.txt
```

不指定 `--format=` 时 john 会猜，多种可能时报错让你选。

## 指定格式

常见值：

| 格式 | 哈希示例 |
|---|---|
| `raw-md5` | 32 位 hex MD5 |
| `raw-sha1` | 40 位 SHA1 |
| `raw-sha256` | 64 位 SHA256 |
| `md5crypt` | `$1$salt$hash` |
| `sha512crypt` | `$6$salt$hash` |
| `bcrypt` | `$2a$/$2y$/$2b$...` |
| `nt` | NTLM (32 位 hex) |
| `lm` | LanMan |
| `netntlmv2` | Net-NTLMv2 |
| `krb5tgs` | Kerberoast TGS |
| `ZIP` / `rar` / `pdf` | 文件密码 |

## 单词 + 规则（mangling）

```bash
john --format=nt --wordlist=rockyou.txt --rules hashes.txt
john --wordlist=passwords.txt --rules=Best64 hashes.txt
```

`--rules` 不带值默认套全部内置规则；常用的轻量规则集：

| 规则集 | 特点 |
|---|---|
| `Best64` | 64 条最有效规则 |
| `Single` | 仅 single 模式用的 |
| `Wordlist` | 标准变形 |
| `Jumbo` | Jumbo 版扩展规则 |

## 增量爆破

```bash
john --incremental=Digits hashes.txt           # 纯数字
john --incremental=Lower hashes.txt            # 小写字母
john --incremental=Alnum hashes.txt            # 字母+数字
john --incremental=ASCII hashes.txt            # 全可打印
```

## 掩码攻击

```bash
john --mask=?u?l?l?l?l?d?d hashes.txt
```

| 占位 | 字符集 |
|---|---|
| `?l` | a-z |
| `?u` | A-Z |
| `?d` | 0-9 |
| `?s` | 特殊符号 |
| `?a` | 全部可打印 |

## Single 模式

利用 GECOS、用户名等做变形：

```bash
john --single hashes.txt
```

对 SSH / shadow 类账户特别有效。

## 多核并行

```bash
john --fork=4 --wordlist=rockyou.txt hashes.txt
```

## 查看结果

```bash
john --show hashes.txt                 # 已破解
john --show --format=nt hashes.txt     # 指定格式查看
```

破解结果保存在 `~/.john/john.pot`。

## 暂停 & 恢复

```bash
john --session=ssh1 --wordlist=rockyou.txt hashes.txt
# Ctrl-C 中断后
john --restore=ssh1
```

## 与其它工具串

shadow 文件：

```bash
unshadow /etc/passwd /etc/shadow > unshadowed.txt
john --format=sha512crypt --wordlist=rockyou.txt unshadowed.txt
```

ZIP / RAR / PDF：

```bash
zip2john secret.zip > zip.hash
john --format=ZIP --wordlist=rockyou.txt zip.hash
```

类似的转换器还有 `rar2john`、`pdf2john.pl`、`ssh2john`、`keepass2john`。

## 常见坑

- 普通 john（非 jumbo）格式很少 → macOS / Linux 都装 jumbo 版
- rockyou.txt 在 macOS Homebrew 不会自动有，要从 SecLists 拷过来
- `--rules` 后跟字典叠加规则会爆炸增长，1G 字典 + 全规则可能跑几小时
