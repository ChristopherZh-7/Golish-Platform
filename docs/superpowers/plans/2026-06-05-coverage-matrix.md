# 覆盖矩阵（Coverage Matrix）实现计划

> **面向 AI 代理的工作者：** 必需子技能：`.cursor/skills/executing-plans` 逐任务执行；每任务 `.cursor/skills/test-driven-development`（先写失败测试 → 红 → 实现 → 绿 → commit）。设计：`docs/design/2026-06-05-coverage-matrix.md`。
>
> **目标：** 给 `StageDeliverable` 加结构化 coverage 段 + `gate_rules` 配套积木（`Collection::Coverage` + `coverage_complete` op），让「每个资产 × 每类技术都有终态、零 not_attempted」成为纯 JSON 可声明、确定性可校验的过关标准。
>
> **加性、可回滚：** coverage 缺省空数组 = 今天行为；空 `expected_techniques` = `coverage_complete` no-op。先加能力，样例 stage 才接。

> **执行状态（2026-06-05 · MCP-agent-4）：** 用户拍板完整版（资产从 DB / technique 挂标准 / expected 走 skeleton / checked_empty 也要证据，见设计 §6.5）。
> - **✅ Phase 1 已实现并验证（本次，不依赖资产库）：Task 1（数据模型）+ Task 2（Coverage 集合 + status 谓词 + ④ found/checked_empty 证据规则能力）。** 511 nextest 全绿 + clippy -D 零告警 + fmt clean。
> - **⏸ Phase 2 deferred（Task 3-7 的活体部分）：** `coverage_complete`（需从 DB 注入 in-scope 资产，§2.7 DB 确认）+ skeleton 动态生成 expected（Task 4 的动态版）+ WSTG/ATT&CK 标准映射（Task 2 的 ② 升级）+ submit schema/charter（Task 5）+ 样例 stage（Task 6）。**阻塞于同事资产库合入 + 用户 DB 授权。**
> 下方 Task 1/2 已落地；Task 3-7 按 Phase 2 触发时执行（其中 Task 3 改为 `eval_with_context` 注入资产；Task 4 改为 skeleton 生成）。

---

## 前置约定

- 受影响 crate：`golish-agent-kit`（types / rule_engine / stage_spec / prompts）+ `golish-agent-app`（submit 工具 schema）。
- 验证命令（每任务跑相关子集，收口跑全套）：
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(rule_engine)|test(harness::types)|test(coverage)' --status-level fail`
  - `cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail`
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings && cargo fmt --check`
- **开放问题先按设计 §6 默认值落地**（资产=deliverable 自报；technique=自由字符串；expected 放 spec；found 强制证据、checked_empty/blocked/n_a 要 note）。若评审改这些默认，调对应任务。

---

## Task 1 · 数据模型（types.rs）+ serde 测试（TDD）

**1.1（红）** `harness/types.rs` 测试加：CoverageCell serde 往返（含 status 各值）；StageDeliverable 带/不带 coverage 都解析（不带 → 空）。

**1.2（绿）** 实现（设计 §3）：`CoverageStatus`（found/checked_empty/blocked/not_applicable，snake_case）+ `CoverageCell { asset, technique, status, #[serde(default)] evidence_refs, #[serde(default)] note }` + `StageDeliverable` 加 `#[serde(default)] pub coverage: Vec<CoverageCell>`。

**1.3** 修因加字段而炸的 StageDeliverable 字面构造（grep `StageDeliverable {`；多数测试 helper 用 `..` 或逐字段——逐字段的补 `coverage: vec![]`；`#[serde(default)]` 不救 Rust 字面）。预期点：各 gate check 测试的 helper + e2e_tests + execute_harness_loop_tests。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(harness::types)'` 绿 + `cargo check -p golish-agent-kit --tests`（找全字面构造）。**commit**：`feat(harness): add coverage matrix types to StageDeliverable`。

---

## Task 2 · rule_engine：Coverage 集合 + 字段 + Eq(status)（TDD）

**2.1（红）** rule_engine 测试：`for_all over coverage where {eq status found} require {non_empty evidence_refs}` —— 一个 found 缺证据的 cell → Block；都挂证据 → Pass。

**2.2（绿）** 实现：
- `Collection::Coverage`（+ `items()` 分支返回 coverage cells）。
- `ItemRef::Coverage(&CoverageCell)`；`ItemField` 加 `Asset/Technique/Status`；`resolve` 加 coverage 分支（asset/technique/note→Text，status→新 FieldVal::Status，evidence_refs→List）。
- `Pred::Eq` 支持 status：`resolve` 返 status 时与 value 文本比较（加 `status_to_str`）。
- `pred_holds` / `non_empty` 对 coverage.evidence_refs 走 List 分支。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(rule_engine)'` 绿。**commit**：`feat(harness): coverage collection + status predicate in gate rule engine`。

---

## Task 3 · rule_engine：`coverage_complete` op（读 spec.expected_techniques）（TDD）

**3.1（红）** 测试（构造 deliverable.coverage + 内联 spec.expected_techniques）：
- 缺 (asset×technique) → Block，reason 含缺失对。
- 全部终态 → Pass。
- `expected_techniques` 空 → Pass（no-op）。
- `terminal_status` 限定为 `["found"]` 时，checked_empty 的格也算缺 → Block。
- serde：`coverage_complete` 解析；status 写错值 → Err（fail-closed）。

**3.2（绿）** 实现 `GateRule::CoverageComplete { #[serde(default)] terminal_status: Option<Vec<CoverageStatus>>, on_fail }`；`eval_one` 加分支（设计 §4.2 求值：techniques=spec.expected_techniques；assets=coverage distinct asset；逐 (asset,technique) 查终态 cell；缺口→block_from + 附前 N 缺失对）。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(rule_engine)'` 绿。**commit**：`feat(harness): coverage_complete gate op (expected techniques × assets)`。

---

## Task 4 · `StageSpec.expected_techniques`（TDD）

**4.1（红）** stage_spec 测试：内联 spec 带 expected_techniques 解析出数组；缺省空。

**4.2（绿）** `StageSpec` 加 `#[serde(default)] pub expected_techniques: Vec<String>`。修 `spec_with` 等字面构造补该字段（finding_verification_check.rs 的 helper）。

**验证：** `cargo nextest -p golish-agent-kit -E 'test(stage_spec)|test(all_twelve)'` 绿。**commit**：`feat(harness): StageSpec.expected_techniques for coverage_complete`。

---

## Task 5 · submit 工具 schema + stage charter 提示

**5.1** `harness_submit_tool.rs::parameters()` 加 `coverage` 数组属性（item: asset/technique/status/evidence_refs?/note?），描述强调「checked_empty 要证据/理由；未测不要进矩阵（= not_attempted = 不过关）」。构造零改（serde default 接）。

**5.2** `prompts/mod.rs::stage_charter`：当 `spec.expected_techniques` 非空，追加一行「本阶段须对每个资产覆盖技术：<join> —— 每格 found+证据 / checked_empty+证据 / blocked|n_a+理由」。

**验证：** `cargo nextest -p golish-agent-app -E 'test(harness_submit_tool)'` + `cargo nextest -p golish-agent-kit -E 'test(prompts)'`（如有）绿。**commit**：`feat(harness): expose coverage in submit schema + stage charter`。

---

## Task 6 · 样例 stage 接入 + 集成测试

**6.1** 选一个攻击 stage（建议 `vuln_triage`）配：`expected_techniques`（如 `["sqli","xss","idor","ssrf"]`）+ `gate_rules` 追加 `{op:coverage_complete,...}` + `{for_all over coverage where status==found require non_empty evidence_refs}`。

**6.2** `gate/mod.rs` 集成测试：用迁移后 vuln_triage embedded spec，coverage 缺 idor 格 → `validate_stage_gate` Block 且 reason 含 coverage incomplete；补齐 → Pass。

**验证：** `just test-harness` + `all_twelve_stage_specs_load` 绿。**commit**：`feat(harness): wire coverage gate on vuln_triage (sample) + integration test`。

---

## Task 7 · 文档 + 收口

**7.1** `docs/design/2026-06-02-harness-stage-spec-reference.md` §8 补 `coverage_complete` op + `over:coverage` + status 谓词；指针到本设计。
**7.2** 收口：
```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --no-fail-fast --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings && cargo fmt --check
```
**7.3** 证据复制进 `agent-progress.md`；`feature_list.json` 加 `coverage-matrix-2026-06-05`（passing + evidence）。commit：`chore(harness): record coverage matrix evidence + feature status`。

---

## 自检

**规格覆盖**（对照设计）：§3 数据模型→T1；§4.1 Coverage 集合→T2；§4.2 coverage_complete→T3；§4.3 expected_techniques→T4；§5 agent 填→T5；§7.6 样例→T6；§9 验证→T3/T6/T7。

**加性/可回滚**：coverage `#[serde(default)]` 空 = 旧行为；expected_techniques 空 = coverage_complete no-op；样例只接 1 个 stage。每任务独立 commit。

**fail-closed**：CoverageStatus / coverage_complete / 新 ItemField 全 typed enum → `all_twelve_stage_specs_load` 抓写错。

**开放问题**（设计 §6，执行前确认默认是否调整）：资产源（自报 vs 注入）、technique 词表、expected 放 spec vs skeleton、checked_empty 是否强制证据。MVP 取设计默认；评审改则调 T3/T4/T5。

**YAGNI**：不做资产集注入、不内置 WSTG/ATT&CK 词典、不做 skeleton 动态生成、不做 UI——均列为 Phase 2。
