# 红队 Phase 3：多 org coverage 轴（方案 A · 调度层逐 org）实现计划

> **面向 AI 代理的工作者：** 使用 `executing-plans` 逐任务实现。每个任务先写失败测试（TDD），看它失败，再写最小实现，再验证。
> 设计：`docs/design/2026-06-12-redteam-phase3-multi-org-coverage.md`（方案 A 推荐起步）+ 总纲 `docs/design/2026-06-12-redteam-db-truth-master.md`。

**目标：** `--stage-run --include-subsidiaries --to <stage>` 跑完母 org 的 slice 后，对 Phase 2 落库的每个合格子 org（`organizations.parent_id = 母`）串行再跑一遍 `target_intel..=<stage>` 的 slice（子 org 不重跑 scoping——授权与 org 树在母趟完成）；engagement 级结局 = 母与所有子全 OK 才 OK，任一 BLOCK/失败 → 整体 FAILED（设计 §2「漏掉任何一个 org = BLOCK」）。

**架构（方案 A，gate 零改动）：** coverage gate 天然按 org 隔离（06-10：`in_scope_assets(org_id)`、coverage_truth 按 org 查）——每趟 `orchestrate()` 换绑 `harness_org_id` 即可让 Phase 0/1/2 的 DB 真值判定自动对每个 org 生效。改动全部落在 `golish/src/stage_run/mod.rs` 调度层 + 纯函数。**不动** rule_engine / execute.rs hook / coverage_truth / orchestrator 内核。

**技术栈：** Rust 2021、`golish`（stage_run CLI）、`golish-db::repo::organizations::list`（已存在，project 隔离）、`cargo nextest` / `clippy -D warnings`。

---

## 0. 现状勘查（2026-06-12 实读）

- `run()` 主流程：seed（母 org + targets）→ bridge/session（单 session_id 四身份同值）→ `orchestrate(...)` 单趟（内部新建 orchestrator + 独立 task/operation_state + `run_stage(entry, allowlist)`）→ `format_report` → exit（`result.map(|_| ())`；**BLOCK 终态 = orchestrate Err("stage blocked") → exit 非 0**）。
- `orchestrate()` 已接收 `include_subsidiaries/subsidiary_threshold`（Phase 2）并 `set_subsidiary_scope`。
- `auto_promote_discovered_children`（Phase 2）落 child org（organizations + intel.asset_intel_discovery），**不落 targets**——子 org 趟 target_intel 起步时 `in_scope_assets(child)` 为空 → gate 资产轴 fallback 自报（合法降级）；agent enrich 落 targets 后，下游 EAS/enum 的资产轴自动硬化。**这是 target_intel 的本职**（从公司名找资产），不是 blocker。
- `golish_db::repo::organizations::list(pool, project_path)` 已存在（hydrate.rs 在用），I2 项目隔离内置。
- 子趟共享同一 bridge/session：evidence 落同一 session；gate 账本 facts 按 (asset × technique) 匹配 + 资产轴按 org 过滤 → 跨 org 不互相投影（母的 moresec.cn 事实不会填子 org 的格）。SUBSIDIARY gate 只在母趟 scoping 激活（子趟传 include_subsidiaries=false，不递归发现孙公司——非目标）。
- sub_agent override bug（设计 §5 前置）已于 2026-06-12 修复并活体验证（`register_preserving_overrides`）。

---

## 1. 任务分解

### Task F · 纯函数（stage_run/mod.rs，全部可单测）

- **F.1 `child_slice(profile_id, to) -> Option<(StageKind, HashSet<StageKind>)>`**：子 org 趟的 slice。`to == Scoping` → `None`（--only scoping = 只建树，无子趟）；否则 `resolve_slice(profile_id, Some(TargetIntel), to)` 的 Ok → Some。测试：red_team profile 下 to=scoping → None；to=enumeration → Some(entry=target_intel, allowlist 含 target_intel..=enumeration 不含 scoping)。
- **F.2 `filter_child_orgs(orgs: Vec<Organization>, parent: Uuid) -> Vec<Organization>`**：`parent_id == Some(parent)` 过滤。测试：命中/不命中/parent_id None。
- **F.3 `build_child_objective(child: &Organization, parent_name: &str, to: StageKind) -> String`**：含子 org 真实 id（agent 直接调 recon_* 不用猜）、子公司语境（only THIS subsidiary）。测试：含 organization_id / 子名 / 母名。
- **F.4 `subsidiary_summary(results: &[(String, bool)]) -> String`**：per-org OK/FAILED 汇总段（空 → 空串）。测试：混合结果渲染。

### Task G · 编排接线（run() 第 6.5 步）

母趟 `result` 之后、报告之前：

```text
if args.include_subsidiaries
   && 母趟 Ok
   && seed.org_id = Some(parent)
   && let Some((child_entry, child_allowlist)) = child_slice(profile, to_stage):
    children = filter_child_orgs(organizations::list(db_pool, workspace_str)?, parent)
    for (i, child) in children:
        eprintln "[stage-run] ── subsidiary i+1/n: {name} ──"
        r = orchestrate(..., child_entry, child_allowlist.clone(),
                        &build_child_objective(...), Some(child.id),
                        /*include_subsidiaries=*/false, threshold)
        sub_results.push((child.name, r.is_ok()))
        (子趟 Err 不中断后续子趟——每个子 org 独立收集，最后聚合)
```

报告：`println!(subsidiary_summary(&sub_results))`；exit 聚合：母 Err → Err（不跑子趟）；任一子 Err → `Err(anyhow!("subsidiary stage runs failed: [names]"))`；全 Ok → Ok。

- 验证：`cargo check -p golish`、`nextest -p golish`（含 F 全部新测）、母趟行为零回归（不带 flag 时 6.5 步整段不进）。

### Task H · 门禁 + 状态

- `cargo fmt -p golish --check` / `clippy -p golish --all-targets -- -D warnings` / `nextest -p golish` 全绿。
- `agent-progress.md` + `feature_list.json` 更新。
- 活体（需 ENScan 凭据 + 真子公司母企 + 用户在场）：`--stage-run --profile red_team --to target_intel --include-subsidiaries --org <母> --target <根域名>` → 母 scoping（建树）+ 母 target_intel + 每个合格子各一趟 target_intel；报告出现 per-org 汇总；不带 flag 时行为与 Phase 2 前逐字节一致。

---

## 2. 决策记录

1. **不加新 CLI flag**：`--include-subsidiaries` 即 engagement 级「范围含子公司」语义（Phase 2 建树 + Phase 3 逐 org 收集），`--to scoping` 自然退化为只建树。
2. **串行**（设计风险①：provider 限流/规模），不做并发；不设数量上限（阈值 51% 已天然限量，截断会违反「漏 org = BLOCK」语义）。
3. **子趟不重跑 scoping**：授权确认 + org 树构建是 engagement 级动作，已在母趟完成；子趟 entry 固定 target_intel。
4. **子趟传 include_subsidiaries=false**：不递归发现孙公司（设计非目标），SUBSIDIARY gate 不在子趟激活。
5. **子趟失败不中断兄弟趟**：每个子 org 独立收集单元，全部跑完后聚合报告 + 统一 Err——一次跑完暴露全部缺口，而不是修一个跑一遍。

---

## 3. 红线对齐（AGENTS.md）

- **I2**：children 来自 `organizations::list(pool, project_path)`（项目隔离）+ parent_id 过滤；子趟 org_id 绑 gate 资产轴。
- **org 隔离不串**（总纲 §8）：方案 A 复用 06-10 隔离，跨 org 事实因 asset 不重叠不互相投影。
- **Phase 0 真值继承**：每子趟 coverage_truth/db_truth_facts 按子 org_id 查——found 权威性自动生效。
- **零回归**：不带 `--include-subsidiaries` 时 6.5 步整段跳过，单趟行为逐字节不变。
- **§2.7**：无 schema/migration、无远端推送；纯调度层。活体跑真实 ENScan API 时需用户在场。
