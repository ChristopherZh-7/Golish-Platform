# #5 · source_query_log 层（证明「哪些数据源查过」）

> 评审 claim #5。消费模型 = **A 审计/provenance-only**（用户 2026-06-23 拍板：reviewer/报告读，coverage gate 不 block）。状态：进行中（2026-06-23）。**本 PR 范围 = 设计 + migration（I10 第 1 步，表先 inert 落地）；repo/写/读（reviewer）路径接入 = 后续 PR**。

## 1. 问题（评审 #5）

coverage 矩阵是 `asset × technique`，回答「这格 found 没」。#4 `technique_outcomes` 加了 provenance（source/query/result_count/confidence/collected_at），但每 `(asset × technique)` **只一行**（`UNIQUE(run_id,asset,technique)`，重跑 upsert）——一个 technique 被**多个数据源**覆盖时（CT = crt.sh + ctfr；OSINT = 多 provider；DNS = 多 resolver）塌成一行，**证不了「逐源查询」**。

被动信息收集的完整性还需要证明「**哪些数据源查过、各自什么结果**」：我查了 CT、WHOIS、OSINT provider、代码平台——但为空 / 失败 / 无凭证。现状缺这一层：

- `audit_log`（evidence 底座）：有 `evidence_technique` / `evidence_outcome`，但**无** source(provider) / query / result_count / timing 的结构化列。
- `technique_outcomes`（#4）：(asset × technique) 单行，多源塌一行。

→ 需要更细一层：每 `(run × source × query × target)` 一行。

## 2. 与现有两层的关系（不重复造）

| 层 | 粒度 | 回答 | gate 读? |
|---|---|---|---|
| coverage（gate 判定） | (asset × technique) 终态 | 这格覆盖了没 | 是 |
| `technique_outcomes`（#4） | (run × asset × technique) 单行 + provenance | 哪个 provider 让这格 found（收口态） | 是（灰度 dual-read） |
| **`source_query_log`（#5，本设计）** | (run × source × query × target) 多行 | 逐条源查询的态/计数/用时/证据 | **否**（消费模型 A） |

三层正交：#5 是**最细的查询日志**，不替代前两层；coverage gate 判定**逐字节不变**（消费模型 A：gate 不读本表，仅 reviewer/报告/诊断读）。

## 3. Schema

见 migration `20260623000003_source_query_log.sql`。

| 列 | 说明 |
|---|---|
| id / organization_id(FK CASCADE) / run_id | PK + I2 org 隔离 + run 隔离 |
| `source` | 数据源/provider（crt.sh / rdap / subfinder / ENScan_GO …） |
| `query` | 实际查询/命令文本 |
| `target` | 被查资产 canonical_asset_key；`''` = org 级/非资产专属 |
| `technique` | 贡献的 technique id（GOLISH-INTEL-*）；NULL = 未映射 |
| `status` | `found` \| `empty` \| `error` \| `blocked`（同 #4 词表，承接 T2 error / I8 empty 只来自真「跑了→空」） |
| `result_count` | 结果条数（empty=0；NULL=未知） |
| `evidence_ids` | `BIGINT[]` 指 audit_log 真实行（I7） |
| `detail` | 备注/失败原因/无凭证说明 |
| `started_at` / `finished_at` | 查询用时 |
| created_at / updated_at | 落库审计 |

`UNIQUE(run_id, source, query, target)`：每 (run, 源, 查询, 目标) 一行，重跑同查询 upsert 幂等不堆叠。`target NOT NULL DEFAULT ''` 让 org 级查询的 UNIQUE 行为确定（避免 nullable 列在 UNIQUE 中视为相异而重复堆叠）。

## 4. 写 / 读（reviewer）集成（**本 PR 不实现，后续 PR**）

- **写（后续 step2，gray-switch）**：被动情报采集点（enrich provider 调用、命令路径 dig/subfinder/whois、CLI 兜底）book 证据时**同步 upsert** source_query_log：`source` 取 provider/工具名、`query` 取查询/命令、`target` 走 `canonical_asset_key`（org 级取 `''`）、`status` 同 `passive_intel_outcome_for_run`（含 T2 error）、`result_count` 取条数、`evidence_ids` 指 audit_log、timing 取实际起止。**非致命**：upsert 失败只 warn，不回滚证据（证据为底、本表为审计物化）。gray-switch（env，默认 off），与 #4 写路径同款。
- **读（后续 step3，reviewer/报告）**：reviewer / 报告生成 / `run_tree.py` 诊断从本表投影「源覆盖矩阵」（每 technique × 每 source 的查询态）。**coverage gate 判定不接本表**（消费模型 A）；future B（gate 强制源覆盖）若做，再在 gate 侧加灰度读 + 「每 technique 必需源」taxonomy（另立设计）。

## 5. 不变量

- **I2 IDOR**：`organization_id NOT NULL` + FK `ON DELETE CASCADE`；读写按 org 过滤（含批量）。命令路径无 org 的 session 级查询仍以 audit_log 为底座，不强塞本表。
- **I7**：`evidence_ids` 全指 audit_log 真实行；fabricated-ref 校验照跑。
- **I8**：`status=empty` 只来自真「跑了→空」；`error` = 失败阻断（承接 T2），二者不混；**缺行 = 该源未查**，绝不从缺行推断 empty。
- **I10 / §2.7**：分步——**本 PR 只建表（inert）** → 后续上写路径（gray-switch）→ 再上 reviewer 读。每步独立可回滚。
- **I5**：本表后端内部（审计/reviewer 用），不跨 IPC；前端要展示源覆盖再 derive ts-rs。

## 6. 依赖与顺序

- 依赖 **E1 规范键**（`canonical_asset_key`）：`target` 须规范键（org 级取 `''`），与 #4 同。
- 顺序：本 migration（step1）→ repo + 写路径（step2，gray-switch）→ reviewer/报告读（step3）。

## 7. 验证（本 PR）

- migration SQL 语法 sanity（CREATE TABLE IF NOT EXISTS + 2 索引 + 注释；纯新增）。
- 表 inert（无读无写）→ 既有测试零影响；`cargo check -p golish-db` 编译通过（migration 是静态资源，sqlx 编译期不校验未用表）。
- 活体（用户环境）：app 重启 apply migration → `\d source_query_log` 列齐、UNIQUE/索引在。

## 8. 回滚

- 表未接写/读 → 删 migration 文件（未 apply 时）或 `DROP TABLE source_query_log`（已 apply 时，无数据依赖）即回滚。
