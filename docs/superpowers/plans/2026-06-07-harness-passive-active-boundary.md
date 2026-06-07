# harness 被动/主动阶段边界重构 实现计划

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现此计划；每个任务独立 commit。

**目标：** 把 harness 12 阶段管线的被动/主动边界改成单一判据「是否接触目标主机」——被动子域名枚举 + url-history 收归 `target_intel`（被动·零接触），`external_attack_surface`（EAS）专做接触目标的主动测绘并从 `target_intel` 继承子域名 evidence；复用现有 `active_scan` 审批门。
**架构：** 纯配置驱动（gate/graph/stage_spec 引擎均按 JSON 参数化，无需改 Rust 逻辑）。改 2 个 stage JSON + 1 处 orchestration charter + 同步既有单测。阶段顺序 / DAG 不变。
**技术栈：** Rust（golish-agent-kit，`cargo nextest`/`clippy`）+ JSON 资源（`resources/harness/stages/`）+ `just` 命令。

> 设计依据：`docs/design/2026-06-07-harness-passive-active-boundary.md`（已 commit `123bae72`）。

---

## 文件结构（创建/修改 + 职责）

| 文件 | 动作 | 职责 |
|---|---|---|
| `resources/harness/stages/external_attack_surface.json` | 修改 | 删被动子域名/url-history 工具类型 + 删 `subdomain_enum_passive` 硬地板 + 继承 target_intel 子域名 evidence |
| `resources/harness/stages/target_intel.json` | 修改 | 给 `min_invocations` 加 `subdomain_enum_passive:1`，把被动子域名设为本阶段硬地板（与从 EAS 删除对称）|
| `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs` | 修改（≈411-419）| 修正 orchestration charter：子域名枚举/url-history 归 target_intel；EAS 描述改为「对已发现/已批准 host 做主动测绘」。顺手修 charter↔spec 不一致 |
| `backend/crates/golish-agent-kit/src/harness/stage_spec.rs` | 修改（test ≈190）| 同步既有 `external_attack_surface_inherits_evidence_from_target_intel` 测试 + 新增 allowed_tool_types/min_invocations 断言 |

---

## Task 0：确认子域名 evidence_kind 名（设计 §9-1，唯一待核点）

**为什么：** EAS `inherits_evidence_from` 要加的子域名 evidence kind 字符串必须是「子域名枚举实际落账的 kind」。已知现有 inherit kinds（`dns_a`/`asn`/`whois`）是自由字符串（`asn` 都不在 `evidence_kinds.json` 老化注册表里），所以不能臆造。

**文件：** 只读排查，无改动。

**步骤：**
1. 查子域名枚举落账的 kind 候选：
   ```bash
   rg -n "subdomain|target_asset|ct_log|book.*evidence|append.*evidence|kind" \
     backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs \
     backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/
   ```
2. 查 ledger 实际写入点 kind 取值：
   ```bash
   rg -n "fn append|kind\s*:|evidence_kind" backend/crates/golish-pentest/src/evidence_ledger/append.rs
   ```
3. 对照 `resources/harness/evidence_kinds.json` 已注册 kinds（`dns_a/dns_aaaa/ct_log/whois/target_asset/...`）。
4. **决策规则**：
   - 若子域名枚举落账 kind = `"subdomain"` → 后续任务 inherit `"subdomain"`。
   - 若实际落的是 `"target_asset"`（Surface Workbench 资产）或 `"ct_log"`（CT 来源）→ inherit 该实际 kind。
   - 取到的确认值记为 `<SUBDOMAIN_EVIDENCE_KIND>`，写进 Task 2 / Task 4 的 JSON 与断言（默认假定 `"subdomain"`，与 output_parser `data_type:"subdomain"`、finding `kind:"subdomain"` 一致；若排查否定则替换）。

**验证：** 终端输出里能指认出子域名枚举写 evidence 时用的 kind 字面值；记录该值。

**提交：** 无（纯排查；结论用于后续任务）。

---

## Task 1：(RED) 在 stage_spec 测试里写下新 EAS / target_intel 形状

**文件：** `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`（`#[cfg(test)] mod tests`）

**步骤：**
1. 把既有 `external_attack_surface_inherits_evidence_from_target_intel`（≈190 行）补一条 subdomain 断言：
   ```rust
   #[test]
   fn external_attack_surface_inherits_evidence_from_target_intel() {
       let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
       assert_eq!(s.inherits_evidence_from.len(), 1);
       let inh = &s.inherits_evidence_from[0];
       assert_eq!(inh.stage_kind, StageKind::TargetIntel);
       assert!(inh.evidence_kinds.contains(&"dns_a".to_string()));
       assert!(inh.evidence_kinds.contains(&"asn".to_string()));
       assert!(inh.evidence_kinds.contains(&"whois".to_string()));
       // 重构新增：EAS 继承 target_intel 发现的子域名（host 来源）
       assert!(inh.evidence_kinds.contains(&"subdomain".to_string()));
   }
   ```
2. 新增一条 EAS 工具/地板收敛断言：
   ```rust
   #[test]
   fn external_attack_surface_is_target_touching_only() {
       let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
       // 被动子域名 / url-history 已下沉 target_intel：EAS 不再允许
       assert!(!s.allowed_tool_types.contains(&"recon/subdomain".to_string()));
       assert!(!s.allowed_tool_types.contains(&"recon/url-history".to_string()));
       // 接触目标的工具保留
       assert!(s.allowed_tool_types.contains(&"recon/http".to_string()));
       assert!(s.allowed_tool_types.contains(&"recon/visual".to_string()));
       assert!(s.allowed_tool_types.contains(&"recon/dns".to_string())); // 公共前置工具
       // 不再把被动子域名枚举钉为 EAS 硬地板
       assert!(!s.min_invocations.contains_key("subdomain_enum_passive"));
       assert!(s.min_invocations.contains_key("http_probe"));
   }
   ```
3. 新增一条 target_intel 拥有被动子域名的断言：
   ```rust
   #[test]
   fn target_intel_owns_passive_subdomain_and_url_history() {
       let s = load_stage_spec_from_json(TARGET_INTEL_JSON).expect("parse");
       assert!(s.allowed_tool_types.contains(&"recon/subdomain".to_string()));
       assert!(s.allowed_tool_types.contains(&"recon/url-history".to_string()));
       // 被动子域名设为本阶段硬地板
       assert!(s.min_invocations.contains_key("subdomain_enum_passive"));
   }
   ```
   若文件顶部尚无 `TARGET_INTEL_JSON` 常量，补：
   ```rust
   const TARGET_INTEL_JSON: &str =
       include_str!("../../../../../resources/harness/stages/target_intel.json");
   ```
   （`StageKind` 已在 `use super::*` 链上可用；若编译报缺，加 `use crate::harness::types::StageKind;`。）

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit \
  external_attack_surface_inherits_evidence_from_target_intel \
  external_attack_surface_is_target_touching_only \
  target_intel_owns_passive_subdomain_and_url_history
```
预期：**FAIL**（JSON 尚未改，断言不满足）。确认红。

**提交：**
```bash
git add backend/crates/golish-agent-kit/src/harness/stage_spec.rs
git commit -m "test(harness): assert touches-target boundary for EAS/target_intel stage specs"
```

---

## Task 2：(GREEN) 改 external_attack_surface.json

**文件：** `resources/harness/stages/external_attack_surface.json`

**步骤：**
1. `allowed_tool_types` 删 `recon/subdomain` + `recon/url-history`：
   ```json
   "allowed_tool_types": ["recon/dns", "recon/http", "recon/visual"],
   ```
2. `min_invocations` 删 `subdomain_enum_passive`：
   ```json
   "min_invocations": {
     "dns_resolve": 1,
     "http_probe": 1
   },
   ```
3. `inherits_evidence_from` 给 target_intel 加 `<SUBDOMAIN_EVIDENCE_KIND>`（Task 0 确认值，默认 `subdomain`）：
   ```json
   "inherits_evidence_from": [
     {
       "stage_kind": "target_intel",
       "evidence_kinds": ["dns_a", "asn", "whois", "subdomain"]
     }
   ]
   ```

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit \
  external_attack_surface_inherits_evidence_from_target_intel \
  external_attack_surface_is_target_touching_only
python3 -c "import json;json.load(open('../resources/harness/stages/external_attack_surface.json'))"
```
预期：两测 **PASS**；JSON 合法（无输出即合法）。

**提交：**
```bash
git add resources/harness/stages/external_attack_surface.json
git commit -m "feat(harness): make external_attack_surface target-touching only; inherit subdomains from target_intel"
```

---

## Task 3：(GREEN) 改 target_intel.json

**文件：** `resources/harness/stages/target_intel.json`

**步骤：**
1. `min_invocations` 加 `subdomain_enum_passive:1`（保留既有 `dns_resolve`）：
   ```json
   "min_invocations": {
     "dns_resolve": 1,
     "subdomain_enum_passive": 1
   },
   ```
   （`allowed_tool_types` 已含 `recon/subdomain`+`recon/url-history`、`expected_techniques` 已含 `GOLISH-INTEL-SUBDOMAIN`，无需改。）

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit target_intel_owns_passive_subdomain_and_url_history
python3 -c "import json;json.load(open('../resources/harness/stages/target_intel.json'))"
```
预期：测试 **PASS**；JSON 合法。

**提交：**
```bash
git add resources/harness/stages/target_intel.json
git commit -m "feat(harness): make passive subdomain enum a target_intel min-invocation floor"
```

---

## Task 4：修正 orchestration charter（charter↔spec 一致）

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs`（≈411-413）

**步骤：**
1. 把这两行：
   ```
   - `target_intel` — passive intel: whois, ASN, DNS records, registrant info. (情报收集)
   - `external_attack_surface` — passive + light-active external recon: subdomain enum (passive + CT logs), DNS resolution, HTTP probing, external port discovery. (资产测绘 / 攻击面 / 外部侦察)
   ```
   改为：
   ```
   - `target_intel` — passive intel (zero-touch): whois, ASN, DNS records, registrant info, passive subdomain enum (subfinder/amass -passive + CT logs), url-history (gau/waybackurls). (情报收集)
   - `external_attack_surface` — active external recon on already-discovered/approved hosts: DNS resolution, HTTP probing, fingerprinting, screenshots. Subdomains come from upstream target_intel (no enumeration here). (资产测绘 / 攻击面 / 外部侦察)
   ```
2. 全局搜其它把 subdomain 派给 EAS 的提示，确保无残留矛盾：
   ```bash
   rg -n "subdomain" backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs \
     backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs
   ```
   若 `execute.rs::synthesize_stage_subtask` 的 `ExternalAttackSurface` 分支仍指示「枚举子域名」，同样改为「对继承的 host 做 HTTP 探测」（与本任务 charter 一致）。

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit
cargo clippy -p golish-agent-kit --all-targets 2>&1 | rg -n "warning|error" || echo "clippy clean"
```
预期：golish-agent-kit 全部单测 **PASS**；clippy **0 warning**。
（若存在 charter 渲染断言测试，更新为：target_intel 文案含 "subdomain"、EAS 文案不含 "subdomain enum"。）

**提交：**
```bash
git add backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs
git commit -m "fix(harness): align stage charter with touches-target boundary (subdomain -> target_intel)"
```

---

## Task 5：全量收口 + 端到端复验

**文件：** 无（验证 + 文档收尾）。

**步骤：**
1. 后端定向 + 全量门禁：
   ```bash
   cd backend && cargo nextest run -p golish-agent-kit
   cd .. && just precommit
   ```
2. 端到端（小米 MiMo，复用既有 headless runner；真实 LLM/网络，按需）：
   ```bash
   just kill
   golish --stage-run --profile red_team --to external_attack_surface \
     --org 默安科技 --target moresec.cn --provider xiaomi --model mimo-v2.5-pro --auto-approve
   ```
   观察日志确认：
   - `target_intel` 阶段跑被动子域名枚举（recon/subdomain）+ 落 subdomain evidence；
   - 跨 `active_scan` 审批后进 `external_attack_surface`；
   - EAS **不再**自枚举子域名，而是从继承 evidence 拿 host 后 `http_probe`/截图。
3. 更新 bookkeeping：
   - `agent-progress.md` 追加会话记录（目标 / 改动 / 验证证据：nextest+clippy+precommit 输出片段 / 端到端日志关键行 / commit 列表）。
   - `feature_list.json` 视情况加/更新条目（如 harness 边界重构），状态按证据置 `passing`/`in_progress`。

**验证：** `just precommit` 全绿（命令 + 退出码 + 关键输出记进 progress）；端到端日志出现上述三条行为证据。

**提交：**
```bash
git add agent-progress.md feature_list.json
git commit -m "docs: record harness passive/active boundary refactor evidence"
```

---

## 自检

**1. 规格覆盖度**（对照设计 §3.2 改动清单）：
- EAS 删 recon/subdomain+url-history → Task 2 ✓
- EAS 删 min_invocations.subdomain_enum_passive → Task 2 ✓
- EAS 继承 subdomain evidence → Task 2（kind 由 Task 0 锁定）✓
- target_intel 加 subdomain_enum_passive 地板 → Task 3 ✓
- charter 修正（含 charter↔spec 不一致）→ Task 4 ✓
- 阶段顺序/DAG 不变 → 无任务（operation_graph.json 不动）✓
- 验证（单测/precommit/端到端）→ Task 5 ✓

**2. 占位符扫描：** 唯一「待确认」是 Task 0 的 `<SUBDOMAIN_EVIDENCE_KIND>`——这是带确定命令 + 决策规则的排查步骤（默认 `subdomain`），非空泛占位。其余步骤均含实际代码/命令。

**3. 类型一致性：** `inherits_evidence_from` / `evidence_kinds` / `allowed_tool_types` / `min_invocations` 字段名贯穿 Task 1-3 与现有 `stage_spec.rs::StageSpec` 一致；测试常量 `EXTERNAL_ATTACK_SURFACE_JSON` 既有、`TARGET_INTEL_JSON` 在 Task 1 补充。`StageKind::TargetIntel` 为既有枚举值。
