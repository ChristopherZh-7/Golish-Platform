# ffuf 基础用法

ffuf（Fuzz Faster U Fool）是 Go 写的高速 Web fuzzer。核心思想：用 `FUZZ` 关键字标记 URL / 数据 / Header 中要替换的位置，然后从字典里依次代入。

## 安装

```bash
brew install ffuf                  # macOS
go install github.com/ffuf/ffuf/v2@latest
```

验证：

```bash
ffuf -V
```

## 最简单的目录爆破

```bash
ffuf -u https://target.com/FUZZ -w /usr/share/wordlists/dirb/common.txt
```

`FUZZ` 会被字典每行替换。结果默认显示状态码、大小、行数、词数。

## 过滤 / 匹配

只看 200 / 301 / 302 / 403：

```bash
ffuf -u https://target.com/FUZZ -w common.txt -mc 200,301,302,403
```

排除 404 大小为 1234 的伪 200：

```bash
ffuf -u https://target.com/FUZZ -w common.txt -fs 1234
```

| 参数 | 含义 |
|---|---|
| `-mc` | match status code |
| `-ms` | match size |
| `-mw` | match words |
| `-ml` | match lines |
| `-fc` | filter status code |
| `-fs` | filter size |
| `-fw` | filter words |
| `-fl` | filter lines |

## 文件后缀爆破

```bash
ffuf -u https://target.com/FUZZ -w common.txt -e .php,.bak,.txt,.html,.zip
```

## 参数发现

```bash
ffuf -u "https://target.com/api?FUZZ=test" -w params.txt -fs 0
```

## 子域名 / vhost 探测

vhost：

```bash
ffuf -u https://target.com -H "Host: FUZZ.target.com" -w subdomains.txt -fs 0
```

## POST 数据 fuzz

```bash
ffuf -u https://target.com/login \
     -X POST \
     -d "username=FUZZ&password=test" \
     -w users.txt \
     -fc 401
```

## 多关键字

```bash
ffuf -u https://target.com/PATH/FILE \
     -w paths.txt:PATH \
     -w files.txt:FILE
```

## 速率与并发

```bash
ffuf -u https://target.com/FUZZ -w common.txt -t 80 -rate 200
```

`-t` 是并发线程，`-rate` 是每秒请求上限。

## 输出 JSON

```bash
ffuf -u https://target.com/FUZZ -w common.txt -of json -o ffuf.json
```

## 递归

```bash
ffuf -u https://target.com/FUZZ -w common.txt -recursion -recursion-depth 2
```

## 常见坑

- 不过滤 `-fs / -fc` 会被默认 404 页淹没
- `-recursion` 容易爆站，先跑一层再决定要不要递归
- 字典质量决定一切，推荐 SecLists：`/usr/share/seclists/Discovery/Web-Content/`
