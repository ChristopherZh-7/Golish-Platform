# Burp Suite Community 基础用法

Burp Suite 是 PortSwigger 出的 Web 安全测试事实标准。Community Edition 免费但**没有主动扫描器**和速率限制；Professional 才有 Active Scan、Intruder 全速、BCheck 等。

## 安装

```bash
brew install --cask burp-suite              # macOS
sudo apt install burpsuite                   # Debian / Ubuntu (Kali 自带)
```

要求 Java 17+，Burp 自带捆绑 JDK，一般不用单独装。

验证：

```bash
burpsuite --diagnostics                      # 命令行启动检查
```

## 启动模式

```bash
burpsuite                                    # GUI 默认
burpsuite --use-defaults                     # 跳过启动对话框
burpsuite --project-file=engagement.burp     # 打开/创建项目（持久化）
```

Community 不支持自动保存项目，必须用 `--project-file=` 才能下次接着干。

## 基础工作流：浏览器代理

1. 启 Burp → Proxy → Intercept is **off**（先关，免得每个请求都拦）
2. 浏览器代理设 `127.0.0.1:8080`（推荐用 Firefox + FoxyProxy）
3. 装证书：`http://burp` → 下 `cacert.der` → 浏览器导入信任 CA
4. 浏览目标网站，请求自动进 Proxy → HTTP history

## 6 大模块速览

| 模块 | 用途 |
|---|---|
| **Proxy** | 拦截 / 修改 HTTP(S) |
| **Target** | 站点地图 + Scope 过滤 |
| **Intruder** | 参数 fuzz / 爆破 |
| **Repeater** | 单包重放修改 |
| **Decoder** | 编码 / hash / 解码 |
| **Comparer** | diff 两个响应 |
| **Extender** | 装 BApp Store 插件 |
| **Logger** | 全部请求日志 |

## Repeater（最常用）

1. 在 Proxy → HTTP history 找到要测的请求
2. 右键 → Send to Repeater
3. 切到 Repeater tab，改 body / header 任意位置
4. 点 Send，看响应

## Intruder（爆破 / fuzz）

Community 版很慢（限速），但小量任务还能用：

1. 拦到一个登录请求 → Send to Intruder
2. Positions tab：清掉默认 § 标记，手动选要 fuzz 的位置（点 Add §）
3. Payloads tab：贴字典 / 用内置生成器
4. Attack：开始

四种 attack type：

| 类型 | 含义 |
|---|---|
| Sniper | 单参数轮替（最常用） |
| Battering Ram | 同 payload 写所有位置 |
| Pitchfork | 多参数同步用各自字典（一对一对应） |
| Cluster Bomb | 多参数全组合（笛卡尔积） |

## 设 Scope（避免你的代理被全网请求淹）

Target → Scope → Add → URL pattern → 勾"Use advanced scope control"。

设完之后：

- Proxy → 勾"Show only in-scope items"
- Repeater 里只重放 in-scope 请求

## 常用 BApp（免费插件）

Extender → BApp Store：

| BApp | 用途 |
|---|---|
| **Logger++** | 强化的全请求日志 + 高级过滤 |
| **Autorize** | 越权（IDOR / privilege）半自动测试 |
| **JWT Editor** | JWT 解码 / 签名 / 攻击 |
| **Param Miner** | 隐藏参数挖掘 |
| **Turbo Intruder** | 高速 Intruder（绕 Community 限速） |
| **Active Scan++** | 增强 Active Scan（Pro 才完整生效） |
| **Backslash Powered Scanner** | 异常注入扫描 |
| **HTTP Request Smuggler** | smuggling 测试 |

## 拦截 HTTPS

证书 `cacert.der` 装到浏览器**信任根 CA**，不是中间证书。装错的常见症状：浏览器一切 https → "证书不安全"。

iOS / 移动设备：装 PortSwigger CA 到设备 → 设置 → 通用 → 关于 → 证书信任设置 → 启用。

## 与 Frida / mitmproxy 的差异

- **Burp**：抓 HTTP(S) 流量，最适合 Web 应用
- **Frida**：动态插桩，最适合移动 App / 原生程序
- **mitmproxy**：CLI / Python，适合自动化、自定义脚本

Web 测试用 Burp 没毛病。

## 常见坑

- Community 版 Intruder 限速 → 大字典爆破跑不动，要么装 Turbo Intruder，要么升级到 Pro
- 默认 Proxy 监听 127.0.0.1:8080，移动设备测试要改成 0.0.0.0
- HTTP/2 + h2c 时部分功能（Repeater 的 follow redirects）有兼容问题
- 项目文件超过 1G 时启动慢 → 做完一阶段就 archive
- 现代 Web 大量 WebSocket / SSE → Burp 都能拦但需要在 Proxy → WebSockets history 里看
