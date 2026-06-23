# #6 · expansion_queue（证明「新线索有没有继续追」）

> 评审 claim #6。消费模型 = **A 审计/reviewer-only**（用户 2026-06-23 拍板：reviewer/报告读 pending 线索，coverage gate 不 block）。状态：进行中（2026-06-23）。**本轮范围 = 设计 + migration(inert) + repo enqueue + 入队写点 + reviewer 读（run_tree.py）。入队写路径无灰度开关、始终开启**（用户 2026-06-23 决定测试阶段默认写——非致命 warn + 消费模型 A（gate 不读）→ 始终写零 gate 行为影响，写失败/表未 apply 只 warn）。

## 1. 问题（评审 #6）

被动收集的「完整性」不只看 `asset × technique` 覆盖，还取决于「**发现的新线索有没有继续追**」。recon 跑出的子公司（`agent_intel.rs::SubsidiaryCandidate`）、新域名、github org、email 域等线索，目前**没有任何 pending/lead 队列**记录——发现即散，gate 证不了「发现了子公司却没递归深挖」。

→ 需要一层 `expansion_queue`：登记每条发现的待扩展线索 + 处理态。

## 2. 消费模型（用户拍板 A）

| 方案 | 内容 | 取舍 |
|---|---|---|
| **A 审计/reviewer-only（采纳）** | 入队发现的线索；reviewer/run_tree.py 报「高置信 pending 线索」；**gate 不 block** | 低风险、与 #4/#5 同哲学（先低风险加表+写+reviewer 读，gate 强制留后续 opt-in） |
| B gate 强制 | 「高置信 pending 线索未处理 → 不能 complete」真 BLOCK | 高风险（改 gate completion 行为、翻活体 run、须定置信阈值）；future opt-in |

`status` / `processed_at` 列已为 B 预留；本轮不接 gate。

## 3. Schema

见 migration `20260623000004_expansion_queue.sql`。

| 列 | 说明 |
|---|---|
| id / organization_id(FK CASCADE) / run_id | PK + I2 org 隔离 + run 隔离 |
| `lead_type` | new_domain / brand / app / github_org / subsidiary / email_domain |
| `lead_value` | 线索值（公司名 / 域名 …；**不**过 canonical_asset_key——子公司是公司名非主机） |
| `source` | 发现处（recon_discover_subsidiaries / enrich provider …） |
| `confidence` | 0..1（NULL=未知） |
| `status` | pending（缺省）/ processed / skipped / blocked（**B 预留**） |
| `evidence_ids` | `BIGINT[]` 指 audit_log（I7） |
| `detail` / `discovered_at` / `processed_at`(B 预留) / created_at / updated_at | 元数据 |

`UNIQUE(run_id, lead_type, lead_value)`：每 (run,类型,值) 一条，重复发现 upsert 幂等不堆叠；**冲突不重置 status**（已处理线索重复发现不退回 pending）。

## 4. 写 / 读集成（本轮实现）

- **写（enqueue，无灰度开关 · 始终开启）**：`recon_discover_subsidiaries` 落库点（`direct/mod.rs`）——纯函数 `expansion_leads_from_subsidiary_discovery(v)` 从 recon JSON 的 `subsidiaries[]` 候选抽线索（lead_type=subsidiary，`meets_threshold`→高/中置信），逐条 `enqueue_expansion_lead`。**非致命** warn（写失败 / 表未 apply 只 warn，绝不影响主流程）。用户 2026-06-23 决定测试阶段始终写（**无 `GOLISH_EXPANSION_QUEUE_WRITE` 开关**）；消费模型 A（gate 不读）下始终写也零 gate 行为影响、便于活体观察。其余线索源（new_domain/github_org/email_domain，来自 OSINT enrich）= 同模式后续可加。
- **读（reviewer，run_tree.py §8）**：`run_tree.py --db` 投影本 run 的 expansion_queue（按 lead_type/status 计数 + 标记高置信 pending）。**coverage gate 不读本表**（消费模型 A）。

## 5. 不变量

- **I2 IDOR**：`organization_id NOT NULL` + FK CASCADE；读写按 org。
- **I7**：`evidence_ids` 全指 audit_log 真实行。
- **I8**：线索是「发现的待办」，与 coverage outcome 正交；本表**不参与** coverage 判定（不会把 pending 线索误当 checked_empty/found）。
- **I10 / §2.7**：分步——migration(inert) → 入队写(gray-switch) → reviewer 读。每步独立可回滚；表 inert = 零行为变化。
- **I5**：后端内部（审计/reviewer 用），不跨 IPC；前端要展示线索队列再 derive ts-rs。

## 6. 验证（本轮）

- migration SQL sanity（CREATE TABLE IF NOT EXISTS + UNIQUE + 2 索引 + 注释，纯新增）。
- repo SQL-builder 单测 + `expansion_leads_from_subsidiary_discovery` 纯函数单测 + flag 单测。
- `cargo nextest` / `cargo clippy -D warnings`（golish-db / agent-kit / agent-app / agent-runtime）+ `py_compile run_tree.py`。
- 默认 off ⇒ 既有测试零回归、表 inert。

## 7. 回滚

- 表未接读（gate 不读）→ 删 migration（未 apply）或 `DROP TABLE expansion_queue`（已 apply，无数据依赖）即回滚；写路径 env 关即停。

## 8. future B（gate 强制，另立设计）

gate completion 增「高置信 pending 线索未处理则 BLOCK」：需 ① 置信阈值 ② 处理态判定（lead_value 是否已成 in-scope org/asset，或 processed_at 非空）③ gray-switch + gate 单测 ④ 防活体 run 误翻。本表 status/processed_at 已为此预留。
