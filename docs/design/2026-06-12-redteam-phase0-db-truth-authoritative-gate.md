# Phase 0：DB 真值权威 gate + 引用相关性

> 日期：2026-06-12
> 状态：设计（可执行级，首个实现目标）。总纲见 `2026-06-12-redteam-db-truth-master.md`。实现计划见 `docs/superpowers/plans/2026-06-12-redteam-phase0-db-truth-authoritative-gate.md`。
>
> **状态更新（2026-06-22 · 核当前代码 + git log）**：✅ **已落地**——`authoritative_found` 权威 gate（`found` 只认 DB 真值，模型自报 / 改名 evidence 一律不算）已在 `resources/harness/stages/target_intel/spec.json` + `golish-agent-kit/src/harness/gate/rule_engine.rs`；per-org 权威 stage gate commit `15f88c3a`。
> 不变量：AGENTS.md I7 / I8 / §2.5（gate 语义变更先设计）；gate 纯函数不破。

---

## 1. 问题（一句话）

`coverage_complete` 现在让 **`found` 格靠模型自报过关**（`declared` 分支 + `derive_from_items` 分支），且 found 唯一的额外约束只查 evidence「非空 / 有同 technique 自报 claim」，**从不校验所引 evidence 在账本里的真实 technique**。强模型据此把 dig 的 DNS 输出改名成 `whois_data_observed` 等，12 格全报 found，gate 照过（live run `stage-run-c4422add` 实证）。

## 2. 目标 / 非目标

**目标**：让「`found`」这个终态**只能由真实数据兑现**——账本里有该 (asset, technique) 的 Found 事实，或 DB 业务表里该 (asset, technique) 真有行（两者都经现有 `ctx.evidence_facts` 通道，DB 真值是哨兵 id=0 的 Found 事实）。模型自报 found（无论写在 coverage cell 还是 technique-tagged claim）**不再单独构成 found 终态**。

**非目标**：
- 不动 `checked_empty / blocked / not_applicable` 的自报语义（但 checked_empty 仍须有 Empty 账本事实，I8——这已由 `derive_from_evidence` 的 Empty 分支支持，本期不放松）。
- 不动 findings 链路（永远只出自模型）。
- 不加新落点（那是 Phase 1）；本期只改「怎么判」，不改「数据从哪来」。
- 不动 gate 纯函数原则（DB 查询仍在外层 hook）。

## 3. 现状精确勘查（实读，给出改动锚点）

### 3.1 `coverage_complete`（`golish-agent-kit/src/harness/gate/rule_engine.rs:311`）

逐格判终态的三条来源（任一即终态）：

```rust
let declared = d.coverage.iter().any(|c|
    c.asset == *asset && c.technique == *tech && terminal.contains(&c.status));      // ← (A) 纯自报，found 也吃
let derived = derive_from_items && terminal.contains(&CoverageStatus::Found)
    && (claims/findings 里有 subject==asset && technique==tech);                      // ← (B) 自报 claim 派生 found
let derived_from_evidence = derive_from_evidence
    && ctx.evidence_facts 里有 asset+technique 精确匹配（Found→found / Empty→checked_empty）; // ← (C) 真值
if !declared && !derived && !derived_from_evidence { gaps.push(...) }
```

- (A) `declared`：deepseek 的 12 个 found cell 走这条（status=found 在 terminal 集）。
- (B) `derived`：deepseek 的 12 个 technique-tagged claim 也满足这条。
- (C) `derived_from_evidence`：真值通道，DNS/SUBDOMAIN（账本+DB）真命中；WHOIS/ASN/CT/OSINT 这次**没有**真事实命中（无 whois 命令、enrich 只落 2 条 DB 真值）。

**漏洞 = (A) 和 (B) 让 found 不经真值即终态。**

### 3.2 真值通道（已就绪，本期复用不改）

`execute.rs::fetch_evidence_facts_for_gate`（line 1244）已合并：
- 账本事实：`repo.evidence_facts_for_session(sid)` → `(asset, technique, outcome, evidence_id)`，由 `evidence_facts.rs` 命令派生（dig→DNS、whois→WHOIS、subfinder→SUBDOMAIN）落 `audit_log` 三列。
- DB 真值：`repo.db_truth_facts(org_id, assets)` → `coverage_truth_facts`（ASN/CT/SUBDOMAIN/DNS），经 `db_truth_facts_to_evidence` 转哨兵 id=0 的 Found 事实。

`EvidenceFact { asset, technique, outcome: Found|Empty, evidence_id }`。

## 4. 设计

### 4.1 核心改动：found 终态只认真值（C），废 (A)/(B) 对 found 的兜底

在 `coverage_complete` 里，把每格终态判定按 status 分流：

```
对 (asset × tech)：
  found_terminal      = derived_from_evidence 命中 Found 事实            // 只此一条能产 found
  empty_terminal      = (declared 写了 checked_empty)               // 自报，但 ↓
                        且 derived_from_evidence 命中 Empty 事实         // 必须有「跑了→空」账本事实（I8）
  other_terminal      = declared 写了 blocked / not_applicable（+ note，受 max_other_skips）
  terminal = found_terminal || empty_terminal || other_terminal
  缺 → gap
```

要点：
- **`found` 唯一来源 = `ctx.evidence_facts` 里的 Found 事实**（账本派生 or DB 真值）。模型自报 found cell / technique-tagged claim **不再单独算数**。
- **`checked_empty` 收紧为「自报 + Empty 账本事实双要」**（之前 `declared` 单独就能算 checked_empty；现在必须有真 Empty 事实兜 I8）。这同时实现了用户要的「引用相关性」——你说某格 checked_empty，账本里必须真有该 technique 的 Empty outcome。
- `blocked / not_applicable` 保持自报 + note（判断态，受 `max_other_skips` 上限，本期不动）。

### 4.2 这等价于「引用相关性校验」

旧设计想的「found 格引的 evidence 必须以该 technique 落账」——本设计用更强的方式达成：found 直接由账本/DB 的 (asset, technique, Found) 事实产生，事实的 technique 是落账时由 `evidence_facts.rs`（命令派生）或 `coverage_truth.rs`（业务表列）**确定性赋的**，模型碰不到。于是「引用相关」由构造保证，无需再单独写一条 cross-check 规则。

> 模型自报的 coverage 数组与 technique-tagged claim **降级为参考/UX**：gate 不再读它们判 found。它们仍可用于 `coverage_corroborated`（若保留）等非 found 判定，但不再是 found 的真值源。

### 4.3 开关与灰度（关键，防 brick）

新增**逐技术的「权威 found」开关**，避免 Phase 1 落点没到位时把 target_intel brick：

- 复用现有 `derive_from_evidence` 的双开关风格，新增 `authoritative_found: bool`（`#[serde(default)]`，缺省 false = 旧行为逐字节不变）。
- `authoritative_found=true` 时，found 走 §4.1 收紧逻辑（只认真值）。
- **灰度策略**：target_intel.json 先只对「已有真值源」的技术开（DNS/SUBDOMAIN/ASN/CT）；WHOIS/OSINT 在 Phase 1 落点到位后再纳入。实现上：`authoritative_found` 可配一个 `authoritative_techniques: Option<Vec<String>>`（None=全部期望技术；Some=仅这些技术收紧，其余仍走旧自报）。
- 默认全 off → 现有所有 stage 零回归。

### 4.4 BLOCK reason 仍精确

found 被 §4.1 收紧后产生的 gap，复用现有 `never attempted (asset × tech)` 文案；配合 Phase「诊断式 reflector」（已落，`refiner.rs::build_db_truth_diagnosis`）给出「DB 里这格为空 → 去跑 X 命令」。本期不改 reflector。

## 5. 影响面

| 文件 | 改动 | 风险 |
|---|---|---|
| `golish-agent-kit/.../gate/rule_engine.rs` | `CoverageComplete` 加 `authoritative_found` + 可选 `authoritative_techniques`；`coverage_complete()` found/empty 终态判定改 §4.1 | 中（核心 gate 纯函数，TDD 全覆盖） |
| `resources/harness/stages/target_intel.json` | `coverage_complete` 开 `authoritative_found:true` + `authoritative_techniques:["...DNS","...SUBDOMAIN","...ASN","...CT"]`（灰度，WHOIS/OSINT 待 Phase 1） | 低（配置，灰度） |
| 测试 | rule_engine 新增单测（见 §7） | 低 |

> 不动 hook、不动 coverage_truth、不动 evidence_facts、不动 execute.rs——真值通道已就绪，本期只改「判」。

## 6. 红线对齐

- **I7**：found 只指向真实事实行（账本真 id 或 DB 真行）；哨兵 id=0 仍不进 evidence_refs/claims（沿用现有 `evidence_id>0` 过滤）。
- **I8**：checked_empty 必须有 Empty 账本事实，DB 无数据不推断；found 收紧不影响 empty 的「跑了→空」语义。
- **gate 纯函数**：本期全部改动在 `coverage_complete` 纯函数内，DB 查询仍在 hook，可单测。
- **零回归**：`authoritative_found` 默认 false；缺省所有 stage 行为逐字节不变。

## 7. 验证（DoD）

纯函数单测（`rule_engine.rs` tests 模块，TDD 先红后绿）：
1. `authoritative_found=true`：自报 found cell **无**匹配 Found 事实 → 该格仍 gap（BLOCK）。【回归基线：deepseek 的 WHOIS/OSINT 格】
2. `authoritative_found=true`：自报 found cell **有**匹配 Found 事实（账本或 DB 哨兵）→ found 终态（PASS）。【DNS/SUBDOMAIN】
3. `authoritative_found=true`：technique-tagged claim 但**无** Found 事实 → 不再算 found（与旧 `derive_from_items` 行为分叉，钉死）。
4. `authoritative_found=true` + `authoritative_techniques=["DNS"]`：仅 DNS 收紧，WHOIS 仍走旧自报（灰度精确）。
5. checked_empty 自报但**无** Empty 账本事实 → gap（I8 收紧）。
6. checked_empty 自报 + 有 Empty 事实 → 终态。
7. `authoritative_found=false`（缺省）→ 与旧行为逐字节一致（自报 found 仍算终态）。

集成 / 活体：
- 用 transcript `stage-run-c4422add` 的 deliverable 做回归——开 `authoritative_found` 后 target_intel 应 BLOCK 在 WHOIS/OSINT（+ ASN/CT 若 enrich 未落 DB），DNS/SUBDOMAIN PASS。
- `cargo nextest -p golish-agent-kit`（含新单测）全绿；`clippy -D warnings`；`just precommit` 全绿。

## 8. 风险与缓解

- **brick target_intel**：found 收紧后 OSINT（无真值源）必 BLOCK → 用 `authoritative_techniques` 灰度只收紧 DNS/SUBDOMAIN/ASN/CT；WHOIS/OSINT 待 Phase 1。或 Phase 0+1 同期上（总纲 §6.1）。
- **ASN/CT 依赖 enrich 落库**：若 provider 没 key / 没落 `organizations.asns|certificates`，这两格也会 BLOCK——这是**正确的**（真没采到），交给 reflector 提示「配 provider key / 跑 enrich」。
- **误伤 GUI/chat 路径**：那条路径 org_id=None、`in_scope_assets` 缺失 → 真值通道本就退回纯账本；且 `authoritative_found` 默认 off。双保险零回归。

## 9. 与既有设计的关系

- 直接补齐 `2026-06-12-db-truth-driven-gate-and-diagnostic-reflector.md` §3.1 指出但未闭合的那一层：「coverage_truth 是加性补格，不是权威否决」。本期把它变成权威否决（对开了开关的技术）。
- coverage_truth 的维度扩展（WHOIS/OSINT/主动）在 Phase 1；本期一旦那些维度进了 `ctx.evidence_facts`，§4.1 的 found 判定**自动**覆盖，无需再改 gate。
