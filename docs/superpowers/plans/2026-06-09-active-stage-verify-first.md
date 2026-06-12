# 主动阶段重排 + verify-first gate 实现计划（已执行）

> **状态：** crate 级已实现并验证（2026-06-09）。配套设计：`docs/design/2026-06-09-active-stage-verify-first.md`。**本计划取代**先前的"纯配置版"草稿（最终方案为"重排 + 纯配置 + 1 处 D2 Rust"）。

**目标：** ①把端口/服务前移到 `external_attack_surface`(EAS)、JS/API 移到 `enumeration`，修非标端口 web 服务的 JS/API 漏测；②两阶段用 `coverage_complete` + per-stage `expected_techniques` 强制"每个权威资产逐技术终态+证据"。

**架构：** 复用现成 `coverage_complete` 引擎 + 权威 `in_scope_assets` 资产轴 + `stage_charter` 自动渲染 + 词典 fail-closed 守卫。唯一 Rust 逻辑改动 = `surface_mapping.rs` 的 `D2_REQUIRED_CATEGORIES`（`[Surface, JsApi]`→`[Surface]`）。其余为 3 JSON + 测试 + 描述文案。

**技术栈：** Rust 2021、`cargo nextest`、JSON 资源（`include_str!` 内嵌）；crate `golish-agent-kit`（+ 顺带修 `golish-pentest`）。

---

## 文件结构

| 文件 | 职责 |
|---|---|
| `golish-pentest/src/handlers/env.rs` | 预存 blocker 修复（E0384 macOS conda 双赋值） |
| `harness/surface_mapping.rs` | D2_REQUIRED → [Surface]（核心逻辑）+ 测 |
| `harness/gate/surface_coverage_check.rs` | 测：only-Surface → Pass |
| `harness/gate/rule_engine.rs` | 测：named_check surface_coverage only-Surface → Pass |
| `harness/stage_spec.rs` | 测：EAS 3 技术 + 含 port-scan；enum 3 技术 + 无 port-scan |
| `task_orchestrator/prompts/mod.rs` | generator charter EAS/enum 描述 + EAS charter 测 |
| `task_orchestrator/subtask_phases/execute.rs` | K::ExternalAttackSurface / K::Enumeration 子任务描述 |
| `resources/harness/technique_taxonomy.json` | 6 个 GOLISH-EAS-*/ENUM-* |
| `resources/harness/stages/external_attack_surface.json` | +port-scan、3 EAS 技术、coverage_complete、描述 |
| `resources/harness/stages/enumeration.json` | -port-scan、3 ENUM 技术、coverage_complete、描述 |

---

## Task 0 — 解 blocker（前置）

`golish-pentest/src/handlers/env.rs` macOS conda 分支把不可变 `conda_installed`/`conda_path` 赋值两遍（E0384），阻断整条编译链。改为局部 `mut found_installed/found_path` 收集 → 单次赋值（与 Windows 分支风格一致）。
**验证：** `cargo check -p golish-agent-kit` 不再因 golish-pentest E0384 失败。✅

## Task 1 — RED（EAS+enum 技术断言）

`stage_spec.rs` 加/改测：EAS 含 `GOLISH-EAS-{LIVENESS,PORT,SERVICE-FINGERPRINT}` + coverage_complete + port-scan 工具；enum 含 `GOLISH-ENUM-{DIR,PARAM,JSAPI}` + coverage_complete + 无 port-scan。
**验证：** `cargo nextest -p golish-agent-kit external_attack_surface_requires_per_asset / enumeration_requires_per_asset` → FAIL（JSON 未改）。✅ RED

## Task 2 — 登记 technique_taxonomy（GREEN 前置）

加 6 条：`GOLISH-EAS-LIVENESS/PORT/SERVICE-FINGERPRINT` + `GOLISH-ENUM-DIR/PARAM/JSAPI`（防 `all_embedded_expected_techniques_are_recognized` fail-closed）。
**验证：** JSON 合法 + `technique_taxonomy` 测绿。✅

## Task 3 — 改两 stage JSON（GREEN）

- `external_attack_surface.json`：allowed +`recon/port-scan`；`expected_techniques=[EAS-LIVENESS,PORT,SERVICE-FINGERPRINT]`；gate_rules 加 coverage_complete（保留 surface_coverage）；描述。
- `enumeration.json`：allowed 去 `recon/port-scan`；`expected_techniques=[ENUM-DIR,PARAM,JSAPI]`；gate_rules 加 coverage_complete；描述。
**验证：** Task 1 测转 GREEN。✅

## Task 4 — 唯一 Rust 逻辑：D2_REQUIRED

`surface_mapping.rs`：`D2_REQUIRED_CATEGORIES = &[SurfaceCategory::Surface]`（去 JsApi）。同步改受影响的 3 处测试为"only-Surface → Pass"：
- `surface_mapping.rs::only_surface_satisfies_required_after_jsapi_moved_to_enumeration`
- `surface_coverage_check.rs::only_surface_passes_after_jsapi_moved_to_enumeration`
- `rule_engine.rs::named_check_surface_coverage_passes_on_surface_only`
**验证：** 三测绿。✅

## Task 5 — 描述文案

`execute.rs`（K::ExternalAttackSurface = 定义攻击面含端口；K::Enumeration = 内容枚举含 JS/API、勿重扫端口）+ `prompts/mod.rs` generator charter 两行同步。
**验证：** EAS charter 测含 `GOLISH-EAS-LIVENESS`；编译绿。✅

## Task 6 — 全量回归收口

**验证（本机实跑 2026-06-09）：**
- `cargo nextest -p golish-agent-kit` → **528 passed / 0 failed**
- `cargo clippy -p golish-agent-kit --all-targets` → 零告警
- `cargo fmt -p golish-agent-kit --check` → clean
- 3 JSON `json.load` → 合法

## Task 7 —（待办）收尾

⏳ full `just precommit`（前端 + 全 workspace）｜ ⏳ 端到端 MiMo（red_team 到 EAS/enum，确认非标端口 web 的 JS/API 不漏 + gate BLOCK→PASS，需授权烧额度）｜ ⏳ 更新 `agent-progress.md`/`feature_list.json`（加条目）｜ ⏳ commit。

---

## 自检

**1. 规格覆盖度（对照设计 §3）：** 重排职责(Task 3,5)✓ / 6 技术(Task 2)✓ / D2(Task 4)✓ / 两阶段 coverage_complete(Task 3)✓ / 去重(0 代码，设计 §3.4 实读 CLOSED)✓ / 描述(Task 5)✓。**P1（专用枚举工具、分母联动）不在本计划范围**，另起。
**2. 占位符扫描：** 无 TODO；Task 7 为显式待办（需授权/全栈），非占位。
**3. 类型一致性：** 技术 id `GOLISH-EAS-*`/`GOLISH-ENUM-*` 在 taxonomy / stage JSON / stage_spec 测三处一致；`D2_REQUIRED_CATEGORIES` 单一来源。

> 已执行到 Task 6（crate 级全绿）。Task 7 收尾待用户决定（precommit / 端到端 / commit）。
