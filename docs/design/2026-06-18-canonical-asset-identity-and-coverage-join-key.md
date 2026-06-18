# 2026-06-18 · 规范资产身份（E1）+ 技术结局物化表（E3）：coverage 单一真值源

> 日期：2026-06-18
> 状态：设计（§8 D0–D5 已于 2026-06-18 由用户拍板 → 待写 `docs/superpowers/plans/` 实现计划）
> 作者：BaJie MCP-agent-3（无角色）
> 关联：
> - `docs/design/2026-06-15-db-truth-single-source-deliverable.md`（DB 真值唯一源 + per-technique outcome facts；**本文是其前置依赖 + 物化升级**，见 §2）
> - `docs/design/2026-06-11-coverage-auto-derive-from-evidence.md`（`(asset, technique, outcome)` 投影模型 + audit_log 三列）
> - `docs/design/2026-06-12-redteam-phase0-db-truth-authoritative-gate.md`（`authoritative_found` — found 只认真值）
> - `docs/design/2026-06-15-ip-centric-asset-model.md`（domain↔IP edges / 资产树；与本文相邻但正交，见 §2.3）
> 不变量：AGENTS.md I2（IDOR / org 隔离）、I7（阶段交付必须有 evidence）、I8（「已检查为空」≠「未检查」）、I5（跨 IPC 类型 ts-rs 同步）、I10（schema 向后兼容：先扩 → 再上写 → 再上读 → 再清旧）、§2.5（gate validator 是确定性规则）、§2.7（改 schema 先确认 — **用户已于 2026-06-18 确认**）

---

## 0. 一句话

本文做两件互相咬合的事：

- **E1 · 规范资产身份**：把「一个资产的规范字符串」收敛成**唯一一个确定性函数** `canonical_asset_key(value)`，让落库、in-scope 轴、真值读取、gate join 四处用**同一把钥匙**对齐——根治「身份漂移→fact 静默不命中→格永判 not_attempted→无限 needs_fix（= 反复多次提交）」这一族 bug（CT 死循环、`77.48` 垃圾、URL-IP 误分类都是它的子集）。
- **E3 · 技术结局物化表**（用户拍板做独立表）：新建 `technique_outcomes` 表，作为 coverage gate 的**单一真值源**——每 `(run × asset × technique)` 一行，带 `outcome / evidence_ids / seq`，**用 E1 规范键作 asset**。它把今天「`coverage_truth` 业务表 union + audit_log 投影」两套真值来源合并成一处，并把用户最初「逐维存库、每维存序号、发 id 让 gate 自己查」的直觉**做成一等公民**。

> **E1 与 E3 的关系（关键）**：E1 是 E3 的**前置 substrate**。`technique_outcomes.asset` 必须是规范键，否则 gate join 照样漂移、新表照样填不满。**先 E1、后 E3。**

---

## 1. 问题（已核源码，行号为 2026-06-18 工作树，实现时复核）

### 1.1 现有「资产身份」逻辑有四套，互不知情

| # | 函数 / 位置 | crate | 干什么 | 已为哪类 bug 单独打过补丁 |
|---|---|---|---|---|
| A | `normalized_host` `runner.rs:961` | golish-recon-app | trim / 去尾点 / 小写 / 弃 IP / URL→host / 去 `www.` | —（与 B **逐字节重复**） |
| B | `normalized_host` `persistence.rs:1038` | golish-recon-app | 同 A，**复制粘贴的第二份** | — |
| C | `registrable_domain` `persistence.rs:553` | golish-recon-app | 手搓 PSL：硬编码 `["com","net","org","gov","edu","co","ac"]` 取注册域 | `124.196.77.48`→`77.48` 垃圾（`registrable_domains:571` 加 IP 跳过补丁兜） |
| D | `AssetClass::from_value` + `is_url_wrapped_ip` `technique_resolver.rs:36/67` | golish-agent-kit | gate 侧从 value 推资产类（URL-wrapped IP 特判） | `http://1.2.3.4` 被当 domain（host-aware 补丁） |
| E | gate join 本身 `rule_engine.rs:556-565` | golish-agent-kit | `f.asset == *asset && f.technique == *tech`（**裸字符串相等，零归一**） | —（join 处根本没有归一） |
| F | `coverage_truth` SQL `coverage_truth.rs:43` `IP_TYPE_IN_LIST` | golish-db | SQL 端按 `targets.type` 文本匹配 IP 类 | — |

- **A/B 是同一份代码的两个副本**：改一处忘另一处 = 经典漂移源。
- **C 是个伪 PSL**：`ne.jp` / `com.br` 等真实多级后缀不在硬表里 → 取错注册域；且它被误用在「身份」语境（CT 查询目标）才酿成 `77.48` 事故。它**本不该**当 join 身份用——「注册 apex（给 CT/WHOIS 查谁）」和「资产规范身份（给 coverage 对齐谁）」是**两件事**，现在被混为一谈。
- **D/E 在 gate crate**，**A/B/C 在 recon crate**，**F 在 db crate**：三个 crate 各归一各的，没有任何机制保证「证据落库时写的 asset 串」== 「gate in-scope 轴里的 asset 串」== 「coverage_truth SELECT 回来的 asset 串」。

### 1.2 join 不命中 = 永久 BLOCK（「反复提交」的机器解释）

`authoritative_found` 下（target_intel 已开，见 redteam-phase0），某格 found 终态 **only** 来自 `has_fact(Found)`：

```rust
// rule_engine.rs:560-565（节选）
let has_fact = |want: EvidenceOutcome| {
    ctx.evidence_facts.as_deref().is_some_and(|facts| {
        facts.iter().any(|f| f.asset == *asset && f.technique == *tech && f.outcome == want)
    })
};
```

`f.asset == *asset` 是**精确字符串相等**。证据这边由 `passive_intel_facts_from_command`（`evidence_facts.rs:53`）从命令行抽资产（只做了 `trim_end_matches('.')`，没小写/没去 www/没 URL 解包）；in-scope 轴那边由 `in_scope_assets(org_id)` 读 `targets.value` 原样。二者**各归一各的**：`Pingan.com` vs `pingan.com`、`www.pingan.com` vs `pingan.com`、`http://1.2.3.4` vs `1.2.3.4`、`pingan.com.` vs `pingan.com`——任何一个差异 → fact 静默不计入 → 格永远 `not_attempted` → gate 永远 needs_fix。**这正是 CT/SUBDOMAIN「跑步机」与多次提交的底层机制**，`registrable_domains` 那个 bug 只是这一族里最显眼的一例。

### 1.3 真值来源有两套，靠 union 拼（E3 要解的）

今天 coverage 真值来自两处、形态不同：

- **命令账本**（DNS/WHOIS/SUBDOMAIN）：`audit_log` 三列 `(evidence_asset, evidence_technique, evidence_outcome)`（migration `20260611000001/2`），append-only 证据行。
- **业务表**（ASN/CT/OSINT，无 CLI 工具）：`coverage_truth.rs` 读 `organizations.asns/certificates/whois/intel` 等，**只产 Found、绝不产 empty**（I8 红线）。

两套在 `gate_context` 里按 technique union（`harness_submit_tool.rs:201-243`）。后果：① ASN/CT/OSINT 结构上**无法**有 checked_empty（业务表读不出「跑了→空」）——这正是 CT 在 crt.sh 挂掉时死循环的结构根源；② 真值口径分散、难一致、难 run 隔离。

---

## 2. 与已有设计的关系（先讲清，避免重复造）

### 2.1 E3 物化表 = audit_log 三列之上的「当前态」物化，不是替代

`(asset, technique, outcome)` 的 **append-only 事实**已有承载（audit_log 三列 + `coverage_truth`）。E3 的 `technique_outcomes` **不替代它们**，而是其上的**每 `(run × asset × technique)` 当前态物化视图**：append-only 日志回答「历史上跑过什么」，物化表回答「本 run 这一维现在是什么终态」。gate 读后者（单一、规范键对齐、可 run 隔离），证据可追溯回前者（`evidence_ids`）。

> 这也顺手合并了 §1.3 的两套真值来源：命令路径与 enrich/业务表路径**都写进同一张 `technique_outcomes`**，gate 只读这一张。

### 2.2 与 `db-truth-single-source` 的关系：前置 + 物化升级

`2026-06-15-db-truth-single-source-deliverable.md` 设计了「DB 作唯一真值源、harness 投影 facts 层、agent 只交判断层」（PR1 落库 / PR2 投影 / PR3 提交简化，决策待审，未实现）。本文是它的两块拼图：

- **前置**：它默认「落库 asset == in-scope 轴 asset == coverage_truth asset」，但 §1 证明今天不成立 → E1 补上这个前提。
- **物化升级**：它的 PR1-D2「book `(technique, asset, outcome)` ledger fact」本是写 audit_log 三列；本文把这步**升级为 upsert `technique_outcomes`**（带 run 隔离 + seq + 规范键），其 PR2 投影直接读这张表。二者方向一致，本文给出更干净的落点。

### 2.3 与 `ip-centric-asset-model` 正交

`2026-06-15-ip-centric-asset-model.md` 解决 domain↔IP **edge 关系 + 资产树渲染**；本文解决**单个资产的规范字符串身份**与**coverage 真值物化**。两者相邻不冲突：ip-centric 的 edge 两端节点应当用本文的 `canonical_asset_key` 标识。

---

## 3. 设计

### 3.1 E1 · 唯一的规范身份函数

新增一个**确定性纯函数**（无 IO），所有路径唯一入口：

```rust
/// 资产的规范身份：coverage join / 落库 / 真值读取四处统一用它。
pub struct AssetKey {
    pub key: String,       // 归一后的规范字符串（join 用的那把钥匙）
    pub class: AssetClass, // Domain | Ip | Cidr | Url | Other（复用/迁移自 AssetClass）
}

pub fn canonical_asset_key(value: &str) -> Option<AssetKey>;
```

归一规则（确定、可单测逐条钉死）：

1. `trim` 首尾空白；空 → `None`。
2. ASCII 小写。
3. 若是 `http(s)://` URL：取 **authority 的 host**（去 userinfo、去端口；IPv6 去 `[]`）。URL-wrapped IP（`http://1.2.3.4`）→ host 为 IP（`is_url_wrapped_ip` 逻辑并入本函数）。
4. 去 FQDN 尾点（`pingan.com.` → `pingan.com`）。
5. IP：解析为 `IpAddr` 后用其**规范文本**（消除 `::1` vs `0:0:…`、前导零差异）。CIDR：规范网络形。
6. 分类得 `class`。
7. **故意不做的事**：① **不去 `www.`**——`www.example.com` 与 `example.com` 是**不同资产**，coverage 必须区分（去 www 是 recon 「org 拥有域去重」的关切，**不是**身份归一，保留在 recon 侧）；② **绝不**做 `registrable_domain` 截断——注册 apex 是「查询目标」概念，**不是** join 身份（§1.1 C 的事故根因就是把它当身份）。

> 红线：`canonical_asset_key` 只**归一**、绝不**截断/合并**到 apex。`registrable_domain` 保留为独立函数，仅服务「CT/WHOIS 查哪个 apex」，二者职责分明、不得再混用。

### 3.2 E1 四处接入同一把钥匙

| 接入点 | 现状 | 改为 |
|---|---|---|
| **落库** `technique_outcomes.asset` / `evidence_asset` | 抽出原始资产串 | 抽出后过 `canonical_asset_key().key` 再写 |
| **in-scope 轴** `in_scope_assets(org_id)` | 读 `targets.value` 原样 | 读出后 `canonical_asset_key` 归一 |
| **gate 真值读** | `coverage_truth` 业务表 union | 读 `technique_outcomes`（其 asset 已是规范键） |
| **gate join** `f.asset == asset` | 裸相等 | 两侧都已是规范 key → join 处**无需改逻辑**（核心纯函数 byte-for-byte 不动） |

### 3.3 E3 · `technique_outcomes` 物化表（用户拍板做独立表）

```sql
-- coverage gate 的单一真值源：每 (run × asset × technique) 的当前覆盖态。
-- 命令路径与 enrich/业务表路径都写这里；gate 只读这里；证据可回溯 audit_log。
-- I10 安全：纯新增表，不动既有表/列；audit_log 三列保留为 append-only 事实底座。
CREATE TABLE IF NOT EXISTS technique_outcomes (
    id              BIGSERIAL PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE, -- I2 org 隔离
    run_id          TEXT NOT NULL,         -- session/stage_run 键 = run 隔离（freshness）
    asset           TEXT NOT NULL,         -- canonical_asset_key().key（E1 规范键）
    technique       TEXT NOT NULL,         -- 注册 technique id（GOLISH-INTEL-* 等）
    outcome         TEXT NOT NULL,         -- 'found' | 'empty'（I8：empty 只来自真「跑了→空」）
    evidence_ids    BIGINT[] NOT NULL DEFAULT '{}',  -- 指向 audit_log 真实行（I7 可追溯）
    seq             BIGINT NOT NULL,       -- 本 run 内落库序号（用户「每维存序号」诉求）
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, asset, technique)      -- 每维一行；重跑同维 = upsert，幂等不堆叠
);
CREATE INDEX IF NOT EXISTS idx_technique_outcomes_run ON technique_outcomes (run_id, organization_id);
```

语义：

- **维度 = `technique`**；**逐维一行**；UNIQUE 保证重跑只 upsert（解掉用户看到的「同维反复堆叠/反复提交」）。
- **`asset` 是 E1 规范键**——这就是为什么 E1 必须先行。
- **`outcome` 只 `found`/`empty`**：I8 不破，`empty` 仍只来自真实「跑了→空」的 outcome（含失败 run → checked_empty，承接已修的 `passive_intel_outcome_for_run`）。**绝不**从「缺行」推断 empty——缺行 = `not_attempted` = gate 照旧 BLOCK（fail-closed）。
- **`evidence_ids` 指向 audit_log**：保 I7，gate 可校验存在性、报告可追溯。
- **`seq` + `run_id`**：用户最初「逐维存库、存序号、发 id 让 gate 查」的字面落地；run 隔离让本 run coverage 不被上次 engagement 残留满格（顺解 freshness）。
- gate 侧：新增「从 `technique_outcomes` 投影 `EvidenceFact`」的读路径，**取代** `coverage_truth` 业务表 union；`rule_engine` 判定逻辑不变（仍是 `has_fact(Found/Empty)`，只是 facts 来源换成这张表）。

---

## 4. 红线（任一违反即否决实现）

1. **gate 纯函数不变**：归一/换源只发生在**边界**（落库 / 读 in-scope / 投影 facts），`rule_engine.rs` 判定逻辑 byte-for-byte 不动（用既有 PASS/BLOCK parity 测试守）。
2. **I8 不破**：`technique_outcomes.outcome` 的 `empty` 只来自真实「跑了→空」；缺行 = not_attempted = BLOCK，**绝不**从缺行/缺业务表数据推断 empty。
3. **身份 ≠ 截断**：规范键绝不做 registrable-apex 合并、不剥 `www.`（否则把**不同资产误并成一格**，新漏报风险，比现状更糟）。
4. **I2 IDOR**：`technique_outcomes` 一切读写按 `organization_id` 过滤（含批量），与 coverage 资产盘 org 隔离对齐。
5. **I10 / §2.7（用户已确认建表）**：分步「先建表 → 再上写路径 → 再上读路径（灰度 dual-read）→ 旧 union 留作回退后再清」。任何一步独立可回滚。
6. **I7 不破**：`evidence_ids` 全指向 audit_log 真实行；`enforce_evidence_existence` / fabricated-ref 校验照跑。
7. **I5**：新增跨 IPC 类型（若 `technique_outcomes` 行要给前端看）须 ts-rs；纯 gate 内部用则不跨 IPC（实现复核）。
8. **不削弱判错**：completeness / corroborated / denominator / freshness / fabricated 全保留。

---

## 5. 落地分步（每步独立可回滚、TDD 全绿）

### PR-A · E1 `canonical_asset_key`（纯函数 + 单测，零接入）
- 新函数 + 详尽单测（大小写 / 尾点 / URL 解包 / URL-IP / IPv6 规范 / CIDR / **www 保留** / **apex 不截断** / 空→None）。`is_url_wrapped_ip` 并入；`AssetClass` 复用或迁移（§8 D1）。零行为变化。

### PR-B · E1 边界接入（治本，按风险从低到高）
1. 读 in-scope 轴 `in_scope_assets` 返回前归一。
2. `evidence_asset` / 落库点抽取后归一。
3. 删 `normalized_host` 重复副本（A/B 合一为调规范函数的薄封装；www-去重语义留 recon 侧独立 helper）。
- **验证**：活体 pingan target_intel，因身份漂移产生的 `never attempted (CT/SUBDOMAIN)` 消失。

### PR-C · E3 建表 + 写路径（I10 第 1/2 步）
1. migration `CREATE TABLE technique_outcomes`（§3.3，纯新增）。
2. golish-db 加 repo（upsert / 按 run+org 读，全 org 过滤）。
3. 落库点（命令路径 `evidence_facts` 落点 + enrich/landing 路径 `persistence.rs` 三钩子）在 book 证据时**同步 upsert** `technique_outcomes`（asset 走 E1 规范键，seq 取 run 内自增）。非致命：upsert 失败只 warn，不回滚证据。
- **验证**：活体跑后直查 `technique_outcomes`：pingan 6 维齐、asset 全规范、outcome/evidence_ids/seq 正确；UNIQUE 重跑幂等。

### PR-D · E3 gate 读路径切换（I10 第 3 步，灰度）
1. 新增「从 `technique_outcomes` 投影 `EvidenceFact`」，注入 `GateContext.evidence_facts`。
2. 灰度 dual-read：先与旧 `coverage_truth` union 并行比对（log 差异），parity 稳后 opt-in 切到表（`#[serde(default=false)]` 风格，先只 target_intel）。
- **验证**：弱模型 target_intel 修前 18+ needs_fix → 修后凭「落库 + 极简提交」过 gate；`rule_engine` 既有测试全绿（核心未变）。

### PR-E ·（可选，后续）清旧 union（I10 第 4 步）
- 读路径全切表、活体稳定后，下线 `coverage_truth` 业务表 union 读法（或保留为 `technique_outcomes` 的回填器）。单独排期。

> 顺序：E1（PR-A/B）必须先于 E3（PR-C/D），因为表的 `asset` 依赖规范键。

---

## 6. 影响面

| crate / 文件 | 改动 | PR | 风险 |
|---|---|---|---|
| 新 `canonical_asset_key`（位置见 §8 D1） | 新纯函数 + 单测 | A | 低（纯增量） |
| `golish-agent-kit/.../technique_resolver.rs` | `AssetClass` 复用/迁移、`is_url_wrapped_ip` 并入 | A | 低 |
| `golish-agent-app/.../subtask_phases/execute.rs`、`harness_submit_tool.rs` | in-scope 轴归一 | B | 中 |
| `golish-agent-kit/.../evidence_facts.rs` | 抽取后归一 asset | B | 中 |
| `golish-recon-app/.../persistence.rs`、`runner.rs` | `normalized_host` 去重；`registrable_domain` 职责澄清 | B | 中 |
| `golish-db/migrations/2026XXXX_technique_outcomes.sql` | **新表** | C | 中（schema，I10 分步） |
| `golish-db/src/repo/technique_outcomes.rs`（新） | upsert / org+run 读 repo | C | 中 |
| 命令路径 + enrich/landing 落库点 | 同步 upsert technique_outcomes | C | 中（落点多，逐处对） |
| `golish-agent-kit/.../harness/gate/rule_engine.rs` | **不改判定**；新增 facts 投影源 | D | 中（核心 gate，TDD 全覆盖） |
| `golish-db/src/repo/coverage_truth.rs` | 灰度 dual-read → 后续退役 | D/E | 中 |

**ts-rs/IPC**：`AssetKey` / `technique_outcomes` 行默认后端内部；若前端要展示逐维进度则 derive ts-rs（I5，实现复核）。

---

## 7. 备选方案与取舍

- **采纳 · 独立 `technique_outcomes` 物化表**（用户 2026-06-18 拍板）。优点：单一真值源（合并 §1.3 两套）、run 隔离、`seq` 落地用户诉求、checked_empty 对 6 维全一等公民、upsert 幂等止住堆叠。代价：一张新表 + 写路径要覆盖所有落库点（双写一致性靠「证据为底、表为物化」+ 非致命 upsert 缓解）。
- **备选（未采纳）· 只用 audit_log 三列 + 改投影**。更省（零新表），但 append-only 日志做「当前态 + run 隔离 + 序号」要在读时聚合，查询复杂、难加 seq 语义；且 ASN/CT/OSINT 仍困在业务表 union。**本是上一版推荐，用户选了更干净的物化表，故降为备选。**
- **备选（拆分）· 引第三方 PSL 库**替 `registrable_domain` 伪 PSL。建议但属「apex 查询目标」职责，与本文正交，单列小任务。
- **备选（临时）· 仅 gate join 处归一**。可作 PR-B 完成前的安全网，非治本。

---

## 8. 决策（D0 + D1–D5 已于 2026-06-18 由用户拍板）

- **D0 · 建独立物化表**：✅ 做（`technique_outcomes`，用户：「可以做新表，没问题」）。
- **D1 · `canonical_asset_key` 落 crate**：✅ ① `golish-pentest-domain`——`TargetType` 已住此、`AssetClass` 一并迁入；该 crate 纯逻辑零 I/O（只依赖 serde/serde_json/ts-rs），`golish-db`/`golish-agent-kit`/`golish-recon-app` 依赖它**零循环**（后两者经 `golish-pentest` 已传递依赖，db 直加即可）。
- **D2 · `seq` 语义**：✅ ② 每 run 从 1 自增（贴「第 1 维、第 2 维…」直觉；首插取 `COALESCE(MAX(seq),0)+1 WHERE run_id=$1`，并发以 UNIQUE + upsert 兜底）。
- **D3 · 写路径范围（PR-C）**：✅ 先只 target_intel 6 维 INTEL 落点，验稳再推 EAS/enumeration。
- **D4 · 读切换灰度（PR-D）**：✅ dual-read 期加「新旧真值比对断言」进活体验证（差异即身份漂移，正是要修的）。
- **D5 · 排期**：✅ E1（PR-A/B）先落、E3（PR-C/D）紧随（表 `asset` 依赖规范键）。

---

## 9. 验证计划（实现阶段，TDD + 活体）

- **PR-A 单测**：`canonical_asset_key` 逐条规则（含 www 保留 / apex 不截断 / IPv6 规范 / 空→None）。
- **PR-B parity**：`rule_engine` 既有 PASS/BLOCK 全绿；新增「身份漂移」回归（`Pingan.com.`/`www.x`/`http://1.2.3.4` 归一后 join 命中）。
- **PR-C 单测 + 活体**：repo upsert 幂等 + org 过滤（IDOR）；活体直查 `technique_outcomes` 6 维齐、asset 规范、evidence_ids 真、UNIQUE 不堆叠。
- **PR-D gate 集成 + 活体对照**：投影体过 coverage_complete(authoritative)；缺行仍 BLOCK；弱模型 target_intel 修前 pause → 修后过 gate；dual-read 新旧真值零差异（或差异即身份漂移，正是要修的）。
- **收口**：`just precommit` 全绿；`code-audit` 复核 I2/I7/I8/I10、join 纯函数未改、findings 红线未触。

---

## 10. 给审阅者的一句话

E1 把**散在三个 crate、各自为政的「资产身份」收成一把钥匙**，让 gate 那行 `f.asset == asset` 真正比同一个东西；E3 在这把钥匙之上立一张 `technique_outcomes` 物化表，作为 coverage 的**单一真值源**（合并今天两套 union、带 run 隔离与逐维序号）。两者咬合，关掉「身份漂移→永久 not_attempted→反复多次提交」的机器根因。门槛不变（I2/I7/I8 不破、缺证据照旧 BLOCK），变的只是「身份在边界归一一次、真值物化在一处」。
