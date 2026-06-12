# Coverage 资产盘按 operation/organization 隔离 设计

> 目的：修一个**操作隔离 gap**——harness 的 `coverage_complete` gate 用 `in_scope_assets()` 作为"资产维度分母"，但该集合取的是**全库** `SELECT DISTINCT value FROM targets WHERE scope='in'`（不按当前 operation / org 过滤）。持久化的嵌入式 Postgres 跨多次 run / 多个 org 累积 in-scope 资产后，coverage 分母会**爆炸**——一次只打一个 org 的 operation，被迫对全库所有 org 的 in-scope 资产逐个 × 每类技术求覆盖，导致 stage（尤其 target_intel）gate 长期 BLOCK / 无限 grind。
>
> 实测证据（2026-06-09）：headless `--stage-run --org vulnweb --target testhtml5.vulnweb.com` 时，gate 注入的 in-scope 资产是 **41 个**（`*.acme.com`×8、`*.example.com`×8、`pingan.*`、`moresec.cn`、`44.228.249.3`… 全是历次会话/fixture 残留），target_intel 因 `intel coverage incomplete`（41×6 ≈ 246 格永远凑不齐）反复 BLOCK，跑 25min 未出 target_intel。临时处置：把 39 个非本测试资产降为 `scope='out'`（可逆）后只剩 2 个，coverage 盘恢复正常。
>
> 关联背景：`2026-06-05-coverage-matrix.md`（coverage_complete + 资产轴注入 ①③ seam）、`2026-06-09-active-stage-verify-first.md`（EAS/enumeration coverage 技术）。
>
> 证据来源：§1 为 2026-06-09 本会话读码 + 实跑 trace 核对。日期：2026-06-09。

---

## 0. 决策（TL;DR）

- **问题**：`in_scope_assets()`（`db_traits/repo.rs`）→ `in_scope_values(None)`（`golish-app-core/ports/recon/targets.rs`）→ `golish-db .../targets.rs::list_in_scope_values` 的 SQL = `SELECT DISTINCT value FROM targets WHERE scope='in' AND ($1 IS NULL OR project_path=$1 OR project_path='')`，且调用方传 `None` → **全库所有 in-scope target**。coverage_complete 拿它当分母 → 跨 org/跨 run 污染时分母爆炸。
- **根因**：① 资产轴是**全局**的，没绑定当前 operation 的 organization；② 嵌入式 PG 持久，targets 跨会话累积；③ headless seed 只 add 不 clear。
- **方案**：coverage 资产轴**按当前 operation 绑定的 `organization_id` 过滤**。一次 operation 只评估一个 org，分母就是该 org 的 in-scope 资产。
  - DB：`list_in_scope_values` 增加 `org_id` 过滤（向后兼容，None=旧全局行为）。
  - 端口/trait：`in_scope_assets` 带上当前 operation 的 `org_id`。
  - 运行时：把 operation 的 `org_id` 透到 `GateContext`（scoping 确定 org；headless 由 `--org` seed、GUI 由用户选）。
- **范围**：DB 查询 +1 参数、端口/trait/调用方 +org_id、orchestrator→gate 透传 org_id；+ 测试。不改 coverage_complete 引擎本身。
- **非目标**：不改 coverage_complete 算法；不做 per-session 级隔离（org 级足够）；不动 GUI（GUI 本就单 org 工作，受益于同一修复）；DB 清理脚本不入产品（一次性运维）。

---

## 1. 现状勘验（本会话读码 + 实跑 2026-06-09）

| 环节 | 落点（已核） | 问题 |
|---|---|---|
| 资产轴取数 | `db_traits/repo.rs::in_scope_assets()` → `in_scope_values(None)` | 传 None = 全库 |
| 端口 | `golish-app-core/src/ports/recon/targets.rs::in_scope_values(project_path)` | 只有 project_path 维度，无 org |
| SQL | `golish-db/src/repo/targets.rs::list_in_scope_values`：`WHERE scope='in' AND ($1 IS NULL OR project_path=$1 OR project_path='')` | 无 org 过滤；project_path 对 chat/headless 不可靠（chat 传 None） |
| 注入 | `execute.rs` GateContext `in_scope_assets`（来自 repo.in_scope_assets()，非空才注入） | 注入的是全局集 |
| 消费 | `harness/gate/rule_engine.rs::coverage_complete`：资产 × `expected_techniques` 逐格核终态 | 分母 = 全局集 → 爆炸 |
| targets 模型 | `golish-app-core/domain/targets.rs::Target.organization_id: Option<String>` | **已有 org_id 字段**，seed/manage_targets 会绑定 |
| 实测 | trace：`injecting authoritative in-scope assets ... asset_count=10`→实查 41 条 in-scope（多 org/多 run）；target_intel `gate BLOCK intel coverage incomplete (+58 more)` | 分母爆炸实锤 |

> **核心洞察**：`targets.organization_id` 已经存在且 seed 会绑定，缺的只是"coverage 资产轴按当前 operation 的 org 过滤" + "把 operation 的 org 透到 gate"。这是隔离维度缺失，不是数据模型缺失。

---

## 2. 目标 / 非目标

**目标**
1. coverage 资产轴按当前 operation 的 `organization_id` 限定，分母只含本 org 的 in-scope 资产。
2. 持久 DB + 多 org/多 run 不再相互污染 coverage。
3. 向后兼容：org_id 缺省（None）时退回旧全局行为（不破现有 GUI/测试）。

**非目标**
- 不改 coverage_complete 引擎算法。
- 不做 per-session/per-operation 级 target 标记（org 级隔离足够，YAGNI）。
- 不在产品里加"清库"命令（一次性运维脚本，不入库）。
- 不动 project_path 语义（保留，作为次级过滤）。

---

## 3. 提议设计

### 3.1 资产轴按 org 过滤（DB 层）

`golish-db/src/repo/targets.rs::list_in_scope_values` 增加可选 `org_id`：
```sql
SELECT DISTINCT value FROM targets
WHERE scope = 'in'
  AND ($1 IS NULL OR project_path = $1 OR project_path = '')
  AND ($2 IS NULL OR organization_id = $2)
ORDER BY value
```
`$2 = org_id`（None → 不按 org 过滤，旧行为）。

### 3.2 端口 / trait 带 org_id

- `ReconTargetsPort::in_scope_values(project_path, org_id)`（+1 参数）。
- `db_traits/repo.rs::in_scope_assets(org_id)`（默认实现仍返回空；app 层覆盖时透传 org_id）。

### 3.3 把 operation 的 org_id 透到 gate（关键）

- operation 在 **scoping** 阶段确定/创建其 organization（headless：`--org` 经 `seed_upstream` 建 org 拿到 `org_id`；GUI：用户选的 org）。
- orchestrator 持有该 `org_id`，组装 `GateContext` 时一并带上（execute.rs 现注入 in_scope_assets 处）。
- **待核（§9-1）**：operation→org_id 的现成链路——`seed_upstream` 返回了 org_id（trace 见 `org=Some("example") (id=Some(8dc25c89-...))`）；需确认 orchestrator / session 是否已持有它，还是要新加一处透传。

### 3.4 缺省/回退

- operation 无 org（如某些不绑 org 的任务）→ org_id=None → 退回全局行为（不回归）。
- 但本设计推荐：harness operation 总应绑一个 org（scoping 产物），逐步让 org_id 必填。

### 3.5 改动文件清单

| 文件 | 改动 |
|---|---|
| `golish-db/src/repo/targets.rs` | `list_in_scope_values` +org_id 过滤（SQL + 签名）+ 测 |
| `golish-app-core/src/ports/recon/targets.rs` | `in_scope_values` +org_id 参数 |
| `golish-agent-kit/src/db_traits/repo.rs` | `in_scope_assets` +org_id |
| `golish-agent-app/.../recon.rs`（impl） | 透传 org_id |
| `task_orchestrator/.../execute.rs` | 组装 GateContext 时带 operation org_id |
| orchestrator / session | 持有并传递 operation 的 org_id（§9-1 实读后定接线点） |

---

## 4. 数据流图

```mermaid
flowchart TD
  SC[scoping 确定/创建 org] --> OP[(operation org_id)]
  OP --> ORCH[orchestrator]
  ORCH -->|GateContext.org_id| GATE
  DB[(targets 表)] -->|list_in_scope_values(project, org_id)| ASSETS[in-scope 资产 仅本 org]
  ASSETS --> GATE{coverage_complete 分母 = 本 org 资产 × expected_techniques}
  GATE -->|仅本 org 资产| PASSBLOCK[PASS/BLOCK]
```

---

## 5. 错误处理 / 边界

- **org_id=None**：退回全局（旧行为），不 panic、不回归。
- **org 下无 in-scope 资产**：注入空 → execute.rs 现有"非空才注入"逻辑回退自报集（现行为）。
- **同一资产挂多 org**（如 acme.com 在 8 个 org）：按 org 过滤后只算当前 org 的那条，天然解决跨 org 重复。
- **project_path + org 双过滤**：org 为主、project_path 为辅；chat/headless project_path=None 时靠 org 隔离。

---

## 6. 风险 / 回滚

- **R1 org_id 透传链路未现成**：§9-1 实读 orchestrator/session→org 后定接线；若复杂，先在 headless（seed 有 org_id）落地，GUI 路径随后。
- **R2 旧数据 org_id 为 NULL**：历史 targets 可能无 org（trace 见 `org=NULL`）→ 按 org 过滤会漏掉它们。可接受（它们本就是污染/历史）；必要时运维迁移补 org。
- **R3 向后兼容**：org_id=None 退回全局 → 现有 GUI/测试不破。
- **回滚**：org_id 一律传 None 即回到全局行为；纯增量参数，回滚安全。

---

## 7. 验证策略（DoD）

- **单测**：
  - `list_in_scope_values(project, Some(org))` 只返回该 org 的 in-scope；`None` 返回全局（向后兼容）。
  - coverage_complete：注入 org-scoped 资产后，分母只含本 org（用多 org fixture 验证不串）。
- **集成/实跑**：clean DB 后 `--stage-run --org vulnweb`，trace 确认 `asset_count` 只含 vulnweb 资产、target_intel 不再因外部 org 资产 BLOCK。
- **回归**：现有 harness e2e + coverage 测全绿（org_id=None 路径不变）。
- **证据**：`just precommit` 全绿。

---

## 8. 与 AGENTS.md 不变量对齐

- **I2 IDOR/scope**：coverage 仅核当前 org 的资产，强化操作隔离。
- **I5 ts-rs**：不改跨 IPC 类型（内部查询参数）。
- **I6**：新增设计文件。
- **I10 schema**：不改 schema（复用现有 `organization_id` 列）；纯查询过滤。

---

## 9. 开放问题（实现前需核 / 拍板）

1. **（必核）operation→org_id 链路**：实读 orchestrator / session / scoping 产物，确认 operation 的 org_id 在 gate 注入点（execute.rs）是否已可得；`seed_upstream` 返回了 id，但 GUI/chat 路径的 org 来源需核（可能来自 scoping 阶段创建/选定的 org，或 session 绑定）。
2. **（拍板）org_id 缺省策略**：None→全局（向后兼容，推荐起步）vs 逐步必填（更强隔离）。
3. **（运维）历史污染清理**：本次用脚本把 39 个非测试资产降 'out'（可逆）。是否要一个一次性迁移把历史 targets 归到正确 org / 清理？（不入产品）

---

## 10. 分期与后续

- **本期（P0）**：`list_in_scope_values` +org 过滤 + 端口/trait/impl 透传 + execute.rs 注入 org_id + 单测；headless 实跑验证（clean DB 后只见本 org 资产）。
- **P1**：GUI 路径 org_id 透传核对（若与 headless 不同）；org_id 逐步必填。
- **后续**：考虑 per-operation target 集（更细隔离）与 coverage 分母联动（enumeration 单元 → vuln_triage total_units）。

> 下一步：用户审查 → 实读 §9-1（org_id 链路）→ writing-plans 出实现计划 → executing-plans（DB 测先行、端口透传、execute.rs 接线、实跑验证）。本设计独立新增，不覆盖旧文档（I6）。
