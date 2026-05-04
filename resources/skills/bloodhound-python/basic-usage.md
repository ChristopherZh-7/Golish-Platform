# bloodhound-python 基础用法

bloodhound-python（也叫 BloodHound.py）是 BloodHound CE / Legacy 的 Linux 端 ingestor。在 Linux 用 Python 通过 LDAP / SMB 远程采集 AD 数据，输出 JSON / Zip 给 BloodHound GUI 渲染攻击路径图。

## 安装

```bash
pipx install bloodhound
# 或
pip install bloodhound
```

验证：

```bash
bloodhound-python --help
```

## BloodHound 的 GUI 端

- **BloodHound CE**（推荐）：Docker 镜像 `specterops/bloodhound`，浏览器访问 8080
- **BloodHound Legacy**：Java + Neo4j，桌面端

GUI 启好后，把 ingestor 输出的 zip 拖进去即可。

## 最简采集

```bash
bloodhound-python \
  -u lowpriv -p 'Pass123' \
  -d lab.local -dc dc01.lab.local -ns 10.0.0.10 \
  -c Default \
  --zip
```

输出 `<timestamp>_bloodhound.zip`，导入到 GUI。

## 常用参数

| 参数 | 含义 |
|---|---|
| `-u user` | 域用户 |
| `-p pass` | 密码（不传会问） |
| `--hashes LM:NT` | Pass-the-Hash |
| `-d lab.local` | 域 |
| `-dc dc01.lab.local` | DC FQDN |
| `-ns 10.0.0.10` | DNS 解析器（DC IP） |
| `-c Default` | 采集模块 |
| `--zip` | 打包成 zip |
| `--auth-method ntlm/kerberos/auto` | 认证方式 |
| `--ldap-channel-binding` | LDAP 签名/绑定开启时必填 |

## 采集模块

| -c 值 | 含义 |
|---|---|
| `Default` | Group / LocalAdmin / Session / Trusts（轻量） |
| `All` | 全部（很慢，含 ACL/Container/ObjectProps） |
| `DCOnly` | 只问 DC（最快，但缺少会话信息） |
| `Group` | 组成员关系 |
| `LocalAdmin` | 本地 Admin 关系 |
| `Session` | 主机当前登录会话 |
| `RDP` | RDP 用户 |
| `DCOM` / `PSRemote` | DCOM / WinRM 用户 |
| `Trusts` | 域信任 |
| `ACL` | ACL（最常用于找权限滥用） |
| `Container` | 容器（OU/GPO） |
| `ObjectProps` | 对象属性细节 |
| `LoggedOn` | 当前登录用户（强引擎） |

实战推荐：

```bash
bloodhound-python -u u -p p -d d.local -dc dc -ns 10.0.0.10 -c Default,ACL --zip
```

## 不同认证方式

PtH：

```bash
bloodhound-python -u admin --hashes :8846f7eaee... -d lab.local -dc dc01 -ns 10.0.0.10 -c Default --zip
```

Kerberos（先用 impacket 拿 ccache）：

```bash
KRB5CCNAME=admin.ccache bloodhound-python -u admin -k -no-pass -d lab.local -c Default --zip
```

## DC 不能直连场景（chisel 隧道后）

```bash
# chisel 把 DC 的 LDAP/RPC 转到本机
bloodhound-python -u u -p p -d lab.local -dc dc01.lab.local -ns 127.0.0.1 -c Default --zip
```

## 把 zip 喂进 BloodHound CE

GUI → File → Upload Data → 拖 zip 进去。

然后用 Cypher 查询找路径：

- "Shortest paths from owned principals to Domain Admins"
- "Find all kerberoastable accounts"
- "Find computers where Domain Users can RDP"

## 与其它工具配合

- **netexec** 拿到第一个域账号 → bloodhound-python 跑全图
- **bloodhound** GUI 找出 attack path → 用 `nxc` / `impacket` 落地
- **certipy** 检查 ADCS（bloodhound-python 不查 CS）

## 常见坑

- DC 不通 → 先用 chisel 隧道，再 `-ns 127.0.0.1`
- LDAP 签名启用 → 加 `--ldap-channel-binding`，必要时切 LDAPS（加 `--ldap-port 636 --use-ssl`）
- 采集慢 → 先用 `-c DCOnly` 看图，再追加细分采集
- BloodHound CE 和 Legacy 数据 schema 略有差异 → 用对应版本的 ingestor
- 不要在生产域跑 `-c All` —— 会发巨量 LDAP 查询
