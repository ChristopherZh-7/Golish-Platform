# Phase 1：补落点（WHOIS/OSINT 可查列 + 主动阶段落库 + coverage_truth 扩维度）

> 日期：2026-06-12
> 状态：设计（可执行级雏形，待 Phase 0 验稳后细化为实现计划）。总纲见 `2026-06-12-redteam-db-truth-master.md`。
> 复用既有设计：**主动阶段落库 + coverage_truth 扩主动维度已在 `2026-06-12-active-collection-db-truth-closure.md` 完整设计（P0-A/B + P1-C），本 Phase 直接采纳那份，不重复**。本文档只补它没覆盖的「WHOIS/OSINT 被动落点」+ 统一收口。
> 不变量：AGENTS.md I7 / I8 / I10（schema 先扩可空字段）/ §2.7（改 schema 先确认）。

---

## 1. 为什么（一句话）

Phase 0 把 `found` 收紧为「DB/账本真值」后，**没有真值落点的技术会永远 BLOCK**：OSINT（无任何真值源）、WHOIS（仅当模型真跑 `whois` 命令才有账本事实，且数据只进 `organizations.intel` catch-all 无专列）、以及全部主动技术（endpoint_add/host_add 孤儿 → 工具产物 0 落库）。Phase 1 把这些落点补齐，让 Phase 0 的权威 gate 有真数据可读、让被要求的技术「可被合法满足」。

## 2. 范围

| 子项 | 来源 | 状态 |
|---|---|---|
| **1-A 主动落库** `endpoint_add`(katana/ffuf/gobuster/arjun/gau/waybackurls) + `host_add`(masscan) | `2026-06-12-active-collection-db-truth-closure.md` §3.1/3.2（P0-A/B） | 已设计，**直接实现** |
| **1-B coverage_truth 扩主动 6 维** EAS-PORT/SERVICE/LIVENESS + ENUM-DIR/PARAM/JSAPI | 同上 §3.3（P1-C） | 已设计，**直接实现** |
| **1-C WHOIS 落点**（本文档新增） | 本文档 §3 | 新设计 |
| **1-D OSINT 落点**（本文档新增） | 本文档 §4 | 新设计 |
| **1-E coverage_truth 扩 WHOIS/OSINT 维度 + Phase 0 灰度纳入这两类** | 本文档 §5 | 新设计 |

> 1-A/1-B 的细节、改动文件清单、数据流图、红线、DoD **以 `2026-06-12-active-collection-db-truth-closure.md` 为准**，本文档不复制。下面只展开 1-C/1-D/1-E。

## 3. 1-C：WHOIS 落点

### 3.1 现状

`whois` 命令派生进 `audit_log` 三列（`evidence_facts.rs` 已认 `whois`→WHOIS），所以**账本层**有 WHOIS 事实——前提是模型真跑了 `whois`。但**业务表层**无 WHOIS 专列：whois 注册信息（注册商、注册/到期时间、注册人、name servers）目前若落库只进 `organizations.intel` JSONB catch-all（`organization_update` 的兜底路由），`coverage_truth` 不查它。

### 3.2 设计

按 AGENTS.md I10「先扩可空字段」，给 `organizations` 加 WHOIS 专列（沿用 `asns`/`certificates` 的 JSONB 专列模式）：

- migration：`organizations` 加 `whois JSONB` 可空列（`IF NOT EXISTS`，默认 NULL）。结构如 `{ registrar, created, expires, registrant, name_servers: [...], raw_ref }`。
- `output_store/organizations.rs`：`organization_update` 的字段路由加 `whois` 专列分支（whois 数据从 catch-all 提升为专列，类比现有 domain/asn/cert/cidr/email 专列路由）。
- 数据来源：① `whois` 命令输出解析（toolsconfig `whois.json` 的 output 规则补 db_action+字段映射，若当前没有则新增）；② provider enrich 若返回 whois 字段，经 enrich 落账点写专列。

### 3.3 取舍

whois 命令派生的**账本事实**已能满足 Phase 0 的 found（账本通道）。WHOIS 专列主要价值是：① 让 `coverage_truth` 也能 DB 投影 WHOIS（双锚，与 DNS/SUBDOMAIN 对齐）；② 结构化存储供后续阶段/报告复用。**若只想最快让 WHOIS 可满足，账本通道已够**（模型跑 whois 即可）；专列是「更完整」的一步。实现时可先靠账本通道、专列作为 1-C 的增量。

## 4. 1-D：OSINT 落点（最弱的一类，重点）

### 4.1 现状

OSINT 走 **provider 路径**（enscan-go / 0.zone / quake 等经 `recon_enrich_assets` → asset_intel runtime），**不是 shell 命令** → `evidence_facts.rs` 命令派生够不到；且 enrich 数据散落 `organizations.{intel,contacts,social_accounts,historical_vulns}` JSONB，`coverage_truth` 一个都不查 → **OSINT 既无账本派生、也无 DB 投影 → Phase 0 后永远 BLOCK**。这是 live run 里 deepseek 拿 dig 输出冒充 OSINT 的根因（无真路径，只能伪造）。

### 4.2 设计（两块）

**(a) provider 落账打 technique 标注**（对齐 `db-truth-driven-gate` 设计 §5.1(b)，当年只覆盖命令路径、漏了 provider 路径）：
- 在 asset_intel / `recon_enrich` 的 **provider 落 evidence 点**，按 `(provider, query_type) → technique` 映射表打 `GOLISH-INTEL-OSINT`（或 ASN/CT，见下）标注，写 `audit_log` 三列。这样 OSINT 经 provider 跑了就有**账本事实**，Phase 0 的 found 账本通道即可满足。
- 映射示例：`enscan-go-enrichment / quake(org/icp_unit) / 0.zone(contacts/site)` → OSINT；`quake/0.zone 的 asn 维度` → ASN；`cert 维度` → CT。歧义即不映射（沿用保守 `_ => None`）。

**(b) OSINT 可查列**（DB 投影，增量）：
- 给 `organizations` 的 OSINT 暴露项定义一个可稳定查询的结构（如 `intel.osint_records[]` 的稳定 JSONB 路径，或新增 `osint JSONB` 专列），供 `coverage_truth` 查询「该 org 是否有 OSINT 数据」。
- 优先做 (a)（provider 账本标注），它直接解 Phase 0 的 found 可满足性；(b) 是双锚增量。

## 5. 1-E：coverage_truth 扩维度 + Phase 0 灰度纳入

- `coverage_truth.rs`：在现有 4 维（ASN/CT/SUBDOMAIN/DNS）基础上，按 `active-collection-db-truth-closure` §3.3 加主动 6 维；按本文档 §3/§4 加 WHOIS/OSINT 维度（查 `organizations.whois` / OSINT 可查路径）。
- 一旦这些维度进了 `coverage_truth_facts` → `ctx.evidence_facts`，**Phase 0 的 `coverage_complete` found 判定自动覆盖，无需再改 gate**（这是 Phase 0 §4.3 的设计红利）。
- 把 `target_intel.json` 的 `authoritative_techniques` 从 4 类扩到全 6 类（WHOIS/OSINT 纳入）；EAS/enumeration.json 开 `authoritative_found` + 对应主动技术。

## 6. 影响面

| 文件 | 改动 | 来源 |
|---|---|---|
| `resources/toolsconfig/{ffuf,gobuster,masscan,katana,gau,waybackurls,arjun}.json` | db_action 修正/接新 writer | active-collection-closure |
| `output_store/{endpoints.rs(新),mod,store_trait,pg_adapter}.rs` | endpoint_add 落库 | active-collection-closure |
| `golish-db/migrations/*` | api_endpoints UNIQUE 索引 + `organizations.whois` 列(+可选 osint 列) | 本 Phase + closure |
| `output_store/organizations.rs` | whois/osint 专列路由 | 本 Phase 1-C/1-D(b) |
| asset_intel / `recon_enrich` 落账点 | provider→technique 标注（OSINT/ASN/CT） | 本 Phase 1-D(a) |
| `golish-db/repo/coverage_truth.rs` | 扩主动 6 维 + WHOIS/OSINT 维 + 单测 | closure + 本 Phase 1-E |
| `resources/harness/stages/{target_intel,external_attack_surface,enumeration}.json` | authoritative_techniques 扩到全维 | 本 Phase 1-E |

## 7. 红线 / 验证 / 风险

- 红线同总纲 §8（findings 永空、I8、gate 纯函数、I10 migration 兼容、§2.7 schema 先确认）。
- 验证：coverage_truth 各新维度 assemble 单测；endpoint writer + output parser 单测；活体跑 `--to enumeration` 看 `api_endpoints/directory_entries` count>0、`merged DB business-table truth facts` 主动维度>0、`Unknown db_action` 归零。
- 风险：schema 改动按 I10 先扩可空列 → 上写入 → 上读取投影，灰度开关默认 off；provider→technique 映射多源需逐源对齐（中风险，保守映射歧义即弃）。

## 8. 与 Phase 0 的衔接

Phase 0 上线时 `authoritative_techniques` 只含 DNS/SUBDOMAIN/ASN/CT（有真值源的）。Phase 1 每补齐一类落点，就把该类加进 `authoritative_techniques`。**两 Phase 对 target_intel 强烈建议同期上线**（否则 OSINT 不可满足，见总纲 §6.1）——即 Phase 1 的 1-D(a)（OSINT provider 账本标注）应与 Phase 0 一起进 target_intel 灰度。
