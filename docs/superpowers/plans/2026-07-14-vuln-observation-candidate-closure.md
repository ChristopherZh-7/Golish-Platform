# Vuln Observation → Candidate → Verification 实现计划

> Design: [`docs/design/2026-07-14-vuln-observation-candidate-closure.md`](../../design/2026-07-14-vuln-observation-candidate-closure.md)
> Status: in_progress
> Date: 2026-07-14

> **当前执行范围（用户恢复全闭环）**：执行 Task 1–9，包括匿名访问、Candidate
> manifest/context、精确 Verification replay、真实 evidence operation 身份、旧
> `auth_probe` 删除和指定公司的 CLI acceptance。按用户要求不运行 `init.sh`；最终门禁
> 使用 `just precommit`。

## 执行规则

- 不新增表或列；经用户明确授权，以更高版本 additive migration 更新 Candidate shadow
  rebuild function，历史 migration 不改写。
- 按 TDD 逐个 slice 观察 RED 后实现 GREEN。
- 不运行真实外部扫描；HTTP 只用 loopback fixture，Nuclei 执行用 fake/process seam。
- Nuclei template 解析与 pre-spawn 保护只用 tempfile/fake launcher 测试；不再用真实
  `nuclei -tl` 做本地模板探测。
- `vuln_triage` 零 Finding 写入；`attack_candidate` 零扫描工具；Verification 仍只接受 ordinal。
- 任一 malformed/truncated/authorization drift 都不得变成 checked-empty。

## Task 1：注册 feature 和契约红测

**Files**

- `feature_list.json`
- `agent-progress.md`
- `resources/harness/stages/vuln_triage/spec.json`
- `backend/crates/golish-agent-kit/src/harness/{stage_spec,stage_capability,tool_taxonomy,wstg_mapping}.rs`
- `backend/crates/golish-agent-kit/src/task_orchestrator/stage_refiner.rs`
- `backend/crates/golish-sub-agents/src/defaults/**`

**Steps**

1. 把等待 live acceptance 的原 Candidate epic 转 `blocked`，本功能设为唯一
   `in_progress`。
2. 先写测试要求 stage 显示三个具体 capability/tool，不暴露 raw
   `nuclei` / `pentest_run` / `auth_probe`。
3. 把公式化认证类从浅 IDOR `WSTG-ATHZ-04` 替换为匿名访问
   `WSTG-ATHN-04`，并保留 ATHZ 作为后续 Candidate/IDOR 能力。
4. 更新 methodology/prompt/refiner 的工具调用指引。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_capability stage_spec vuln --status-level fail
cd backend && cargo nextest run -p golish-sub-agents vuln_scanner defaults coverage_gap --status-level fail
```

## Task 2：可扩展 VulnAdapter registry 和 Nuclei typed parser

**Files**

- `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/{mod,nuclei}.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`

**Steps**

1. 先写 registry duplicate-id/dispatch 红测。
2. 定义 `VulnObservation` / `VulnAdapterReport` / parse completeness 契约。
3. 写 Nuclei JSONL parser 红测：foreign origin、unrequested template、malformed、
   truncated、clean-zero-hit、去重。
4. 实现 general/fingerprint-targeted adapter，新 adapter 可独立注册。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-pentest-app vuln_adapters nuclei_parser --status-level fail
```

## Task 3：fingerprint 安全 template selector

**Files**

- `backend/crates/golish-scan-runner/src/nuclei/{mod,poc_match}.rs`
- `backend/crates/golish-scan-runner/src/types.rs`
- `backend/crates/golish-pentest-app/Cargo.toml`

**Steps**

1. 先写红测：非 Nuclei PoC 过滤、unsafe id 拒绝、template-id 去重、
   current-owner fingerprint only。
2. 新增只读 selector：不 backfill target，不写 Finding，不启动进程。
3. 给 targeted adapter 返回 template id + fingerprint rationale；空集明确终止，
   不 fallback general。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-scan-runner poc_match fingerprint --status-level fail
```

## Task 4：guarded Nuclei 执行与 evidence-first landing

**Files**

- `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/evidence.rs`
- `backend/crates/golish-pentest-app/src/pentest_ai/run.rs` (only if a test seam is required)

**Steps**

1. 先写 fake-launch 红测：未绑 exact workspace/org/origin 不启动；owner drift 在
   fake spawn 前阻断。
2. 两个 Nuclei tool 只接受 `target_id + target_url`，通过 target resolver 获取
   `TargetAuthorizationSnapshot/TargetWriteGuard`。
3. 固定 foreground/safe flags，不接受 raw args/proxy/template path/output override。
4. 解析现有本地 template tree，proof/active 都显式绑定 `-t <canonical-dir> -duc`；
   targeted selector 空集在解析模板前直接 N/A，其他缺失/空/坏配置路径零进程失败。
5. 把同一 local-path witness 下沉到 runner 最后 async seam，与 target guard 一起在
   launch closure 前复验；共享 batch-placeholder 与 shell-quote helper，禁止复制规则。
6. 先写 typed redacted evidence，再以 guarded batch 写 outcome；任一 evidence 失败
   使整个 sibling group 保持 partial。
7. 工具结果显式禁用 generic structured storage/evidence。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-pentest-app vuln_nuclei guarded_vuln --status-level fail
```

## Task 5：匿名访问 capability

**Files**

- `backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/evidence.rs`

**Steps**

1. 先写 pure policy/verdict 红测：具体 GET/HEAD only，OPTIONS/mutating/template/
   dangerous/cross-origin 拒绝。
2. 实现 current-owner endpoint planner，endpoint id 只用作过滤，不允许 model
   提供 URL/method/header。
3. 实现 fresh no-cookie client、same-origin bounded redirect、64 KiB body cap 和脱敏
   response fingerprint。
4. 用 loopback fixture 验证 401/403/login redirect/health 200/sensitive 200/
   SPA fallback/foreign redirect，并验证请求无 auth/cookie。
5. 聚合语义：任一 suspicious→found；全部 controlled/public→empty；任一
   inconclusive/error→partial。evidence-first guarded landing。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-pentest-app anonymous_access --status-level fail
```

## Task 6：具体 observation 物化为 Candidate manifest

**Files**

- `backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`
- `backend/crates/golish-db/migrations/20260714000001_candidate_observation_shadow_hash.sql`
- `backend/crates/golish-db/tests/{attack_execution_v2_migrations,attack_rollout_cohort_migrations}.rs`
- `backend/crates/golish-agent-kit/src/harness/attack_execution/{types,decision,classifier,tests}.rs`
- `backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs`

**Steps**

1. 先写红测：negative 格不创建 lead work item；两个 Nuclei hit 不合并；
   observation drift 拒绝；manifest hash 覆盖 observation。
2. 从 exact handoff evidence 中读取新 typed observation batch；positive/suspicious
   每条生成一个 lead item。
3. 每个 target 生成一个 bounded `surface_analysis_v1` item，带 target id 和
   coverage summary/evidence，供 AI 调 `query_target_data`。
4. manifest DTO/DB bridge 新增 observation + hash，Rust canonical hash 与 DB whole-record
   shadow rebuild projection 同步扩展；用 additive function replacement migration 保持历史
   migration 不变，并同步两组手工 manifest fixtures。
5. `surface_analysis_v1` 允许 model 选择 registry-supported technique；具体 lead
   继续禁止 technique drift。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-db attack_candidate_work_items --status-level fail
cd backend && cargo nextest run -p golish-db \
  --test attack_execution_v2_migrations \
  --test attack_rollout_cohort_migrations \
  --status-level fail
cd backend && cargo nextest run -p golish-agent-kit attack_execution candidate --status-level fail
```

## Task 7：Candidate analyst 真正看到 manifest 和 scoped memory

**Files**

- `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{sub_agent_call,stage_run_call}.rs`
- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`

**Steps**

1. 先写红测：父 ctx 没 unit id 时，bound Worker 仍用自己的 operation/
   execution/unit/org/worker 身份取 ContextPack；foreign identity 失败。
2. ContextPack retrieval 接受 server-built identity override，不从 model args 取值，不
   fallback 全局 memory。
3. stage-run 保留每 org 的 exact manifest，provider dispatch 前追加 bounded
   frozen-manifest data block；过大直接 block。
4. prompt 要求对 `surface_analysis_v1` 先用 given target id 调
   `query_target_data`；prior hints 必须被当前 evidence 重验。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-agent-runtime scoped_context candidate_manifest stage_run_call --status-level fail
cd backend && cargo nextest run -p golish-sub-agents attack_analyst --status-level fail
```

## Task 8：冻结精确 Verification replay

**Files**

- `backend/crates/golish-agent-kit/src/harness/attack_execution/{classifier,decision,tests}.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs`

**Steps**

1. 先写红测：Nuclei observation 产生 exact template replay；anonymous observation
   产生 exact no-auth request replay；任意改 target/url/template 会改 hash 或被拒绝。
2. `CandidateClassificationInput` 新增 frozen observation。
3. classifier 生成 `verify.nuclei_template_replay` /
   `verify.anonymous_request_replay`，其他 item 继续用静态 registry。
4. verifier 仍只接 ordinal；Nuclei 固定 exact template-id，anonymous replay 固定
   GET/HEAD/no-auth/same-origin/body cap。
5. 保留现有 begin/finish action journal、approval/lease/workspace/target guard。
6. exact replay evidence 必须使用 trusted harness operation id；action journal、提交命令和
   Attempt result 必须绑定同一 evidence id/role。失败且 evidence 无法落账时允许 action
   终结为 blocker，但禁止 Attempt submit。

**RED/GREEN**

```bash
cd backend && cargo nextest run -p golish-agent-kit classifier candidate_plan --status-level fail
cd backend && cargo nextest run -p golish-pentest-app verification_capabilities anonymous_replay nuclei_replay --status-level fail
```

## Task 8A：删除 legacy `auth_probe`

**Files**

- `backend/crates/golish-auth-probe/**`
- `backend/crates/golish-pentest-app/src/pentest_bridge/auth_probe.rs`
- workspace/Cargo/registry/policy/prompt/taxonomy/ownership/module docs

**Steps**

1. 删除旧 crate 与 bridge wrapper，移除 workspace、组合根和 lockfile 依赖。
2. 移除 Chat/sub-agent 注册、策略、prompt、taxonomy/refiner 和 DAG/ownership 条目。
3. 删除仅由旧 wrapper 使用的 Vault helper；保留新 `vuln_probe_anonymous_access` 为唯一匿名链路。
4. 全仓静态搜索证明运行代码、前端和资源中无 `auth_probe` 标识。

**Verification**

```bash
rg -n -i 'auth[_-]?probe|golish-auth-probe' backend frontend resources scripts justfile
python3 scripts/check_dag.py
```

## Task 9：闭环集成与文档收口

**Files**

- `backend/crates/golish-agent-app/tests/**` (new focused integration test if DB fixture permits)
- `resources/harness/stages/{vuln_triage,attack_candidate,verification}/methodology.md`
- `docs/modules/backend/{golish-scan-runner,golish-pentest-app,golish-agent-kit,golish-agent-runtime,golish-memory-app,golish-db}.md`
- `docs/modules/backend/golish-db/repo.md`
- `docs/modules/INDEX.md`
- `agent-progress.md`
- `feature_list.json`

**Steps**

1. 覆盖 `observation evidence → handoff → seed → manifest → plan → ordinal replay`
   的不访外网集成测试。
2. 运行相关 package 测试、Clippy、fmt 和 JSON validation。
3. 运行 `just precommit`（不是 `init.sh`）。
4. 更新 module cards/index/progress/feature evidence。如未进行真实授权 live scan，
   明确区分“代码闭环已验证”和“live acceptance 未做”。
5. 以“广州有创网络科技有限公司”执行 Scoping→Vuln→Candidate→Verification；锁定该次
   session 后逐项核对 `run.log`、`transcript.json`、`run_tree.py --full --db` 和 DB rows。

**Final verification**

```bash
cd backend && cargo nextest run \
  -p golish-scan-runner \
  -p golish-pentest-app \
  -p golish-agent-kit \
  -p golish-agent-runtime \
  -p golish-agent-app \
  -p golish-sub-agents \
  --status-level fail
cd backend && cargo clippy \
  -p golish-scan-runner \
  -p golish-pentest-app \
  -p golish-agent-kit \
  -p golish-agent-runtime \
  -p golish-agent-app \
  -p golish-sub-agents \
  --all-targets -- -D warnings
just precommit
git diff --check
jq empty feature_list.json
```
