# Phase 2：子公司发现进 scoping（企查查 + 投资比例阈值 → 权威 org 树）

> 日期：2026-06-12
> 状态：设计（设计级，待 Phase 0/1 验稳后细化为实现计划）。总纲见 `2026-06-12-redteam-db-truth-master.md`。
> 关联：`2026-06-02-organization-recon-closed-loop.md`（org 闭环）、`2026-06-10-coverage-asset-scope-isolation.md`（org 隔离）。
> 不变量：AGENTS.md I2（资源所有权/范围）、I7、§2.7（schema 改动确认）。

---

## 1. 问题（live run 实证）

用户的红队现实：一次 engagement 给一批公司，规则常含「**投资比例 >X% 的子公司纳入范围**」。正确流程是 scoping 阶段先把 org 树（母 + 合格子）建出来，后续逐个收集。

但当前 live run（deepseek × 默安科技）里：`recon_discover_subsidiaries` 工具**注册了却从未被调用**——deepseek 显式说「subsidiary discovery 是 subsidiaries phase 的事」跳过。根因：**「子公司」不是任何 stage 的 coverage 门槛**，模型没有完整性压力去跑它，于是 org 树永远只有母公司一层，子公司资产整片漏采。

`scoping.json` 现状：`allowed_tool_types: []`、`gate_rules: []`（L0 纯授权确认，无 probing、无门槛）。所以 scoping 现在啥也不强制。

## 2. 目标 / 非目标

**目标**：当 engagement 要求纳入子公司时，scoping 阶段**确定性地**完成「企查查/TYC/KC 子公司发现 → 按投资比例阈值筛 → 母 + 合格子 org 全部落库成权威 org 树」，且 scoping gate 在 DB 真有该 org 树前不放行。

**非目标**：
- 不做多 org 的 coverage 轴本身（那是 Phase 3）——本 Phase 只负责「把 org 树建出来落库」。
- 不改子公司发现工具的内部实现（`recon_discover_subsidiaries` 已存在）；只把它编排进 scoping + 加门槛。
- 不强制所有 engagement 都跑子公司（单域名 bug bounty 不需要）——由 engagement 参数开关。

## 3. 设计

### 3.1 触发：engagement 参数

新增范围参数（CLI flag + GUI scope 配置 + 透传 harness）：
- `include_subsidiaries: bool`（默认 false，保守）。
- `subsidiary_investment_threshold_pct: u8`（默认 50，仅 `include_subsidiaries=true` 时生效）。

来源：`--stage-run` 加 flag（如 `--include-subsidiaries --subsidiary-threshold 50`）；GUI 走 scope 配置。透传链路照搬 `harness_org_id`（`2026-06-10`）的 setter 模式（orchestrator 加字段 + setter + execute.rs 消费）。

### 3.2 子公司发现是「定义范围」，不是「probing」

企查查/TYC/KC 查询是对**工商数据**的 OSINT 查询，**不接触目标主机** → 符合 scoping「no probing」语义（它定义范围，不探测目标）。故把 `recon_discover_subsidiaries` 对应的 tool_type 加入 scoping 的 `allowed_tool_types`（仅当 `include_subsidiaries=true`，或始终允许但靠门槛驱动）。

### 3.3 发现 → 筛 → 落库

`recon_discover_subsidiaries` 流程（编排，非改工具内部）：
1. 对每个种子母公司，查企查查/TYC/KC → 返回子公司列表 + **投资比例**（关键字段，见 §6 验证项）。
2. 按 `subsidiary_investment_threshold_pct` 筛（>=阈值纳入）。
3. 母 + 合格子写入 `organizations`（含 parent-child 关系；`organizations` 已有 org 树支持，见 `2026-06-02` 设计）。每个 org 的根域名进 `targets(scope='in', organization_id=该 org)`。
4. 落 evidence（`recon_discover_subsidiaries` 的产物进账本，technique 可标 `GOLISH-INTEL-SUBSIDIARY` 新登记）。

### 3.4 scoping gate：org 树必须真落库

给 `scoping.json` 在 `include_subsidiaries=true` 时加确定性门槛（DB 真值，沿用 Phase 0 哲学）：
- 新增 gate 检查（外层 hook 查 DB）：`organizations` 里该 engagement 的母 org 存在 + 子公司发现 evidence 存在（区分「跑了→0 个合格子公司」vs「没跑」——I8：前者是 checked_empty 合法终态，后者 BLOCK）。
- 可选：把「子公司发现」做成一个 `GOLISH-INTEL-SUBSIDIARY` 技术维度，纳入 scoping 的 expected_techniques + coverage_truth（org 表投影「该母 org 是否做过子公司发现」）。这样它天然走 Phase 0 的 DB 真值 gate。

> `include_subsidiaries=false` 时 scoping 行为逐字节不变（零回归，gate_rules 仍空）。

## 4. 数据流

```
engagement 参数 (include_subsidiaries=true, threshold=50)
        │
   scoping 阶段
        │ recon_discover_subsidiaries(母公司)  → 企查查/TYC/KC
        │   → [子公司 + 投资比例]  → 按 >=50% 筛
        ▼
   organizations 表：母 org + 合格子 org（parent-child）
   targets：每个 org 根域名 scope='in'
        │
   scoping gate（DB 真值）：org 树真落库 → PASS
        ▼
   target_intel（Phase 3 多 org 轴：母先收 → 子逐个收）
```

## 5. 影响面（设计级，待实现计划细化）

| 文件/区域 | 改动 |
|---|---|
| CLI `--stage-run` 参数 + `stage_run/mod.rs` | 加 include_subsidiaries / threshold flag + 透传 |
| orchestrator + execute.rs | 加 harness 字段 + setter（照 `harness_org_id` 模式） |
| `resources/harness/stages/scoping.json` | 条件 allowed_tool_types + 子公司门槛 gate |
| `recon_discover_subsidiaries` 编排层 | 投资比例筛 + 母/子 org 落库 + evidence 标注 |
| `organizations` 落库 | parent-child 关系（若现有 schema 不足则 migration，§2.7 确认） |
| `technique_taxonomy.json` | 登记 `GOLISH-INTEL-SUBSIDIARY`（若做成技术维度） |
| `coverage_truth.rs` | 可选：org 表投影 subsidiary-discovery-done |

## 6. 风险 / 待验证

- **投资比例字段**：必须确认 enscan/企查查/TYC adapter 的返回里真有「投资比例 / 持股比例」字段可筛——若没有，筛选退化为「全部子公司纳入」或需换数据源。**实现前先核实 adapter 输出**（`golish-intel-providers` 的 enscan/zone mapper）。
- **scoping 不再是纯 L0**：加了工具调用，需确认不破坏 DAG entry 语义（子公司发现是 OSINT 非 probing，风险等级仍 low）。
- **org 树规模**：100 家公司 × 各自子公司可能爆出大量 org/target → 与 Phase 3 的 coverage 轴规模、06-10 的 org 隔离配合，避免分母爆炸。
- **权限/合规**：子公司纳入是授权边界问题（I2）——阈值与纳入清单应可被用户在 scoping 审批（`human_approval.required_before: scope_expansion` 已在）。

## 7. 验证（DoD 雏形）

- 单测：投资比例筛选纯函数（阈值边界）；org 树落库 writer。
- 活体：`--stage-run --include-subsidiaries --subsidiary-threshold 50 --org <有子公司的母公司>` → `organizations` 表出现母+合格子；scoping gate 在 org 树落库后才 PASS；阈值以下的子公司不纳入。
- 零回归：不带 flag 时 scoping 行为逐字节不变。
