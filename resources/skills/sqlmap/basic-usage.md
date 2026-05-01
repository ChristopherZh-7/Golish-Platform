# sqlmap 基础用法

## 启动方式

sqlmap 是 Python 脚本，通过 `python sqlmap.py` 或直接 `sqlmap` 运行（如已安装到 PATH）。

## 最小命令

```bash
sqlmap -u "http://target.com/page?id=1" --batch
```

- `-u` 指定带参数的 URL（`?param=value` 格式）
- `--batch` 自动使用默认选项，不需要人工交互

## GET 参数注入

```bash
sqlmap -u "http://target.com/vuln.php?id=1&name=test" --batch --random-agent
```

sqlmap 会自动测试 URL 中的所有参数。

## POST 参数注入

```bash
sqlmap -u "http://target.com/login" --data="username=admin&password=pass" --batch
```

## 指定测试参数

```bash
sqlmap -u "http://target.com/page?id=1&safe=ok" -p id --batch
```

`-p` 指定只测试某个参数，跳过其他参数。

## Cookie 注入

```bash
sqlmap -u "http://target.com/page" --cookie="session=abc123; role=user" --level=2 --batch
```

`--level=2` 或更高才会测试 Cookie 参数。

## 使用代理

```bash
sqlmap -u "http://target.com/page?id=1" --proxy="http://127.0.0.1:8080" --batch
```

配合 Burp Suite 使用时设置代理。

## 常见输出解读

- `Parameter 'id' is vulnerable` → 找到注入点
- `sqlmap identified the following injection point(s)` → 列出注入类型
- `back-end DBMS: MySQL` → 识别了数据库类型
- `[WARNING] GET parameter 'id' does not seem to be injectable` → 参数不可注入
