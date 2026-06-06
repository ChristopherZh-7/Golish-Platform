# 主动收集覆盖清单（Active Recon Coverage Matrix）· 设计草案

> **目的**：把 harness 的两个主动阶段 `external_attack_surface`（测绘）+ `enumeration`（枚举）从「AI 手调 nmap/httpx/subfinder 几个工具、gate 只要 Surface+JsApi 两类」升级成 **覆盖驱动**：定义一张「每个 in-scope 资产要覆盖哪些主动收集维度」的清单，AI 用一套齐全的工具库自己挑工具把每个维度盖满，gate 按清单卡——盖不全过不了关。
>
> **承接**：`docs/design/2026-06-06-intel-stage-ai-driven-per-mode.md`（被动 intel AI 驱动 P0，已实现）、`docs/design/2026-06-05-coverage-matrix.md`（coverage 矩阵 gate 积木）、`docs/design/2026-06-05-vuln-triage-technique-matrix.md`（漏洞技术矩阵，本设计的同构姊妹）、`resources/harness/technique_taxonomy.json`（技术类登记表，本设计新增 GOLISH-ASM-*/GOLISH-ENUM-*）。
>
> **方案选型**：用户 2026-06-06 拍板「路 B 覆盖驱动」（= 字段驱动的升级版）——不写死流水线（路 A），而是定维度清单 + AI 自由挑工具 + gate 卡完整度。
> **状态**：Direction Draft（待用户增删维度后再出实现计划）。日期：2026-06-06。

---

## 0. 决策（TL;DR）

- **问题**：现网 `external_attack_surface` / `enumeration` 逻辑太薄——AI 只手调 subfinder/nmap/httpx，gate（`surface_coverage`）只要求 Surface（http_service/fingerprint）+ JsApi（api_endpoint）两类 + `min_invocations`（dns_resolve/http_probe/subdomain_enum_passive）。真正的主动测绘+枚举维度有十几类，现状只盖一小角。
- **方向（路 B · 覆盖驱动）**：
  1. **定维度清单**：本文件 §2/§3 的 `GOLISH-ASM-*`（测绘）/ `GOLISH-ENUM-*`（枚举）两组维度，登记进 `technique_taxonomy.json`。
  2. **配工具库**：每个维度给一组推荐工具（AI 经 `pentest_run` 自由挑，不写死顺序）。
  3. **gate 按清单卡**：两个 stage 的 `expected_techniques` 填本清单 → 复用现有 `coverage_complete`：每个 in-scope 资产 × 每个该阶段维度都要有终态（found+证据 / checked_empty+证据 / blocked|not_applicable+note）。
  4. **字段是覆盖的产物**：用户先前要的「8 个固定字段」（端口/标题/状态码/Web服务器/技术栈/真实IP/CDN-WAF/OS）是这些维度的**输出**，写回 `targets` 表（`manage_targets update_recon` 扩字段）。
- **非目标**：不写死流水线；不强制 AI 用某个具体工具（只给推荐）；本期不动漏洞阶段（vuln_triage 往后）；不删同事的 `run_active_collection`（可留作「一键全套」加速器，AI 可选调）。
- **地板非天花板**：本清单是**最低必盖**，AI 可超出（对齐 coverage-matrix「矩阵是地板」原则）。

---

## 1. 两阶段分工（边界）

| 阶段 | 定位 | 一句话 | 授权级 |
|---|---|---|---|
| **external_attack_surface** | 横向铺开：**摊开攻击面** | 「有哪些资产入口」——子域/站点/IP/指纹/截图 | L2 轻主动 |
| **enumeration** | 纵向深挖：**每个入口往里挖** | 「每个入口上有啥端口/目录/内容/API」 | L2 主动 |

> 边界判定：**发现新资产/入口** → 测绘；**对已知入口深挖内容** → 枚举。端口扫描归枚举（现 enumeration `allowed_tool_types` 含 recon/port-scan）。

---

## 2. external_attack_surface 覆盖维度（GOLISH-ASM-*）

| id | 维度 | 盖什么 | 推荐工具 | evidence kind | 粒度 | 必盖? |
|---|---|---|---|---|---|---|
| `GOLISH-ASM-SUBDOMAIN` | 子域名枚举 | 被动 + **主动爆破** + 排列组合(permutation) | subfinder, amass, puredns, dnsx, altdns/gotator | `subdomain` | per-root-domain | ✅ |
| `GOLISH-ASM-DNS` | DNS 解析 | A/AAAA/CNAME 解析、泛解析(wildcard)检测 | dnsx, massdns | `dns_a` | per-host | ✅ |
| `GOLISH-ASM-LIVENESS` | HTTP 探活 | 探活、状态码、跳转链、标题 | httpx | `http_probe` | per-host | ✅ |
| `GOLISH-ASM-FINGERPRINT` | 指纹识别 | Web 服务器 / 技术栈 / framework / CMS | httpx(-td), whatweb, wappalyzer | `fingerprint` | per-site | ✅ |
| `GOLISH-ASM-WAFCDN` | WAF/CDN 识别 | 是否在 CDN/WAF 后 | wafw00f, cdncheck, httpx(cdn) | `waf_cdn` | per-site | ✅ |
| `GOLISH-ASM-REALIP` | 真实 IP | 绕 CDN 找源站(历史DNS/证书/favicon hash) | (历史DNS, tlsx, favicon) | `real_ip` | per-site(在CDN后才适用) | ⚠️ conditional |
| `GOLISH-ASM-SCREENSHOT` | 截图 | 可视化快速分类 | gowitness, aquatone | `screenshot` | per-site | ⚠️ 建议 |
| `GOLISH-ASM-CERT` | 证书分析 | TLS 证书 SAN 掏更多资产 | tlsx | `certificate` | per-site | ⚠️ 建议 |
| `GOLISH-ASM-VHOST` | VHost/SNI | 一个 IP 上多站点 | httpx(vhost), 字典 | `vhost` | per-ip | ⚠️ conditional |
| `GOLISH-ASM-NETBLOCK` | 网段/ASN 存活 | ASN→CIDR→主机存活(有授权时) | dnsx/naabu(ping) | `host_alive` | per-asn | ⚠️ conditional |

---

## 3. enumeration 覆盖维度（GOLISH-ENUM-*）

| id | 维度 | 盖什么 | 推荐工具 | evidence kind | 粒度 | 必盖? |
|---|---|---|---|---|---|---|
| `GOLISH-ENUM-PORT` | 端口 + 服务 | 端口、服务版本、banner（必要时 UDP） | nmap, naabu, masscan | `open_port` | per-host | ✅ |
| `GOLISH-ENUM-DIR` | 目录/内容爆破 | 递归目录、隐藏路径 | ffuf, feroxbuster, gobuster, dirsearch | `dir_entry` | per-site | ✅ |
| `GOLISH-ENUM-CRAWL` | 爬虫 | 链接 / 表单 / 入口参数 | katana, hakrawler | `crawl_url` | per-site | ✅ |
| `GOLISH-ENUM-JSAPI` | JS→API+密钥 | 抓 JS、提 API 端点、泄露密钥 | js_collect + js_extract_apis, LinkFinder | `api_endpoint` | per-site | ✅ |
| `GOLISH-ENUM-PARAM` | 参数发现 | 隐藏参数 | arjun, paramspider, x8 | `parameter` | per-endpoint | ⚠️ 建议 |
| `GOLISH-ENUM-APISPEC` | API 规格枚举 | swagger/openapi、GraphQL introspection | (curl + 解析) | `api_spec` | per-site(有API才适用) | ⚠️ conditional |
| `GOLISH-ENUM-SENSITIVE` | 敏感/备份文件 | .git/.env/备份包/配置泄露 | nuclei(exposures), ffuf(sensitive 字典) | `sensitive_file` | per-site | ⚠️ 建议 |

---

## 4. 与「固定字段」的关系（用户的 8 字段是覆盖的产物）

用户先前要的 `targets` 表固定字段，由对应维度**产出后写回**（`manage_targets update_recon` 扩字段，T-impl）：

| 字段 | 来源维度 |
|---|---|
| ports（端口+服务） | GOLISH-ENUM-PORT |
| http_title / http_status | GOLISH-ASM-LIVENESS |
| webserver / tech（技术栈） | GOLISH-ASM-FINGERPRINT |
| cdn_waf | GOLISH-ASM-WAFCDN |
| real_ip | GOLISH-ASM-REALIP |
| os_info | GOLISH-ENUM-PORT（nmap -O，conditional） |

> 即：**覆盖维度 = 要做的事**；**固定字段 = 做完落库的结果**。gate 卡「维度盖全 + 证据」，字段写回供后续阶段（vuln_triage）与报告用。

---

## 5. gate 怎么接（复用现有积木，零新引擎）

- 两个 stage spec 的 `expected_techniques` 填本清单的「✅ 必盖」项（conditional/建议项作为 AI 可超出的上限，不强制）。
- 复用 `coverage_complete` op（设计 2026-06-05-coverage-matrix）：每个 in-scope 资产 × 每个必盖维度都要有终态：
  - `found` + evidence（盖到了，挂工具证据 id）
  - `checked_empty` + evidence（查了没有，I8：查空≠没查，要挂探测证据）
  - `blocked` / `not_applicable` + note（如非 CDN 资产的 REALIP=not_applicable）
- in-scope 资产集由 harness 外层从 recon 资产库（`targets.scope='in'`）注入 GateContext（**已接通**，见 2026-06-06 合并 recon 会话）。
- 粒度差异（per-root / per-host / per-site / per-ip）→ coverage 的 asset 维度按维度定义的粒度展开（实现期 §需细化「资产粒度归一」规则，列开放问题 Q2）。
- `min_invocations` 旧检查可保留作冒烟下限，或被 coverage_complete 取代（Q3）。

---

## 6. 工具库怎么配（AI 自由挑，不写死）

- 上表推荐工具登记进 `resources/toolsconfig/*.json`（部分已有：subfinder/amass/httpx/nmap/ffuf/gobuster/katana/nuclei…；缺的如 puredns/dnsx/tlsx/wafw00f/gowitness/arjun/feroxbuster 按 tool-installation 技能补）。
- AI 经 `pentest_run` 调，受各 stage `allowed_tool_types` 白名单（tool_taxonomy）管：测绘维度的工具类型要在 external_attack_surface 白名单、枚举维度的在 enumeration 白名单（实现期校对 `allowed_tool_types` 是否需补类型，Q4）。
- prompt 按维度清单提示 AI：「对每个 in-scope 资产，逐项覆盖这些维度；用合适工具，结果用 update_recon 写回 + 在 coverage 标终态引证 evidence」。

---

## 7. 开放问题（实现前需用户拍板）

1. **必盖项范围**：§2/§3 标 ✅ 的是否就是「地板」？conditional/建议项要不要也强制（会更全但更重/更慢）？
2. **资产粒度归一**：coverage 的「asset」维度，子域枚举是 per-root、端口是 per-host、目录是 per-site——gate 怎么把这些粒度统一成可核的格子？（建议：测绘维度按 root/host、枚举维度按 site，分两套 coverage 分母。）
3. **min_invocations 去留**：coverage_complete 上线后，旧 min_invocations 检查保留作冒烟下限，还是删掉只留 coverage？
4. **allowed_tool_types 补充**：现 external_attack_surface 无 recon/visual 截图工具类是否够？enumeration 是否要加类型？
5. **同事 run_active_collection**：留作「一键全套」可选工具（AI 想批量跑就调），还是这期不接？
6. **分期**：建议 P1a = 测绘维度（GOLISH-ASM-*）+ gate + 字段回写；P1b = 枚举维度（GOLISH-ENUM-*）。还是一次到位？

---

## 8. 下一步

用户对 §2/§3 维度表**增删 + 拍板 §7 开放问题（至少 Q1/Q2/Q6）**后，进入 writing-plans 出实现计划 `docs/superpowers/plans/2026-06-06-active-recon-coverage-p1.md`，再 TDD 落地（taxonomy 登记 → stage spec expected_techniques → update_recon 扩字段 → prompt 分流 → 工具库补齐 → gate 端到端）。本设计只是**维度地基**，不是实现计划。
