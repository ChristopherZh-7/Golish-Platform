# Scoping = ORG 层：不注入 in-scope 资产 + 已存在 org 复用(R2) — 设计

> 日期：2026-06-13
> 状态：设计（已与用户对齐方向并批准：隔离档=按 org；scoping 不看 scope=in；复用策略=R2）。
> 父设计：`docs/design/2026-06-13-engagement-scoping-fanout-redesign.md`（§5 阶段化颗粒度、§6.2 scoping 独立化）。
> 关系：本设计是 `2026-06-13-engagement-phase-a-scoping.md` 的一个**聚焦 bug-fix 切片**，只治「scoping 被跨 engagement 脏资产污染」这一具体问题；不重复 Phase A 的 lookup 工具 / create_batch / snapshot / ts-rs 等更大工作量。
> 不变量：AGENTS.md I2（org 归属/IDOR）、I7（阶段交付有 evidence）、I8（已检查为空 ≠ 未检查）。

---

## 1. 问题（实测复现）

用户「搞平安」时，scoping 阶段的 AI 被上一轮 moresec.cn 测试残留的资产带偏，把「平安」降级成「需澄清」。

根因（实读 `subtask_phases/execute.rs:132-139`）：**每个 stage** 拼 prompt 时都会注入「DB 里 `targets.scope='in'` 的资产清单」（`repo.in_scope_assets(harness_org_id)` → `render_in_scope_assets`）。但：

- scoping 跑在 recon **之前**，本不该有「本次」资产；
- scoping 时 `harness_org_id = None`（未绑 org，见 `orchestrator.rs:111`）→ `list_in_scope_values` 的 org 过滤关闭 → 把**全工作区 + 历史 `project_path=''`** 的 in-scope 资产全捞出来；
- 于是上次/别的 org 的 `*.moresec.cn` 作为「权威资产」塞进了 scoping prompt。

数据库脏数据已在本会话清空（targets 95→0, organizations 33→0，独立 verify 归零），但那是**治标**；不改代码下次仍会复发。

## 2. 决策（用户已拍板）

1. **隔离档 = 按 org**（不是按 workspace、也不是按 session）。数据靠 `targets.organization_id` 归属；干活靠 `harness_org_id` 收窄查询。recon 按「母+子家族」一起、attack 按单 org 拆——即父设计 §5 的阶段化颗粒度。本切片只修 scoping 这一环，不实现完整 fan-out。
2. **scoping 是 ORG 层，不是 ASSET 层**：scoping 要的只有「这个集团 org 建了没 + 子公司树完不完整」，不需要 `scope=in` 资产清单（那是 recon 的产物）。
3. **复用策略 = R2**：scoping 开头先查 DB；命中已存在的 org → 复用其树 + 走一次 `ask_human` 重确认 → 提交；未命中 → 正常纠名/发现/建树。

## 3. 本切片的两处改动（最小、可测）

### 改动 1（代码）：scoping 阶段不再注入 in-scope 资产

- 新增纯函数 `prompts::render_in_scope_assets_for_stage(stage_kind, assets)`：`stage_kind == StageKind::Scoping` 时返回空串，否则透传现有 `render_in_scope_assets(assets)`。把「scoping 不注入」这条策略**收敛到一个可单测的点**。
- `execute.rs:137` 的注入调用从 `render_in_scope_assets(&assets)` 改为 `render_in_scope_assets_for_stage(hint.stage_kind, &assets)`。
- 其余阶段（target_intel/EAS/…）行为零变更，仍拿到（按 org 收窄的）权威资产。

### 改动 2（prompt/方法论）：scoping 先查 DB 复用(R2)

- `resources/harness/stages/scoping.methodology.md` 增「STEP 0 · 复用优先」：先 `manage_organizations(action="list")`，若本次主体集团已有 root org → 进入 REUSE 模式：**不重纠名、不重建**，直接对既有树走一次 `ask_human(unit_review)` 重确认，然后 `submit_stage_deliverable`；未命中才走原 1–5 步建树。
- 复用所需工具**已存在**：`manage_organizations action="list"` 读现有 org（IDOR 按 project 收窄）；`create_batch` 本就是 get-or-create（不重复建）。本切片**不加新工具、不改 schema**。

## 4. 为什么安全（影响面）

- **不动 gate**：scoping 的子公司完整度判定仍由 gate 读 DB 真值（`scoping.json` coverage_complete 读 `organizations.parent_id`）；本切片只改 prompt 注入，不碰 `fetch_in_scope_assets_for_gate`（execute.rs:241/414）与 gate 规则。
- **scoping 证据豁免**不变（`scope_check.rs`）。
- 不改 IPC 类型、不改 DB schema、不动外部接口。

## 5. 验证（DoD）

- 单测 `prompts`：`render_in_scope_assets_for_stage(Scoping, assets) == ""`；非 scoping 阶段仍含 `IN-SCOPE ASSETS` 段与资产值。（TDD：先红——证明旧行为会注入——再绿。）
- `execute.rs` 注入点改用新函数（人工核对 diff，唯一注入点已切换）。
- `scoping.methodology.md` 含 STEP 0 复用流程 + R2 重确认。
- 验证命令（scoped）：
  ```bash
  cd backend && cargo nextest run -p golish-agent-kit --status-level fail
  cargo clippy -p golish-agent-kit --all-targets -- -D warnings
  cargo fmt -p golish-agent-kit -- --check
  ```
- 活体（留用户环境）：在带残留的工作区跑一次 scoping，确认 prompt 不再出现 IN-SCOPE ASSETS 段、且对已存在 org 走复用+重确认。

## 6. 非目标

- 不实现完整 org fan-out / 会话工人池（父设计 Phase B/C）。
- 不做企查查纠名工具 `recon_lookup_company` / engagement snapshot（Phase A 其余部分）。
- 不给 scoping prompt 注入具体 org 状态块（靠 methodology 让 agent 自己 `list` 查；如需结构化注入留作后续）。
