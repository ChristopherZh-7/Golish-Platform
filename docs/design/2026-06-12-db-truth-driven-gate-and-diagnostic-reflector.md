# DB 真值驱动的 gate coverage + 诊断式 reflector

> 日期：2026-06-12
> 状态：设计（§7 已由用户 2026-06-12 全部拍板「全按推荐」→ 进入实现，PR1 先行）
>
> **状态更新（2026-06-22 · 核当前代码 + git log）**：🟡 **db-truth gate coverage 部分已落地**——业务表真值投影 `db_truth_facts`（`organizations.asns/.certificates/.whois/.intel` → ASN/CT/WHOIS/OSINT）+ `derive_from_evidence` + `(asset,technique,outcome)` evidence 列均已接。**「诊断式 reflector」部分**由 `docs/design/2026-06-12-unified-refiner.md` 承接（单独跟踪，本轮未逐一核实其落地状态）。
> 关联：`docs/design/2026-06-11-coverage-auto-derive-from-evidence.md`、`docs/design/2026-06-11-substantive-stage-evidence-projection-fallback.md`、`docs/design/2026-06-05-coverage-matrix.md`、`docs/design/2026-06-05-vuln-triage-technique-matrix.md`
> 不变量：AGENTS.md I7（阶段交付必须有 evidence）、I8（「已检查为空」≠「未检查」）、I10（schema 先扩可空字段再上代码）、§2.5（安全语义变更）、§2.7（改 schema 先确认）

---

## 1. 问题（live run 盖棺实证，2026-06-12 mimo-v2.5-pro × moresec.cn）

补跑了一场 `--stage-run -p xiaomi -m mimo-v2.5-pro --to target_intel --org 默安科技 --target moresec.cn`，验证 `2026-06-11-substantive-stage-evidence-projection-fallback` 那次改动。结果：

```
[PASS]  scoping       (findings=0)
[BLOCK] target_intel  (findings=1)   ← 3 次 attempt 耗尽
```

三连 BLOCK 链：
1. attempt 2/3 `coverage_complete`：`*.moresec.cn × {DNS,WHOIS,ASN,CT,OSINT}` **never attempted**（只测了 SUBDOMAIN）。
2. attempt 3/3：`submit_only=true` 锁触发（弱模型 submit-only 通道生效）。
3. final `coverage_corroborated`：found 格**没有 technique-tagged claim/finding 佐证**（模型为过关硬把格标 found 却无真证据）。

两个独立结论：

- **投影兜底（`synthesize_from_evidence_when_missing`）没触发**。它的触发条件是「agent **完全没交** deliverable」，而 MiMo **交了**（只是劣质）→ 走正常 gate BLOCK + repair。**投影兜底解决「漏交」，解决不了「没干全」。**
- **真问题是「gate 验证的锚点」**：用户诉求原话——「想看的是数据库这些字段到底是不是真的有，不是工具调了几次 / 几类」。而当前 gate 锚的根本不是业务数据表（见 §3）。

---

## 2. 目标 / 非目标

**目标**：把 stage gate 的 coverage 判定，从「锚 agent 自报 + 命令派生」升级为「**以数据库业务表的真实结构化数据为准**」——DB 里某 (资产 × 技术) 真有数据才算 `Found`，agent 说什么不算数。配套把 BLOCK 时的 `reflector` 纠正从「机械念缺口」升级为「**诊断式逐步指导**（看 DB 缺口 → 给具体下一步命令）」。

**非目标**：
- 不动 active 阶段（EAS / enumeration / vuln_triage）语义，先 `target_intel` 单阶段灰度。
- 不破 I8——`checked_empty` 仍只能由「跑了→空」的真实信号显式产生，绝不靠「DB 无数据」推断。
- 不破 gate 纯函数原则——DB 查询在**外层 hook** 做，结果经 `GateContext` 注入；`validate_stage_gate*` 仍 DB-free。

---

## 3. 现状勘查（动手前先读，已核对源码）

### 3.1 gate 当前锚的是「命令」，不是「业务表」

链路：`evidence_facts.rs::passive_intel_facts_from_command` 从**命令行字符串**解析 `(technique, asset)` + 从**输出文本**判 `found/empty` → 写 `audit_log` 的 `evidence_technique/evidence_asset/evidence_outcome` 三列（PR2）→ gate 的 `coverage_complete.derive_from_evidence` 经 `GateContext.evidence_facts` 注入后投影 coverage（PR3）。

即 gate 认的是「**agent 跑了 `dig` 命令且输出非空**」，**不是**「`target_assets`/`organizations` 表里真有这条数据」。这与用户要的「看 DB 字段真不真」差一层。

### 3.2 DB 落点覆盖表（6 类被动情报）

| 类 | 业务结果表落点 | 命令派生(`evidence_facts.rs`) | gate 现在认吗 |
|---|---|---|---|
| SUBDOMAIN | ✅ `target_assets`(asset_type=subdomain) 专表 | ✅ subfinder | ✅ |
| DNS | ⚠️ 仅 `targets.real_ip`(A 记录)；MX/NS/TXT/CNAME 无表 | ✅ dig/host/nslookup | ✅ |
| WHOIS | ⚠️ 埋 `organizations.intel` JSONB catch-all，无专列 | ✅ whois | ✅ |
| ASN | ✅ `organizations.asns` JSONB 专列 | ❌ 无映射 | ❌ |
| CT | ✅ `organizations.certificates` JSONB 专列 | ❌ 无映射 | ❌ |
| OSINT | ⚠️ 散在 `organizations.{intel,contacts,social_accounts,historical_vulns}` | ❌ 无映射（enscan/0.zone 在 `tool_taxonomy`=recon/osint，但 `evidence_facts` 未映射） | ❌ |

业务表清单：`targets`(host + http/IP 指纹: real_ip/cdn_waf/http_title/http_status/webserver/...)、`target_assets`(subdomain/ip/service)、`organizations`(18 字段甲方情报库: domains/asns/certificates/ip_ranges/email_domains/contacts/intel JSONB...)、`directory_entries`、`api_endpoints`、`audit_log`(evidence ledger + 三列)。

`output_store` db_action：`target_add`→targets/target_assets；`target_update_recon`→targets 指纹列；`directory_entry_add`→directory_entries；`finding_add`→findings；`organization_update`→organizations.*（domain/asn/cert/cidr/email 有专列路由，其余进 `intel.records[]` catch-all）。

### 3.3 三个断层

1. **gate 没查业务表**：coverage 真值来自 audit_log 命令派生，不是 `target_assets`/`organizations`。
2. **命令派生只覆盖 3/6 类**：`evidence_facts` 只认 dig/whois/subfinder → **ASN/CT/OSINT 永远 not_attempted**（今天 BLOCK 的深层结构原因）。
3. **3 类落点不规整**：DNS 无完整记录表（只 real_ip）、WHOIS/OSINT 埋 JSONB catch-all，无法精确查询。

### 3.4 reflector 现状

BLOCK 时 `execute.rs` 把 gate 的 `repair_correction`（规则模板生成的「缺 X 格」文本）`feeding repair correction back to reflector`。机械、不教模型怎么补，也不基于 DB 现状。

---

## 4. 完整性约束（核心红线，任一不满足则该格不投影 / fail-closed）

1. **保 I7**：投影的 `Found` 必须指向真实存在的数据（业务表行或真 evidence 行），fabricated-ref 校验照跑。
2. **保 I8（红线）**：`checked_empty` 永不靠「DB 无数据」推断——无数据 = 「没测」**或**「测了空」无法区分。`checked_empty` 仍只能由「跑了→空」的真实 outcome 信号显式产生。DB 投影**只产 `Found`**。
3. **findings 永不投影**（沿用 `synthesize_from_evidence` 红线）：DB 驱动只补 coverage/claims 的事实部分，findings 永远由模型/真实漏洞证据产生。
4. **gate 纯函数不变**：DB 查询在外层 hook 做，经 `GateContext` 注入；`validate_stage_gate*` 保持 DB-free、可单测。
5. **三 gate 不动**：completeness/corroborated/denominator 跑同一套；DB 投影只是把「DB 有真数据的格」加进并集，**补不出完整性**——缺数据的 (资产×技术) 仍 BLOCK。

---

## 5. 设计

按工作量从小到大、每步独立可回滚。

### 5.1 断层②修复：让 ASN/CT/OSINT 能被标注（命令路径 + provider 路径）

> 实现前核实（2026-06-12）：`passive_intel_facts_from_command` 只在 **3 个命令路径**调用点工作（`direct/mod.rs` ×2、`bridge_config.rs` ×1，全是 shell 命令 / `pentest_run "{tool} {args}"`）。它**够不到 provider 路径**——OSINT（0.zone/Quake/enscan）经 `recon_enrich_assets` → asset_intel runtime 跑，不是 shell 命令，命令映射加了也不会触发。故本步必须**分两块**：

**(a) 命令路径**（改 `evidence_facts.rs`）：补有 shell 命令形式的类。ASN（`whois` 的 ASN 查询变体）可补；CT 需谨慎——`subfinder -s crtsh` 本质产出仍是子域（归 SUBDOMAIN 更准），无独立 CT 命令工具则此处不补。歧义即不映射（沿用 `_ => None`）。

**(b) provider 路径**（改 asset_intel / `recon_enrich` 落 evidence 点）：OSINT 及经 enrich 得到的 ASN/CT，要在 **provider 落 evidence 时**按 `(provider, query_type)` 打 `GOLISH-INTEL-*` 标注（写 `audit_log` 三列），而非命令解析。这是 PR2 当年只覆盖命令路径、漏掉 provider 路径的补齐。

**收益**：3 类不再永远 not_attempted。**风险**：(a) 低（纯解析+单测）；(b) 中（provider 落账多源，需 `(provider,query_type)→technique` 映射表，逐源对齐）。

### 5.2 断层③修复：补结构化落点

给缺专列的类补可精确查询的落点（按 I10 先扩可空字段）：
- DNS：新增 `dns_records` 表（target_id, record_type, name, value, ...）或 `target_assets` 扩 asset_type=dns_record。承接 dig/dnsx 输出（需 `output_store` 加 db_action + toolsconfig `output` 规则）。
- WHOIS / OSINT：从 `organizations.intel` catch-all 提升为专列（whois 注册信息、osint 暴露项），或定义可稳定查询的 JSONB 路径。

**风险**：中（migration + 落库链路多处对齐），按 §2.7 先与用户确认 schema。

### 5.3 断层①修复（治本）：gate 增 `derive_from_database`

`coverage_complete` 再加一条投影来源（沿用 `derive_from_items` / `derive_from_evidence` 的加性、`#[serde(default false)]` 兼容风格）：

```
{ "op": "coverage_complete", "derive_from_items": true, "derive_from_evidence": true, "derive_from_database": true, "on_fail": {...} }
```

外层 stage-close hook 查业务表（`target_assets`/`organizations.asns`/`.certificates`/`dns_records`/...），把「DB 里 (资产 × 技术) 真有数据」的事实经 `GateContext.db_facts: Option<Vec<DbFact{asset, technique, ref}>>` 注入；gate 纯函数据此投影 `Found`。无注入 → `None` → 逐字节回退旧行为。

**这是单一事实源的最终态**：coverage = DB 业务表的只读投影，模型自报/命令派生退化为辅助。彻底废掉「硬填 coverage 格蒙混」（gate 只信 DB）。

> **实现取舍（2026-06-12 核实后，更优雅）**：业务表投影出的事实与 evidence 投影出的事实**同形**（asset × technique × outcome，见 `rule_engine.rs::EvidenceFact`）。故**首选复用现有 `evidence_facts` 通道 + `derive_from_evidence` 开关**——外层 hook 除了从 `audit_log` 派生 facts，再**从业务表（`organizations.asns/.certificates`、`target_assets`、`dns_records`）派生 facts**，合并后一起注入 `ctx.evidence_facts`。这样 **gate 纯函数层（`coverage_complete`）与 `target_intel.json` 零改动**，全部工作落在 ① `execute.rs` stage-close hook 多查业务表、② `golish-db` 加「按 org/target 查某 technique 是否有数据」的只读查询。`db_facts` 平行通道（§5.3 开头那种 `derive_from_database`）仅在需要显式区分「证据来源 vs DB 来源」时才引入。
> **此步是 IO/集成**（查真实 DB），非纯函数——需集成测试或 DB mock，建议在**聚焦会话**实现以保 TDD 全覆盖 + `just precommit` 全绿（当前会话上下文已大，核心 gate 链路改动不宜仓促）。
>
> **核实修正（影响 §9 顺序）**：PR1「命令路径补全」对 ASN/CT/OSINT **价值有限**——OSINT 走 provider 非命令、ASN 的 `whois AS…` 资产维度（AS 号）与 coverage 的域名资产维度对不齐、CT 无独立命令工具（`subfinder -s crtsh` 本质仍是 SUBDOMAIN）。**结论：真正命中「看 DB 字段真不真」诉求的是本节（DB 业务表投影），建议作为首个实现目标，而非命令映射。**

### 5.4 诊断式 reflector（配套，对应用户诉求1）

BLOCK 时不只回灌 gate 模板文本，而是给 reflector 喂三样：①**DB 现状**（该 run 已落哪些字段 / 哪些为空）②gate 缺口 ③模型最近 N 步在重复什么错 → 让 reflector 生成**具体下一步指令**（如「DB 里 DNS 记录为空，对 moresec.cn 跑 `dig moresec.cn ANY`」）。

- reflector 模型由**前端/settings 配置**（用户可为它选强模型；它若也用 MiMo，诊断质量同样受限）——已确认走配置，不在 harness 写死。
- 与 §5.1-5.3 合一 = **DB 真值驱动闭环**：gate 查 DB 判缺 → reflector 看 DB 缺口给具体命令 → agent 照做 → 输出落 DB → gate 重查 → DB 真有了才 PASS。

---

## 6. 影响面

| crate / 文件 | 改动 | 风险 |
|---|---|---|
| `golish-agent-kit` `evidence_facts.rs` | §5.1 补 ASN/CT/OSINT 命令映射 | 低（加性纯解析 + 单测） |
| `golish-db/migrations` + `output_store` | §5.2 DNS/WHOIS/OSINT 结构化落点（migration + db_action + toolsconfig output） | 中（schema + 落库链路，§2.7 先确认） |
| `golish-agent-kit` `GateContext` + `coverage_complete`(`rule_engine.rs`) | §5.3 增 `db_facts` 注入 + `derive_from_database` 分支（纯函数） | 中（核心 gate，TDD 全覆盖） |
| `golish-agent-kit` `execute.rs` stage-close hook | §5.3 查业务表注入 db_facts；§5.4 诊断式 reflector 上下文组装 | 中 |
| `golish-db/repo` | 新增「按 org/target 取各 technique 是否有数据」只读查询 | 低（只读 SELECT） |
| `resources/harness/stages/target_intel.json` | 开 `derive_from_database:true`（单阶段灰度） | 低 |

---

## 7. 决策（用户 2026-06-12 全部拍板：全按推荐）

- **D-范围** ✅：先 `target_intel` 单阶段灰度，验稳再推 EAS/enumeration。
- **D-顺序** ✅：§5.1 先行（小、不依赖 migration）→ §5.4 诊断式 reflector → §5.2+§5.3（DB 真值锚正式态，含 schema 改动）。
- **D-schema** ✅：DNS 新建 `dns_records` 专表（结构化强、量大）；WHOIS/OSINT 先复用 `organizations` 现成字段（沿用 `asns`/`certificates` 专列模式）。
- **D-reflector 模型**：✅ 用户已定（2026-06-12）——reflector 用什么模型由**前端/settings 配置**，不在 harness 写死；用户可按需为它选强模型。

---

## 8. 风险与缓解

- **DB 投影掩盖「没测全」**：只产 `Found` 且只在 DB 真有数据时；completeness gate 不动 → 缺数据仍 BLOCK；`checked_empty` 永不自动造（I8）。
- **命令映射误判技术类**：保守映射，歧义即弃；`coverage_corroborated` 双重兜底（found 必须有对齐 claim/finding）。
- **schema 改动回滚**：按 I10 先扩可空字段 → 再上写入代码 → 再上读取投影；灰度开关 `derive_from_database` 默认 false，一行回退。
- **弱模型能力下限**：诊断式 reflector 提升跑全率，但救不了吐空内容的模型；必要时该 stage 换强模型（今天 live run 已证 MiMo 在 target_intel 的局限）。

---

## 9. 分阶段路线（PR 拆分，每 PR 独立可回滚、TDD 全绿）

- **PR1 = §5.1 technique 标注补全（两块）**：(a) `evidence_facts` 补有命令形式的类（ASN whois 变体；CT 谨慎）+ 单测；(b) provider 落账点（`recon_enrich`/asset_intel）按 `(provider,query_type)` 打 OSINT/ASN/CT 标注。不依赖 migration，最先落。
- **PR2 = §5.4 诊断式 reflector**：BLOCK 时组装 DB 现状 + 缺口 + 近期行为喂 reflector；可配强模型。
- **PR3 = §5.2 结构化落点**：migration（DNS/WHOIS/OSINT）+ output_store 落库 + toolsconfig output 规则。
- **PR4 = §5.3 derive_from_database**：`GateContext.db_facts` 注入 + `coverage_complete` 投影分支 + `target_intel.json` 灰度开 + 弱模型活体对照（修前 BLOCK→修后 DB 有真数据则 PASS）。

---

## 10. 验证计划（实现阶段，TDD）

- 纯函数单测：`derive_from_database` 仅产 Found；DB 有数据的格投影、无的仍 BLOCK；`checked_empty` 不自动产生；findings 永空；无 db_facts 注入逐字节兼容。
- 命令映射单测：ASN/CT/OSINT 各典型命令映射正确；歧义返 None。
- 活体：弱模型 + `target_intel` × moresec.cn，对照本次（3 attempt BLOCK）→ 改后应能在 DB 真有数据时过 gate；证据落 `agent-progress.md`。
