# Target Intel Goal Loop Fixture/Dev Shadow 实现计划

> Superseded by `2026-08-04-scoping-and-autonomous-corporate-asset-discovery.md`; its semantic executor is promoted to the production architecture instead of remaining a legacy shadow.

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在不改数据库 schema、`golish-db`、generated IPC、生产 Red Team profile 和下游阶段的前提下，先在 fixture/dev-only 环境验证 Target Intel Goal、semantic pivot、动态通用 SubAgent 和独立三态审计。生产 Red Team、Pentest、历史 operation 和旧六轴 Gate 的行为全部保持不变。

**架构：** 复用现有 StageTeam WorkItem、evidence ledger、`source_query_log` 和 asset-intel landing，但 Plan A 的运行选择只能来自显式 fixture/dev context。Audit receipt 是 candidate/frontier 的唯一可恢复 source-of-truth；`expansion_queue` 只是 best-effort mirror，Goal、reviewer、duplicate guard 和 promotion 报告均不得读取它作 authority。Shadow reviewer 只观察并落账 `PASS|REWORK|NEEDS_HUMAN`；任何 verdict 都不得恢复 Controller、打开 request epoch、写 hold、写 pass token或改变 legacy Gate路径。

**技术栈：** Rust 2021、rig-core 0.36、sqlx/PostgreSQL、serde/serde_json、现有 Golish StageTeam runtime、`golish-intel-providers`、`golish-recon-app`、Python 3 `scripts/run_tree.py`。

**设计依据：** [`../../design/2026-08-02-target-intel-goal-loop-and-audit.md`](../../design/2026-08-02-target-intel-goal-loop-and-audit.md)

**实施授权边界：** 本计划不创建或修改 migration，不修改 `backend/crates/golish-db/`，不修改 `frontend/lib/generated/`，不修改生产 `resources/harness/profiles/red_team.json` / `pentest.json` / Target Intel production spec，不切换 `intel_goal_v1` authority，不访问真实 provider/网站/目标。若实现需要其中任一项，立即停止并请求新授权。所有 Cargo 命令前先运行 `just space-guard`；不运行未获授权的 `init.sh`、`just precommit`、全 workspace 或全量前端测试。

## 0. 审阅修订：本节优先级最高

本节覆盖下文任何遗留冲突；实现者遇到旧文字与本节不一致时，必须以本节为准并先修订计划，不能折中执行。

| 场景 | Plan A 行为 |
|---|---|
| production Red Team / Pentest | 完全 legacy；profile、methodology、tool surface、Gate 和 final seal不变 |
| 历史或正在运行的 operation | 不重解释、不注入 Goal/reviewer、不改变 resume |
| fixture/dev shadow | 可构造 Goal、worker、semantic provider、public-source 和 reviewer，但只消费 fake transport / fixture page |
| reviewer PASS | 只写 observed evidence；不产生 pass token，legacy fixture Gate照常运行 |
| reviewer REWORK | 只写 advisory finding；不恢复 chain、不派工、不重跑 |
| reviewer NEEDS_HUMAN | 只写 advisory requirement；不写 hold、不暂停 stage |
| advisory rework capability | 仅纯函数/fixture测试；Plan A runtime selector恒为 disabled |

不可妥协的不变量：

1. `IntelPivotReceiptV1` 的 organization、operation/session、pivot、query plan、artifact、observation、disposition、landing refs 和 capability gap 组成 frontier authority；应用层通过现有 `audit_log` 做 exact org/run/kind 只读投影，不新增表、不改 `golish-db`。
2. Semantic provider先 collect，再由 server-owned `ProjectionAuthorization` 分类，最后才允许 profile/Target/DNS/target_asset projection。Exact domain只授权自身；只有用户 wildcard的 strict child和用户 CIDR内 IP才可继承授权。Candidate、asset-intel profile、证书、ASN、共享 IP 和 reviewer verdict均不能授予 scope。
3. 生产 legacy `recon_map_assets` 保持原实现语义。新 `recon_search_intel` 只能在 fixture/dev bound context注册；不能以 alias方式全局改写旧工具。
4. Provider/public-source结果必须先保存 redacted artifact并 append evidence，成功后才可交给模型。Evidence失败返回 retryable error且不暴露内容。
5. Provider server-side web search在 Target Intel fixture context必须禁用；所有搜索/抓取统一走 host-owned adapter。首版只启用 `strict_passive`；`public_web_readonly` 是显式 disabled capability，不得静默降级。
6. Generic worker和reviewer使用 host-owned neutral prompt，不加载固定 recon/六轴 prompt。现有 immutable `kind + output_schema + input_refs` 只作 fixture技术 discriminator；exact dynamic prompt同时写 input refs 与 audit receipt，不能只存 hash。
7. 任一 material pivot缺 adapter、credential、artifact或 terminal receipt时，`promotion_eligible=false`。Unsupported是 terminal capability事实但绝不是 empty。

---

## 实施前状态

开始 Task 1 前：

1. 完整读取 `AGENTS.md`、`agent-progress.md`、`feature_list.json`、本设计、本计划和以下模块卡：
   - `docs/modules/backend/golish-agent-kit/harness.md`
   - `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
   - `docs/modules/backend/golish-sub-agents/executor.md`
   - `docs/modules/backend/golish-recon-app/agent_tools.md`
   - `docs/modules/backend/golish-recon-app/asset_intel.md`
   - `docs/modules/backend/golish-db/repo.md`
   - `docs/modules/backend/golish/stage_run.md`
2. 确认 `feature_list.json` 中只有零或一个 `in_progress`。只有没有其它 active feature 时，才把 `target-intel-goal-loop-shadow-2026-08-02` 改为 `in_progress`。
3. 在 `agent-progress.md` 顶部新建实施会话记录，写明不改 schema、不调用外部服务的边界和本计划的 focused verification。
4. 记录开工 `git status --short`；共享 dirty tree中的用户改动全部保留。每个 Task只精确暂存本 Task文件，不使用 `git add -A`。

---

## Task 1：建立 fixture/dev-only Goal Contract 与生产负向门禁

**Files:**

- Add: `backend/crates/golish-agent-runtime/src/eval_support/target_intel_goal_shadow.rs`
- Modify: `backend/crates/golish-agent-runtime/src/eval_support/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/eval_support/types.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`
- Test: inline tests in the same files
- Verify unchanged: `resources/harness/profiles/red_team.json`, `resources/harness/profiles/pentest.json`, `resources/harness/stages/target_intel/spec.json`, `resources/harness/stages/target_intel/methodology.md`

### Step 1：先写失败测试

新增：

```rust
#[test]
fn production_profiles_do_not_enable_target_intel_goal_shadow() {
    assert!(!production_profile_enables_goal_shadow("red_team"));
    assert!(!production_profile_enables_goal_shadow("pentest"));
}

#[test]
fn fixture_context_is_the_only_shadow_selector() {
    let fixture = TargetIntelGoalShadowFixture::strict_passive();
    assert_eq!(fixture.mode, GoalShadowMode::ObserveOnly);
    assert!(!fixture.advisory_rework_enabled);
    assert!(fixture.external_transport.is_fake());
}

#[test]
fn existing_operation_context_cannot_be_reinterpreted() {
    assert!(select_goal_shadow(&production_context()).is_none());
    assert!(select_goal_shadow(&fixture_context()).is_some());
}
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -E 'test(production_profiles_do_not_enable_target_intel_goal_shadow) | test(fixture_context_is_the_only_shadow_selector) | test(existing_operation_context_cannot_be_reinterpreted)' --status-level fail
```

预期：fixture contract/selector尚不存在。

### Step 3：实现最小 fixture contract

只在 eval/fixture context中构造：

```rust
#[derive(Debug, Clone)]
pub struct TargetIntelGoalShadowFixture {
    pub mode: GoalShadowMode,              // ObserveOnly
    pub contract_version: &'static str,    // target_intel_goal.fixture.v1
    pub review_schema: &'static str,       // intel_review.v1
    pub browser_mode: &'static str,        // strict_passive
    pub advisory_rework_enabled: bool,     // always false outside pure tests
    pub capability_manifest: CapabilityManifest,
    pub external_transport: FakeTransport,
}
```

规则：

- production constructors永远传 `None`；只有 eval fixture显式注入；
- 不从 profile、环境变量、CLI自由文本或历史 operation推断启用；
- fixture contract包含 Outcome、Constraints、Verification、provider/public capability matrix和 `promotion_eligible` 初始状态；
- `public_web_readonly` 在 manifest中为 `disabled_not_in_plan_a`；
- 生产 spec/methodology完全不改；fixture neutral prompt由后续 Task 7构造；
- runtime event一律标 `fixture_dev_only=true`、`shadow_observe_only=true`。

### Step 4：运行 GREEN 与格式检查

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -E 'test(production_profiles_do_not_enable_target_intel_goal_shadow) | test(fixture_context_is_the_only_shadow_selector) | test(existing_operation_context_cannot_be_reinterpreted)' --status-level fail
cargo fmt -p golish-agent-runtime -- --check
cd ..
git diff --exit-code -- resources/harness/profiles/red_team.json resources/harness/profiles/pentest.json resources/harness/stages/target_intel/spec.json resources/harness/stages/target_intel/methodology.md
```

### Step 5：提交

```bash
git add backend/crates/golish-agent-runtime/src/eval_support/target_intel_goal_shadow.rs backend/crates/golish-agent-runtime/src/eval_support/mod.rs backend/crates/golish-agent-runtime/src/eval_support/types.rs backend/crates/golish-agent-runtime/src/agentic_loop/context.rs
git commit -m "test(intel): add fixture-only goal shadow contract"
```

---

## Task 2：定义非 TS Semantic Pivot、内部执行请求与安全 Provider Matrix

**Files:**

- Modify: `backend/crates/golish-pentest-domain/src/models/asset_intel.rs`
- Modify if export needed: `backend/crates/golish-pentest-domain/src/models/mod.rs`
- Modify: `backend/crates/golish-intel-providers/src/types.rs`
- Modify: `backend/crates/golish-intel-providers/src/fofa/mod.rs`
- Modify: `backend/crates/golish-intel-providers/src/hunter/mod.rs`
- Modify: `backend/crates/golish-intel-providers/src/shodan/mod.rs`
- Modify: `backend/crates/golish-intel-providers/src/quake/mod.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/types.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/template.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs`
- Modify: `resources/intel-providers/fofa.json`
- Modify: `resources/intel-providers/hunter.json`
- Modify: `resources/intel-providers/shodan.json`
- Modify: `resources/intel-providers/quake.json`
- Modify: `resources/intel-providers/0-zone.json`
- Test: inline tests in the same files

### Step 1：写 domain contract RED tests

```rust
#[test]
fn semantic_pivot_canonicalizes_without_accepting_provider_dsl() {
    let pivot = AssetIntelPivot::parse(AssetIntelPivotKind::Domain, " ExAmPle.COM. ").unwrap();
    assert_eq!(pivot.value, "example.com");
    assert!(AssetIntelPivot::parse(AssetIntelPivotKind::Domain, r#"domain=\"example.com\""#).is_err());
}

#[test]
fn native_query_plan_maps_semantics_without_model_selected_provider() {
    let plan = NativePivotPlanner::plan(&domain_pivot("example.com"), IntelSearchIntent::VerifyAttribution, &capabilities());
    assert!(plan.iter().all(|query| !query.wire_query.is_empty()));
    assert!(plan.iter().all(|query| query.semantic_pivot.kind == AssetIntelPivotKind::Domain));
}

#[test]
fn unsupported_pivot_is_blocked_instead_of_falling_back_to_site() {
    let result = NativePivotPlanner::plan(&asn_pivot("AS13335"), IntelSearchIntent::DiscoverRelatedAssets, &domain_only_capabilities());
    assert_eq!(result.unwrap_err().code(), "INTEL_PIVOT_UNSUPPORTED");
}

#[test]
fn semantic_execution_request_is_internal_and_does_not_change_hydrate_ts() {
    assert!(!std::any::type_name::<AssetIntelExecutionRequest>().contains("TS"));
    assert_eq!(hydrate_config_ts_snapshot(), include_str!("../../../../../frontend/lib/generated/AssetIntelHydrateConfig.ts"));
}

#[test]
fn every_material_pivot_has_an_explicit_adapter_or_blocks_promotion() {
    for kind in all_pivot_kinds() {
        let capability = fixture_capability_matrix().get(kind).unwrap();
        assert!(capability.executable() || !capability.promotion_eligible());
    }
}

#[test]
fn provider_literal_compilers_escape_quotes_slashes_and_operators() {
    assert_safe_literal_for_all_providers(r#"Acme\" OR domain=\"evil.test"#);
}
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app -E 'test(semantic_pivot_) | test(native_query_plan_) | test(unsupported_pivot_)' --status-level fail
```

预期：缺少 pivot/intention/planner类型。

### Step 3：实现 typed contract

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetIntelPivotKind {
    CompanyName,
    Brand,
    Domain,
    Hostname,
    Ip,
    Cidr,
    Asn,
    Certificate,
    Icp,
    EmailDomain,
    GithubOrg,
    Repository,
    AppId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelSearchIntent {
    DiscoverRelatedAssets,
    VerifyAttribution,
    EnrichKnownAsset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetIntelPivot {
    pub kind: AssetIntelPivotKind,
    pub value: String,
}
```

安全规则：

- domain/hostname/email domain/IP/CIDR/ASN分别走 canonical parser；company/brand/ICP/GitHub/repo/app id做 Unicode trim、长度和控制字符限制；
- 拒绝明显 DSL token、NUL/newline、超长值和 URL userinfo；
- semantic kind 与 provider `QueryType` 分离；只由宿主 planner映射；
- 保持 exported `AssetIntelHydrateConfig` 完全不变；新增不 derive `ts_rs::TS` 的内部 `AssetIntelExecutionRequest { legacy_config, pivot, intent, projection_authorization, fixture_context }`；
- `template.rs` 接收 server-owned typed bindings，不能让模型值直接做字符串替换或 raw DSL pass-through；
- 每个 native/HTTP descriptor显式声明 `applicable_pivot_kinds`、wire query type、adapter version和 literal encoder；资源缺 metadata时 fixture loader fail closed；
- FOFA/Hunter/Shodan/Quake分别实现 literal encoder，覆盖引号、反斜杠、冒号、等号、布尔操作符和 Unicode；不得因输入包含 `=` / `:` 就 pass-through；
- Matrix至少覆盖 company_name、brand、domain、hostname、ip、cidr、asn、certificate、icp、email_domain、github_org、repository、app_id。IP/CIDR/ASN/certificate/ICP/email/GitHub/repo/app的所有新发现均为 candidate-only；GitHub/repo路由到 Task 4 的 host public adapter；
- 不能安全实现的 material kind返回 `unsupported` terminal receipt并设置 `promotion_eligible=false`，不得 fallback到 broad `Site`；
- HTTP runtime只看显式 metadata，不再以 template是否含 `{{domain}}` 猜适用性；
- Hunter/Shodan observation必须显式绑定 planned query receipt id/requested semantic kind，不能用 mapper返回的 `Site` 反推调用意图。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app -E 'test(semantic_pivot_) | test(native_query_plan_) | test(unsupported_pivot_) | test(query_type)' --status-level fail
cargo clippy -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app --lib --tests -- -D warnings
cargo fmt -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app -- --check
cd ..
jq empty resources/intel-providers/fofa.json resources/intel-providers/hunter.json resources/intel-providers/shodan.json resources/intel-providers/quake.json resources/intel-providers/0-zone.json
git diff --exit-code -- frontend/lib/generated
```

### Step 5：提交

```bash
git add backend/crates/golish-pentest-domain/src/models/asset_intel.rs backend/crates/golish-pentest-domain/src/models/mod.rs backend/crates/golish-intel-providers/src/types.rs backend/crates/golish-intel-providers/src/fofa/mod.rs backend/crates/golish-intel-providers/src/hunter/mod.rs backend/crates/golish-intel-providers/src/shodan/mod.rs backend/crates/golish-intel-providers/src/quake/mod.rs backend/crates/golish-recon-app/src/asset_intel/types.rs backend/crates/golish-recon-app/src/asset_intel/template.rs backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs resources/intel-providers/fofa.json resources/intel-providers/hunter.json resources/intel-providers/shodan.json resources/intel-providers/quake.json resources/intel-providers/0-zone.json
git commit -m "feat(intel): add semantic pivot planner"
```

---

## Task 3：实现 Collect → Authorize → Project 与权威 Audit Receipt

**Files:**

- Add: `backend/crates/golish-recon-app/src/asset_intel/authority.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/mod.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/service/hydrate.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/landing.rs`
- Modify: `backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs`
- Modify: `backend/crates/golish-recon-app/src/agent_tools/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`
- Modify: `backend/crates/golish-agent-runtime/src/execution_mode/policy.rs`
- Modify: `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`
- Modify: `backend/crates/golish/src/pentest_tool_factory.rs`
- Test: inline tests and existing fake-repository tests in those files

### Step 1：写 schema、authority、ordering 与 receipt RED tests

```rust
#[test]
fn recon_search_intel_schema_is_closed_at_every_object_level() {
    let schema = recon_search_intel_parameters();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["pivot"]["additionalProperties"], false);
    assert!(validate(&schema, json!({"organization_id": org(), "pivot": {"kind": "domain", "value": "example.com", "dsl": "x"}, "intent": "verify"})).is_err());
}

#[test]
fn exact_domain_authorizes_only_itself_not_children() {
    let auth = ProjectionAuthorization::from_fixture_scope(exact_domain("example.com"));
    assert!(auth.allows_domain("example.com"));
    assert!(!auth.allows_domain("app.example.com"));
}

#[test]
fn only_explicit_wildcard_and_cidr_authorize_descendants() {
    assert!(ProjectionAuthorization::from_fixture_scope(wildcard("*.example.com")).allows_domain("app.example.com"));
    assert!(!ProjectionAuthorization::from_fixture_scope(wildcard("*.example.com")).allows_domain("example.com"));
    assert!(ProjectionAuthorization::from_fixture_scope(cidr("203.0.113.0/24")).allows_ip("203.0.113.8"));
}

#[test]
fn collection_cannot_write_profile_or_landing_before_projection_authorization() {
    let collected = collect_fixture_shared_certificate();
    assert_eq!(collected.profile_writes(), 0);
    assert_eq!(project(&exact_domain("example.com"), collected).candidate_only.len(), 2);
}

#[test]
fn native_raw_payload_is_redacted_artifact_before_model_visibility() {
    let receipt = collect_native_fixture_with_secret().unwrap();
    assert!(!receipt.raw_artifact_sha256.is_empty());
    assert!(!receipt.redacted_artifact.contains("api_key"));
}

#[test]
fn unsupported_is_terminal_but_never_reported_as_empty() { /* status + reason + capability */ }

#[test]
fn audit_receipt_not_expansion_queue_is_frontier_source_of_truth() { /* exact org/run/kind */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-recon-app -p golish-agent-runtime -p golish-agent-app -E 'test(recon_search_intel_) | test(exact_domain_authorizes_) | test(only_explicit_wildcard_) | test(collection_cannot_write_) | test(native_raw_payload_) | test(unsupported_is_terminal_) | test(audit_receipt_not_expansion_queue_)' --status-level fail
```

### Step 3：实现 fixture-only tool 和 server-owned authority

模型可见 schema只有 `organization_id`、闭合的 `pivot {kind,value}` 和 `intent`；顶层与所有嵌套对象必须 `additionalProperties: false`，runtime validator也拒绝未知字段。实现顺序固定：

1. 用 Task 2 的内部 `AssetIntelExecutionRequest` 和 fixture transport **collect**，本阶段禁止写 organization profile、Target、DNS 或 `target_assets`；
2. 用 operation-frozen fixture scope构造 server-owned `ProjectionAuthorization` 并分类；
3. 最后才 **project** authorized exact refs；其余写 candidate/audit receipt，不能改变 active scope。

Authority规则：

- exact domain只授权 exact value，绝不隐式授权 child；
- 只有用户提供的 `*.example.com` 才授权 strict child，且不授权 apex；
- 只有用户提供的 CIDR才授权其中 IP；
- company/brand、当前候选、organization profile、certificate、ASN、shared IP、provider结果、review verdict都不授予 projection authority；
- certificate/ASN/ICP/email/GitHub org/repository/app identifier在 Plan A一律 candidate-only；它们可继续派生 typed pivot，但不能直接写 active Target。

兼容边界：

- 新 `recon_search_intel` 只在 Task 1 的 fixture/dev context注册；
- production及历史 operation继续走原 `recon_map_assets` 路径，函数、schema和写入顺序不在本计划重解释；
- 不以“把 legacy调用内部改写为新 semantic pivot”实现兼容，避免改变旧行为；
- `PassiveIntelSummary` 返回 bounded exact `pivot_ref/query_receipts/artifact_refs/landed_refs/discovered_pivots`；完整集合用 count+hash留在宿主 receipt。

### Step 4：实现 receipt authority、raw artifact 和 duplicate terminal

- 每次 planned query先生成稳定 key：`pivot:v1:<kind>:<value_sha256>:<adapter_version>:<provider>:<query_type>:<config_hash>`；
- native与 HTTP都必须先保存 redacted raw artifact，再保存 parsed observations；原始凭证、header、cookie、provider token不得进入 artifact或模型；
- evidence append成功后，写不可变 `audit_log` kind=`intel.semantic_pivot_receipt.v1`，包含 exact operation/org/session/pivot/key/status/artifact/evidence/landing/candidate refs；
- `golish-agent-app/src/ai/db_bridge/evidence.rs` 通过现有 repo seam按 exact operation+organization+session+kind读取这些 receipt；不改 `golish-db`，不新增 migration；
- `expansion_queue` 只允许 best-effort mirror，namespace必须含 `fixture:<session>:org:<uuid>`；任何 frontier、duplicate guard、review bundle或promotion逻辑都不得读取它；
- terminal状态明确为 `succeeded | empty | blocked | unsupported`。`unsupported` 必须包含 capability/reason且不得映射成 `empty`；
- receipt/evidence持久化失败时不向模型返回 provider内容、不写 terminal marker，调用保持 retryable；
- 当前 fixture外的既有 duplicate逻辑保持不变。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-recon-app -E 'test(recon_search_intel_) | test(exact_domain_authorizes_) | test(only_explicit_wildcard_) | test(collection_cannot_write_) | test(native_raw_payload_) | test(unsupported_is_terminal_) | test(landing)' --status-level fail
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-agent-app -E 'test(audit_receipt_not_expansion_queue_) | test(source_query_rows_are_keyed_by_semantic_pivot) | test(duplicate_guard_does_not_collapse_sibling_pivot_kinds) | test(receipt_persistence_failure_remains_retryable) | test(legacy_recon_map_assets_is_unchanged_outside_fixture)' --status-level fail
cargo clippy -p golish-recon-app -p golish-agent-runtime -p golish-agent-app --lib --tests -- -D warnings
cargo fmt -p golish-recon-app -p golish-agent-runtime -p golish-agent-app -- --check
git diff --exit-code -- backend/crates/golish-db frontend/lib/generated
```

### Step 6：提交

```bash
git add backend/crates/golish-recon-app/src/asset_intel/authority.rs backend/crates/golish-recon-app/src/asset_intel/mod.rs backend/crates/golish-recon-app/src/asset_intel/service/hydrate.rs backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs backend/crates/golish-recon-app/src/asset_intel/landing.rs backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs backend/crates/golish-recon-app/src/agent_tools/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs backend/crates/golish-agent-runtime/src/execution_mode/policy.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs backend/crates/golish/src/pentest_tool_factory.rs
git commit -m "feat(intel): add authorized fixture projection receipts"
```

---

## Task 4：统一 Root/SubAgent Host-owned Public Evidence Adapter

**Files:**

- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs`
- Modify: `backend/crates/golish-agent-kit/src/tool_provider_impl.rs`
- Modify: `backend/crates/golish-agent-kit/src/tool_executors/web.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor_types.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- Test: inline tests in those modules

### Step 1：写 provider-search、双执行路径和 SSRF RED tests

```rust
#[test]
fn target_intel_fixture_disables_model_provider_server_search() {
    let request = build_fixture_llm_request();
    assert_eq!(request.server_side_web_search_count(), 0);
}

#[test]
fn root_and_subagent_receive_only_host_owned_intel_public_tools() {
    for tools in [root_fixture_tools(), subagent_fixture_tools()] {
        assert!(tools.contains("intel_public_search"));
        assert!(tools.contains("intel_public_fetch"));
        assert!(!tools.contains("web_search"));
        assert!(!tools.contains("web_fetch"));
    }
}

#[test]
fn result_is_not_model_visible_when_evidence_append_fails() { /* both root and ToolProvider */ }

#[test]
fn fetch_revalidates_every_redirect_and_all_resolved_addresses() { /* mixed public/private DNS */ }

#[test]
fn dns_rebinding_after_validation_is_rejected() { /* connect address must be pinned */ }

#[test]
fn public_web_readonly_is_explicitly_disabled_in_plan_a() { /* typed unsupported receipt */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-agent-kit -p golish-sub-agents -E 'test(target_intel_fixture_disables_) | test(root_and_subagent_receive_) | test(result_is_not_model_visible_) | test(fetch_revalidates_) | test(dns_rebinding_) | test(public_web_readonly_)' --status-level fail
```

### Step 3：先封死 server-side/bypass 路径

- Task 1 fixture context建立 host-owned布尔值 `allow_provider_server_web_search=false`；`llm_stream_start.rs` 在构造模型请求前强制移除 provider server search工具；
- `stream_processor/mod.rs` 不得把 server-side search结果直接作为可见 content绕过本地工具执行和 evidence append；
- root runtime和 `golish-agent-kit::ToolProvider` 统一只暴露 `intel_public_search` / `intel_public_fetch`；旧 `web_search` / `web_fetch` 在 fixture context均不可见，fixture外行为不改；
- model provider若仍返回 server-search item，fixture runtime以 policy violation拒绝该 item并写 blocked receipt，绝不作为“non-authoritative可参考内容”塞给模型；
- Pentest hard-skip在创建 adapter、HTTP client或provider request前返回。

### Step 4：实现 `strict_passive` fixture transport 与 evidence-before-model

Plan A只实现 fake transport下的 `strict_passive`：

- search/fetch输入、最终 URL、每跳 redirect、method、status、timestamp、MIME、bounded body、content hash写 audit/evidence receipt；
- evidence和 receipt均 append成功之后，root或SubAgent才收到 untrusted-data envelope；任一持久化失败只返回 typed error，不泄漏 result body；
- 只允许 GET/HEAD；禁止 userinfo、非 HTTP(S)、credential/cookie forwarding、文件下载、目标自有 host和非公开地址；
- DNS解析后检查**全部**地址，拒绝 loopback/private/link-local/multicast/unspecified/metadata；连接时 pin已验证地址；
- 每个 redirect重新解析、重新做全地址检查和 target-owned检查，并设置小的固定 hop上限；
- fake transport提供 redirect、DNS rebinding、mixed-address、oversized body和evidence failure fixtures，测试不得访问互联网。

`public_web_readonly` 在 Plan A不做半成品网络实现：fixture capability matrix固定为 `disabled`，请求时写 `unsupported(capability=public_web_readonly)` receipt并使 promotion report为 false。若未来启用，必须另立设计并重新审计真实 transport的 SSRF、redirect与DNS rebinding防护。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-agent-kit -p golish-sub-agents -E 'test(target_intel_fixture_disables_) | test(root_and_subagent_receive_) | test(result_is_not_model_visible_) | test(fetch_revalidates_) | test(dns_rebinding_) | test(public_web_readonly_) | test(pentest_target_intel_rejects_provider_tools_without_dispatch)' --status-level fail
cargo clippy -p golish-agent-runtime -p golish-agent-kit -p golish-sub-agents --lib --tests -- -D warnings
cargo fmt -p golish-agent-runtime -p golish-agent-kit -p golish-sub-agents -- --check
```

### Step 6：提交

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-runtime/src/agentic_loop/llm_stream_start.rs backend/crates/golish-agent-runtime/src/agentic_loop/stream_processor/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs backend/crates/golish-agent-kit/src/tool_provider_impl.rs backend/crates/golish-agent-kit/src/tool_executors/web.rs backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs
git commit -m "feat(intel): evidence fixture public lookups"
```

---

## Task 5：先落通用 Worker/Reviewer Primitives 与动态派工

**Files:**

- Modify: `backend/crates/golish-sub-agents/src/executor_types.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Test: inline tests in the same modules

### Step 1：写闭合 schema、neutral prompt 与 reviewer primitive RED tests

```rust
#[test]
fn target_intel_spawn_schema_is_closed_and_exposes_no_role_taxonomy() {
    let schema = target_intel_spawn_subagents_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["agents"]["items"]["additionalProperties"], false);
    let item = &schema["properties"]["agents"]["items"]["properties"];
    for required in ["name", "prompt", "subject_refs"] {
        assert!(item.get(required).is_some());
    }
    for forbidden in ["role", "kind", "allowed_tools", "execution_profile", "terminal_contract"] {
        assert!(item.get(forbidden).is_none());
    }
}

#[test]
fn target_intel_dynamic_task_is_server_stamped_to_generic_recon_executor() {
    let request = dynamic_task(
        "核对域名归属",
        "围绕 example.com 寻找独立归属证据并返回 exact refs 与矛盾。",
        vec!["pivot:domain:example.com"],
    );
    let persisted = adapt_target_intel_task(&bound_leader(), request).unwrap();
    assert_eq!(persisted.requested_role, "intel_provider");
    assert_eq!(persisted.requested_kind, "provider_followup");
    assert_eq!(persisted.display_name, "核对域名归属");
}

#[test]
fn target_intel_dynamic_task_dedupes_exact_name_prompt_subjects_only() { /* exact hash */ }

#[test]
fn generic_worker_prompt_is_host_owned_and_contains_no_fixed_recon_six_axis_methodology() {
    let prompt = render_neutral_worker_prompt(&fixture_task()).unwrap();
    assert!(prompt.contains("server-owned constraints"));
    assert!(!prompt.contains("six coverage axes"));
}

#[test]
fn exact_worker_and_reviewer_prompts_are_auditable_not_hash_only() {
    let item = persist_fixture_work_item();
    assert_eq!(item.input_refs.exact_prompt, fixture_exact_prompt());
    assert_eq!(item.audit_receipt.prompt_sha256, sha256(fixture_exact_prompt()));
}

#[test]
fn immutable_kind_schema_and_input_refs_are_technical_discriminator() {
    assert!(cannot_mutate_after_dispatch(["kind", "output_schema", "input_refs"]));
}

#[test]
fn intel_review_v1_accepts_only_pass_rework_needs_human() { /* closed terminal schema */ }

#[test]
fn advisory_rework_decision_is_pure_and_runtime_disabled() { /* no scheduler/state effects */ }
```

内部 `requested_role/requested_kind` 只锁定对既有 WorkItem合同的 server-stamped兼容，不进入模型接口，也不代表新业务角色。`kind + output_schema + input_refs` 是宿主不可变的技术 discriminator；模型不能改写。

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(target_intel_spawn_schema_) | test(target_intel_dynamic_task_)' --status-level fail
```

### Step 3：实现 fixture-only Worker primitive

新增 reserved tool常量 `stage_team_spawn_intel_subagents`，参数严格为：

```json
{
  "agents": [
    {
      "name": "核对 example.com 归属",
      "prompt": "围绕现有 evidence 寻找独立归属证据，并返回新 refs 与矛盾。",
      "subject_refs": ["pivot:domain:example.com"]
    }
  ]
}
```

实现要求：

- 只有 exact bound TargetIntel fixture Goal owner可调用；
- 其它 stage继续使用现行 role/kind接口，不改语义；
- name规范化、长度上限80，prompt非空且长度上限4000，subject refs排序去重且不超过 spec上限；
- host用 `(operation,unit,request_epoch,name,prompt_sha256,subject_refs_sha256)` 生成 dedupe key；
- host内部适配到现有 `intel_provider/provider_followup`，模型不能覆盖；
- child objective使用模型提交的动态任务 prompt作为任务正文，并附 host-owned neutral envelope、Goal/scope/tool policy；不拼入固定 provider角色或六轴方法论；
- exact prompt正文与hash同时写 immutable `input_refs` 和 `intel.work_item_dispatch.v1` audit receipt；只有hash不合格；
- child结果必须返回 evidence/pivot/action refs；纯 prose只作为 summary，不能形成完成事实；
- 仍复用 generic `recon` definition和最小工具面，不新增 builder/registry中的 `intel_*` Agent定义。

### Step 4：实现 reviewer primitive，保持 runtime observe-only

- 新增 reserved `stage_team_request_intel_review`，仅接收 completion claim；bundle和review prompt由宿主提供；
- reviewer同样复用 generic executor，宿主将 `kind=intel_review_shadow_v1`、闭合 `output_schema=intel_review.v1`、不可变 exact `input_refs` stamp进 WorkItem；
- reviewer prompt是中性核验说明，不包含固定 recon六轴答案、预设 PASS或业务角色 persona；
- reviewer工具面只有 `submit_result`，不能 recon/search/fetch/spawn/prepare-final；
- parser只接受 `PASS | REWORK | NEEDS_HUMAN`；REWORK必须有 finding、evidence refs和close condition；
- 本 Task同时实现纯函数 `evaluate_advisory_rework`，供测试验证 fingerprint/material-delta规则；runtime调用点固定为 disabled，且函数不能调 scheduler/repo/state mutation；
- primitives先于 Task 6 Goal owner落地并保持测试全绿，避免 Owner依赖尚不存在的 reviewer接口。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(target_intel_spawn_schema_) | test(target_intel_dynamic_task_) | test(generic_worker_prompt_) | test(exact_worker_and_reviewer_prompts_) | test(immutable_kind_schema_) | test(intel_review_v1_) | test(advisory_rework_decision_) | test(stage_team_dispatch_workers_)' --status-level fail
cargo clippy -p golish-sub-agents -p golish-agent-runtime --lib --tests -- -D warnings
cargo fmt -p golish-sub-agents -p golish-agent-runtime -- --check
```

### Step 6：提交

```bash
git add backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs
git commit -m "feat(intel): add fixture worker review primitives"
```

---

## Task 6：把 Company Controller 改成持续 Goal Owner

**Files:**

- Modify: `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- Test: inline tests in the same modules

### Step 1：写 lifecycle RED tests

```rust
#[test]
fn target_intel_goal_prompt_contains_outcome_constraints_verification_and_no_six_axis_done_list() {
    let prompt = target_intel_goal_objective(&fixture_context()).unwrap();
    for expected in ["Outcome", "Constraints", "Verification", "request review", "candidate does not authorize"] {
        assert!(prompt.contains(expected));
    }
    assert!(!prompt.contains("complete all six coverage axes"));
}

#[test]
fn target_intel_goal_owner_must_update_plan_before_dispatch_or_review() { /* control tool order */ }

#[test]
fn target_intel_goal_owner_resumes_same_chain_after_child_results() { /* chain id stable */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-sub-agents -E 'test(target_intel_goal_)' --status-level fail
```

### Step 3：实现 Goal owner prompt 与 turn protocol

把 TargetIntel从通用 `company_controller_objective` 分支出来：

- 首轮读取 fixture-frozen Goal contract、durable facts、receipts和frontier摘要；
- 每轮必须先 `update_plan`，再二选一：`stage_team_spawn_intel_subagents`、继续 semantic tool调用，或 `stage_team_request_intel_review`；
- 不再调用 `stage_team_prepare_final_submission` 作为自我完成声明；
- child结果回到同一 message chain，Goal owner重新读 material delta并更新计划；
- 当前 run terminal receipt可复用；error保持 frontier，blocked/unsupported记录原因；
- max worker/request只是资源上限，不是完成证明；有 material gap但燃料不足时请求 review并声明 limitation，不能声称完成；
- Goal owner completion claim包含：完成理由、open/blocked frontier、未使用 capability及原因、已知矛盾、residual；
- `StageRunReentryGuard` 仍只作 request-local guard，durable continuity来自现有 Team/Worker/message chain。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-sub-agents -E 'test(target_intel_goal_) | test(company_controller_) | test(stage_team_)' --status-level fail
cargo clippy -p golish-agent-runtime -p golish-sub-agents --lib --tests -- -D warnings
cargo fmt -p golish-agent-runtime -p golish-sub-agents -- --check
```

### Step 5：提交

```bash
git add backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs
git commit -m "feat(intel): run controller as persistent goal owner"
```

---

## Task 7：接入 Observe-only Shadow Reviewer 与同快照 Divergence

**Files:**

- Modify: `backend/crates/golish-sub-agents/src/executor_types.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- Modify: `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs`
- Test: inline tests plus existing fake-repo tests in the modified files

### Step 1：写 verdict parser RED tests

```rust
#[test]
fn intel_review_v1_accepts_only_three_verdicts_and_actionable_rework() {
    assert!(IntelReviewV1::parse(pass_review()).is_ok());
    assert!(IntelReviewV1::parse(rework_without_close_condition()).is_err());
    assert!(IntelReviewV1::parse(json!({"verdict":"retry"})).is_err());
}

#[test]
fn shadow_reviewer_receives_state_actions_contract_then_claim() {
    let bundle = build_shadow_review_bundle(&fixture_repo()).unwrap();
    assert_eq!(bundle.sections.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![
        ReviewSectionKind::DurableState,
        ReviewSectionKind::ObservableActions,
        ReviewSectionKind::FrozenContract,
        ReviewSectionKind::CompletionClaim,
    ]);
}

#[test]
fn shadow_rework_never_resumes_controller_or_opens_epoch() { /* no scheduler/state effects */ }

#[test]
fn shadow_needs_human_never_creates_hold_or_delays_legacy_gate() { /* detached observation */ }

#[test]
fn shadow_and_legacy_verdicts_bind_the_same_material_snapshot() { /* exact revision/hash */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app -E 'test(intel_review_v1_) | test(shadow_reviewer_) | test(shadow_needs_human_) | test(shadow_and_legacy_)' --status-level fail
```

### Step 3：实现 non-authoritative review bundle

新增 reserved host tool `stage_team_request_intel_review`。它只接受 Goal owner的 completion claim；operation/org/plan/worker/revision由 bound context提供。

Shadow bundle由宿主构造：

1. `durable_state`：organizations、trusted targets、current-run landing refs、evidence、authoritative audit receipts及blocked capability；`expansion_queue`只可作为标明non-authoritative的diagnostic mirror；
2. `observable_actions`：current-run tool calls、semantic pivot keys、动态 task exact redacted name/prompt/subject refs/result refs及hash；不收集或暴露 CoT，不能只给prompt hash；
3. `frozen_contract`：fixture Goal/methodology/tool/provider/browser policy的 exact版本和hash；
4. `completion_claim`：Goal owner最后提交内容。

四段和各自 SHA-256写进 reviewer WorkItem的 server-owned input projection；整个 Shadow result另写 `audit_log` evidence kind=`intel_review_shadow.v1`。本 Task不得新增表。

### Step 4：实现通用只读 reviewer invocation

- 不新增 defaults builder/registry固定 `intel_auditor`；
- 动态 reviewer name使用 `审计 Target Intel · round N`，prompt由宿主生成；
- Shadow期通过独立 context按顺序呈现四段，但文档和 event必须标记 `ordered_read_enforcement=prompt_only`；
- reviewer工具面只保留 `submit_result`，bundle作为 frozen input；不提供 recon/search/web/spawn/final submit；
- terminal schema是 `intel_review.v1`；
- host重算 finding fingerprint；
- PASS、REWORK、NEEDS_HUMAN都只写 `intel_review_shadow.v1` observed receipt；不得调用scheduler、恢复Controller、打开epoch、创建hold或写pass token；
- legacy fixture路径不等待reviewer，照常执行prepare-final与legacy Gate；reviewer失败/超时也只能形成shadow failure；
- pure `evaluate_advisory_rework`可以计算“如果启用将如何处理”，但runtime selector恒disabled，输出只能进入报告；
- host在legacy Gate结果出现后写同快照divergence row，至少覆盖 `review_pass_legacy_pass`、`review_pass_legacy_block`、`review_rework_legacy_pass`、`review_rework_legacy_block`；
- reviewer与legacy verdict必须绑定同一material snapshot/hash；若legacy路径在两者之间产生material写入，该样本标stale/不可比较，不能伪造四象限；
- finding fingerprint/no-delta/fuel只在报告中模拟，不产生生产或fixture runtime状态转换；真正的DB CAS在Plan B实现。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app -E 'test(intel_review_v1_) | test(shadow_reviewer_) | test(shadow_needs_human_) | test(shadow_and_legacy_) | test(company_controller_)' --status-level fail
cargo clippy -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app --lib --tests -- -D warnings
cargo fmt -p golish-agent-runtime -p golish-sub-agents -p golish-agent-app -- --check
```

### Step 6：提交

```bash
git add backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs
git commit -m "feat(intel): observe shadow review divergence"
```

---

## Task 8：补 Run Tree 与 Shadow Divergence 可观测性

**Files:**

- Modify: `scripts/run_tree.py`
- Add: `scripts/tests/test_run_tree_intel_goal.py`

### Step 1：写 fixture RED test

Fixture覆盖：

```text
Goal owner
  -> pivot company_name
  -> dynamic subagent "核对域名归属"
  -> pivot domain
  -> freeze one comparable material snapshot
  -> detached review REWORK (observed only)
  -> legacy Gate PASS on the same snapshot
  -> divergence review_rework_legacy_pass
```

断言输出包含：

- Goal contract version/mode/browser mode；
- dynamic task name、prompt hash/subject refs，不显示隐藏 role为业务角色；
- semantic pivot kind/value hash、provider/query status、evidence/landing refs；
- review round、bundle hash、ordered-read enforcement、verdict、finding和close condition；
- explicit `controller_resumed=false`、`hold_created=false`、`pass_token_written=false`；
- four-quadrant divergence key与same-snapshot hash；
- candidate disposition与active authorization分开显示。

### Step 2：运行 RED

```bash
python3 -m unittest scripts.tests.test_run_tree_intel_goal -v
```

预期：缺少 Goal/review渲染。

### Step 3：实现只读渲染

- 优先读 transcript + `run.log` +现有 DB rows；
- Shadow review从 evidence/audit和WorkItem output投影，不新增 DB query table；
- 默认截断 raw page/provider内容，只展示 hash、refs、status、bounded summary；
- `--full` 也不得输出凭证、cookie或未脱敏secret；
- 明确标注 `SHADOW (legacy gate authoritative)`。

### Step 4：运行 GREEN

```bash
python3 -m unittest scripts.tests.test_run_tree_intel_goal -v
python3 -m py_compile scripts/run_tree.py
```

### Step 5：提交

```bash
git add scripts/run_tree.py scripts/tests/test_run_tree_intel_goal.py
git commit -m "feat(intel): render goal review shadow traces"
```

若仓库已有不同的 `run_tree.py` 测试目录，使用现有目录并只暂存真实创建的 test文件；不得同时创建第二套 harness。

---

## Task 9：锁定 Rollout、Pentest Hard-skip 与 Failure Corpus

**Files:**

- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Modify: `backend/crates/golish/src/stage_run/runtime_v2.rs`
- Test: inline tests in `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs` and `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Modify: module cards listed in “实施前状态” for responsibilities/interfaces changed by Tasks 1–8
- Modify: `docs/modules/INDEX.md`
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

### Step 1：写端到端 fake-provider RED corpus

至少覆盖：

1. fixture company→domain→evidence→review PASS，同时legacy Gate PASS；
2. fixture reviewer发现未追 GitHub org并给REWORK，但Controller/epoch/工具调用数不变，legacy Gate仍独立完成；
3. provider无凭证，receipt blocked，reviewer可观察为residual或NEEDS_HUMAN，但不创建hold且绝不记empty；
4. pure advisory evaluator识别同一finding无material delta；runtime仍不执行状态转换；
5. shared CDN IP保持 ambiguous relation，不成为 Target active root；
6. stale/foreign org pivot在 dispatch前拒绝；
7. receipt/evidence append失败使 pivot保持 retryable；
8. production Red Team与Pentest的Goal owner、SubAgent、reviewer、新provider/public-web dispatch均为0；
9. 非 TargetIntel stages保持现有 scheduler/tool schema；
10. fixture重放相同receipt/bundle得到相同hash且不重复fake query；本计划不宣称production crash/resume。

所有测试使用 fake provider、fixture page和临时/隔离 DB seam，不访问互联网。

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish -E 'test(target_intel_goal_shadow_)' --status-level fail
```

### Step 3：完成 rollout guard

- production runtime没有fixture selector时无条件保持legacy；当前Red Team/Pentest profile和历史operation均不进入Goal Shadow；
- 只有显式eval/fixture context能构造Goal/worker/reviewer，且外部transport必须为fake；
- Pentest hard-skip在任何 Goal/worker/provider构造前返回；
- production CLI/resume不识别或恢复fixture Goal；fixture replay只用于测试，不承诺生产durability；
- Shadow verdict绝不写pass token、恢复chain或hold；legacy fixture Gate不等待detached reviewer；
- event、tool result和run tree都标 `fixture_dev_only=true, shadow_observe_only=true`；
- 不增加 frontend generated read model。

### Step 4：运行 focused acceptance

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit -p golish-intel-providers -p golish-pentest-domain -p golish-recon-app -p golish-sub-agents -p golish-agent-runtime -p golish-agent-app -p golish -E 'test(target_intel_goal_) | test(target_intel_goal_shadow_) | test(semantic_pivot_) | test(intel_review_v1_) | test(pentest_target_intel_)' --status-level fail
just space-guard
cargo clippy -p golish-agent-kit -p golish-intel-providers -p golish-pentest-domain -p golish-recon-app -p golish-sub-agents -p golish-agent-runtime -p golish-agent-app -p golish --lib --tests -- -D warnings
cargo fmt -p golish-agent-kit -p golish-intel-providers -p golish-pentest-domain -p golish-recon-app -p golish-sub-agents -p golish-agent-runtime -p golish-agent-app -p golish -- --check
cd ..
python3 -m unittest scripts.tests.test_run_tree_intel_goal -v
python3 -m py_compile scripts/run_tree.py
jq empty resources/harness/profiles/red_team.json
jq empty resources/harness/profiles/pentest.json
jq empty resources/harness/stages/target_intel/spec.json
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
git diff --check -- resources/harness/profiles/red_team.json resources/harness/stages/target_intel/spec.json resources/harness/stages/target_intel/methodology.md backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-pentest-domain/src/models/asset_intel.rs backend/crates/golish-pentest-domain/src/models/mod.rs backend/crates/golish-intel-providers/src/types.rs backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs backend/crates/golish-recon-app/src/asset_intel/runtime/http.rs backend/crates/golish-recon-app/src/agent_tools/mod.rs backend/crates/golish-recon-app/src/asset_intel/agent_intel.rs backend/crates/golish-recon-app/src/asset_intel/landing.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs backend/crates/golish-agent-runtime/src/execution_mode/policy.rs backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs backend/crates/golish-agent-runtime/src/execution_mode/modes/task.rs backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs backend/crates/golish/src/pentest_tool_factory.rs backend/crates/golish/src/stage_run/runtime_v2.rs scripts/run_tree.py scripts/tests/test_run_tree_intel_goal.py docs/modules docs/design/INDEX.md docs/superpowers/plans/INDEX.md feature_list.json agent-progress.md
```

### Step 5：更新文档和状态

- 更新每张受影响模块卡的职责、接口、依赖、坑和 focused test入口；
- 更新 `docs/modules/INDEX.md` 对应行的状态/说明；
- 在 `agent-progress.md` 记录每个 RED/GREEN run id、命令、exit code、关键证据、未跑全量门禁和未调用外部服务；
- 只有所有 focused acceptance通过且 Shadow不扩大scope，才把本 feature改为 `passing`；
- `feature_list.json.evidence` 明确写“fixture/dev-only observe shadow；production profiles unchanged；verdict has zero runtime authority”；
- Plan B继续 `not_started`，不因 Plan A passing自动获得 schema/cutover授权。

### Step 6：提交

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish/src/stage_run/runtime_v2.rs docs/modules docs/design/INDEX.md docs/superpowers/plans/INDEX.md feature_list.json agent-progress.md
git commit -m "test(intel): close goal loop shadow rollout"
```

---

## 完成标准

Plan A 可以标记 `passing`，当且仅当：

1. fixture/dev TargetIntel用 Outcome/Constraints/Verification驱动持续 Goal；production Red Team/Pentest仍完全legacy；
2. semantic pivot不接受 raw DSL/provider/scope/evidence authority；
3. exact query、artifact、landing和evidence receipts可回放；
4. 动态 SubAgent模型接口只有 name/prompt/subject refs，且没有新增固定业务 Agent定义；
5. Shadow reviewer能解析PASS/REWORK/NEEDS_HUMAN，但三种verdict均不恢复chain、不创建hold、不写pass token；advisory/fixed-point仅作纯函数报告；
6. candidate attribution和active authorization不混写；
7. Shadow verdict不产生 pass token，旧 Gate仍为 authority；
8. production Red Team与Pentest均为零新Goal/reviewer/provider/public-web dispatch，所有外部transport测试均为fake；
9. focused test、scoped Clippy/rustfmt、Python、JSON和exact diff checks有新鲜证据；
10. 模块卡、索引、feature和progress与实际状态一致。

Plan A passing只表示fixture架构与安全边界成立，不代表生产Shadow已上线。先生成fixture divergence/cost/safety报告并与用户评审；未获Plan B对schema、operation-frozen contract、generated IPC和两次profile promotion的明确授权前停止。
