# #4 · technique_outcomes 物化表 + provenance（DB-truth 带 source）

> 评审 claim #4。**建在已拍板的 E3 之上**：`docs/design/2026-06-18-canonical-asset-identity-and-coverage-join-key.md` §3.3（D0「建独立物化表」用户 2026-06-18 已确认）+ 计划 `docs/superpowers/plans/2026-06-18-pr-b-canonical-key-wiring.md` PR-C/PR-D。状态：进行中（2026-06-23）。**本 PR 范围 = 设计 + migration（I10 第 1 步，表先 inert 落地）；repo/写/读路径接入 = 后续 PR**。

## 1. 问题（评审 #4）

`coverage_truth_facts`（`coverage_truth.rs:368`）只返回 `(asset, technique)` 二元组——gate 知道「这格 found」，但**答不出来源**：哪个 provider、哪条 query、何时采、置信几何。读 SQL 实证：per-asset 维只 `SELECT DISTINCT value`；org-intel 维（ASN/CT/WHOIS/OSINT）是 `organizations.*` JSON blob 的 presence bool。source/query/confidence **库里根本没存** → 必须落 provenance（schema）。

## 2. 与 E3 的关系（不重复造）

E3 已设计 `technique_outcomes` 作为 coverage gate 的**单一真值源**（每 `(run × asset × technique)` 一行，带 `outcome / evidence_ids / seq`，`asset` 用 E1 规范键）。#4 与 E3 **本质同一张表**——都要 per-(asset,technique) 物化行——只是 #4 强调 **provenance**（source/query/confidence/collected_at）。

**故 #4 = E3 表 + provenance 列**。本设计在 E3 §3.3 schema 上**加列**，不另起炉灶；E3 的 D0–D5 决策（独立表 / 规范键 asset / seq 每 run 自增 / org 隔离）全部沿用。

## 3. Schema（E3 §3.3 + #4 provenance 列）

见 migration `20260623000002_technique_outcomes.sql`。在 E3 基础上：

| 列 | 来源 | 说明 |
|---|---|---|
| id / organization_id / run_id / asset / technique / outcome / evidence_ids / seq / created_at / updated_at | **E3 §3.3** | 不变（沿用） |
| `outcome` 取值 | E3 + **T2** | `found`\|`empty`\|`error`\|`blocked`（承接 T2 的 error 第三态；I8：empty 只来自真「跑了→空」） |
| **`source`** | #4 新增 | 数据源/provider（`crt.sh` / `rdap` / `subfinder` …），NULL=未知 |
| **`query`** | #4 新增 | 实际查询/命令文本 |
| **`result_count`** | #4 新增 | 结果条数（empty=0） |
| **`confidence`** | #4 新增 | 0..1 置信度，NULL=未知 |
| **`collected_at`** | #4 新增 | 该维实际采集时刻（freshness 用；org-intel 可取 `*_collected_at`，命令路径取 run 时刻） |

`UNIQUE(run_id, asset, technique)`：每维一行，重跑 upsert 幂等不堆叠（解「同维反复堆叠」）。

## 4. 写 / 读集成（**本 PR 不实现，后续 PR-C/D**）

- **写（PR-C step 2）**：命令路径（`evidence_facts` 落点）+ enrich/landing（`persistence.rs` 钩子）book 证据时**同步 upsert** `technique_outcomes`：`asset` 走 `canonical_asset_key`、`outcome` 同 `passive_intel_outcome_for_run`（含 T2 error）、`source/query` 取工具/provider 与命令、`evidence_ids` 指 audit_log、`seq` 取 run 内自增、`collected_at` 取采集时刻。非致命：upsert 失败只 warn，不回滚证据（证据为底、表为物化）。
- **读（PR-D step 3，灰度）**：gate 新增「从 `technique_outcomes` 投影 `EvidenceFact`」读路径，**dual-read** 与现有 `coverage_truth` union 并行比对、parity 稳后 opt-in 切换（先 target_intel）。`rule_engine` 判定逻辑 **byte-for-byte 不变**（仍 `has_fact(Found/Empty/Error)`，只换 facts 来源）。

## 5. 不变量

- **I2 IDOR**：`organization_id NOT NULL` + FK `ON DELETE CASCADE`；一切读写按 org 过滤（含批量）。命令路径无 org 的 session 级事实仍留 audit_log 底座（不强塞本表）。
- **I7**：`evidence_ids` 全指 audit_log 真实行；fabricated-ref / existence 校验照跑。
- **I8**：`outcome=empty` 只来自真「跑了→空」；**缺行 = not_attempted = gate 照旧 BLOCK**，绝不从缺行推断 empty。
- **I10 / §2.7**（建表 D0 已确认）：分步——**本 PR 只建表（inert）** → 后续上写路径 → 再上读路径（灰度 dual-read）→ 旧 union 留作回退后再清。每步独立可回滚。
- **I5**：本表默认后端内部（gate/审计用），不跨 IPC；若前端要展示逐维进度再 derive ts-rs。

## 6. 依赖与顺序

- 依赖 **E1 规范键**（`canonical_asset_key`，PR-A 已落 + PR-B 边界接入部分完成）：`technique_outcomes.asset` 必须是规范键，否则 gate join 照样漂移。读/写路径接入前需确认 PR-B 的 `in_scope_assets` 归一就绪。
- 顺序：本 migration（PR-C step 1）→ repo + 写路径（PR-C step 2）→ gate 读路径灰度（PR-D）→ 清旧 union（PR-E，可选）。

## 7. 验证（本 PR）

- migration SQL 语法 sanity（CREATE TABLE IF NOT EXISTS + 索引 + 注释；纯新增）。
- 不接写/读 → 既有测试零影响（表 inert）；`cargo check -p golish-db` 编译通过（migration 是静态资源，sqlx 编译期不校验未用表）。
- 活体（用户环境）：app 重启 apply migration → `\d technique_outcomes` 列齐、UNIQUE/索引在。

## 8. 回滚

- 表未接写/读 → 删 migration 文件（未 apply 时）或 `DROP TABLE technique_outcomes`（已 apply 时，无数据依赖）即回滚。
