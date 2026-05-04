# resources/wordlists/

> 渗透测试常用字典本地缓存。**所有 `.txt` / `.lst` / `.tar.gz` 文件已在 `.gitignore` 里忽略**——字典是数据不是代码，不入库。

## 一键下载（默认 ~1 MB）

```bash
./scripts/download_wordlists.sh
```

加参数：

```bash
./scripts/download_wordlists.sh --force       # 已存在也强制重下
./scripts/download_wordlists.sh --extra       # 额外拉 rockyou (~134 MB)
```

运行后这里会出现：

```
common.txt                          (~50 KB)  目录爆破基础
raft-small-directories.txt          (~250 KB) 中量目录
raft-small-files.txt                (~250 KB) 中量文件
quickhits.txt                       (~50 KB)  高命中路径
subdomains-top1million-5000.txt     (~50 KB)  Top 5K 子域
subdomains-top1million-20000.txt    (~200 KB) Top 20K 子域
burp-parameter-names.txt            (~25 KB)  HTTP 参数名
api-endpoints.txt                   (~10 KB)  API 路径
top-usernames-shortlist.txt         (< 1 KB)  喷洒用户名
xato-net-10m-usernames-dup-1k.txt   (~10 KB)  Top 1K 用户名
passwords-top1k.txt                 (~10 KB)  密码 Top 1K
probable-v2-top1575.txt             (~15 KB)  高概率密码
```

加 `--extra` 还会下：

```
rockyou.txt                         (~134 MB) 经典泄漏密码全集
fasttrack.txt                       (~7 KB)   常用账号密码
```

## 来源 & 授权

所有字典来自 [danielmiessler/SecLists](https://github.com/danielmiessler/SecLists)（MIT License）。本仓库仅自动下载，**不重新打包，不修改内容**。

如需更全的字典：

```bash
git clone https://github.com/danielmiessler/SecLists \
  resources/wordlists/SecLists
# ↑ 整库 ~1.2 GB，只在本机用，不会被 git 跟踪
```

## 与项目内工具的对应关系

| 字典 | 工具用法示例 |
|---|---|
| `common.txt` | `ffuf -u target/FUZZ -w resources/wordlists/common.txt` |
| `raft-small-directories.txt` | `gobuster dir -u target -w resources/wordlists/raft-small-directories.txt` |
| `subdomains-top1million-5000.txt` | `gobuster dns -d target.com -w resources/wordlists/subdomains-top1million-5000.txt` |
| `burp-parameter-names.txt` | `ffuf -u "target?FUZZ=test" -w resources/wordlists/burp-parameter-names.txt -fs 0` |
| `top-usernames-shortlist.txt` | `nxc smb 10.0.0.0/24 -u resources/wordlists/top-usernames-shortlist.txt -p Pass --continue-on-success` |
| `passwords-top1k.txt` | `hydra -L users.txt -P resources/wordlists/passwords-top1k.txt ssh://target -t 4 -f` |
| `rockyou.txt` | `hashcat -m 1000 hashes.txt resources/wordlists/rockyou.txt -r rules/best64.rule` |

## 自定义字典

把你自己的字典 .txt 直接丢进这个目录就好，不会被 git 跟踪。

如果是公司专用资产名 / 内部域名 / 历史泄露密码——一律放本地，**不要 commit**。

## 重新生成 / 更新

字典会随 SecLists 更新。需要刷新本地副本时：

```bash
./scripts/download_wordlists.sh --force
```

## 没网 / 离线场景

如果跑脚本时拉不到（防火墙 / 离线），可以：

1. 在能联网的机器上跑一次脚本
2. 把整个 `resources/wordlists/` 目录拷到目标机器
3. 或者预先 git clone SecLists 到 U 盘
