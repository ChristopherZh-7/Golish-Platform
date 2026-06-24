# intel → EAS handoff 强化 实现计划

> **面向 AI 代理的工作者：** 用 executing-plans 逐任务实现。每任务单独 commit，TDD（先红后绿）。
> 设计：`docs/design/2026-06-24-intel-to-eas-handoff.md`。

**目标：** 把情报阶段采到、但只躺在 `organizations.*` JSON 里的攻击面（CIDR/ASN 段、CT 子域）**物化成 `targets.scope='in'` 行**，并加厚 intel→EAS 的交接（富字段 + 优先级），让下一阶段（EAS）真正吃到情报，而不只是当目标列表。
**架构：** 统一「物化成 targets」入口（被动 + 主动同一入口）+ 有界发现闭环（递归 1 层 + 新鲜度窗）。分 L0 完整性 / L1 厚度 / L2 gate / b1 按域名查 provider 四块，各自灰度。
**技术栈：** Rust（golish-recon-app / golish-db / golish-agent-app / golish-agent-kit）、sqlx、cargo nextest、resources JSON（provider 配置 + stage spec）。
**不变量：** I2 / I7 / I8 / I9（非事务非 fatal）/ I10（灰度 + `target_type='cidr'` 已存在零 schema 改）/ §2.7（cidr 主动扫人审）。

> **实现状态（2026-06-24，BajieAsk-agent-4）**：
> - ✅ **P0** Task P0-1/P0-2（`plan_promotable_cidrs` + landing 串接，`landing.rs`）、P0-3（`in_scope_targets_impl` 投影加宽，`recon.rs`）— 测试 `plan_promotable_cidrs_*` PASS。
> - ✅ **P1** `hostnames_from_certificates` + landing 串接（`landing.rs`）— 测试 `hostnames_from_certificates_*` PASS。
> - ✅ **P2 b1** `domain` 入 `AssetIntelHydrateConfig`（`types.rs`）+ `{{domain}}` render（`template.rs`）+ native domain 分支 `query_applies`（`native.rs`）+ provider 门 `provider_supports_domain`（`hydrate.rs`）+ `recon_map_assets` `domain` 入参（`agent_tools/mod.rs`）+ `fofa.json` domain 查询 — 测试 `query_applies_*` / `map_assets_schema_*` PASS。**D2 自动递归触发 deferred**（能力就绪，自动 wiring 需 EAS 落点）。
> - ✅ **P3** `attack_surface_priority` + `rank_attack_surface_seeds`（`app-core/domain/targets.rs`）+ `attack_surface_seeds_impl`（`recon.rs`）+ trait/override（`repo.rs`/`mod.rs`）+ `list_attack_surface_seeds` executor（`security.rs`）+ 声明（`security_tools.rs`）+ 两处 gating（`config.rs`/`tool_list.rs`）— 测试 `rank_attack_surface_seeds_*` PASS。
> - ⏸ **P4 deferred**：守卫 `active_stages_derive_from_evidence_but_not_authoritative`（`stage_spec.rs`）禁止 EAS 开 `authoritative_found`（active Empty 事实源未就绪）。维持 default off。
> - 验证：nextest 7 crates 全绿（修 2 计数断言：fofa 2→3、tool decls 41→42）+ clippy -D warnings exit 0 + check-fe exit 0。未活体（需重启）。

---

## 文件清单

| 文件 | 职责 | 改动 |
|---|---|---|
| `golish-recon-app/src/asset_intel/landing.rs` | 被动物化入口 | `plan_promotable_assets` 保留 CIDR → `cidr` 种子；新增 CT SAN 抽取 + 物化；`upsert_target` 加 `cidr` 分支 |
| `golish-recon-app/src/asset_intel/template.rs` | 渲染 | 加 `{{domain}}` 变量（3 处 render） |
| `golish-recon-app/src/asset_intel/agent_tools/mod.rs` | 工具入参 | `recon_map_assets` 加可选 `domain` 入参 |
| `golish-recon-app/src/asset_intel/service/hydrate.rs` | provider 运行 | 域名查询分支（domain 而非 company_name 进 render） |
| `golish-agent-app/src/ai/db_bridge/recon.rs` | EAS handoff | `in_scope_targets_impl` 投影加宽；新增 `attack_surface_seeds_impl` |
| `golish-agent-kit/src/tool_executors/security.rs` | 工具执行 | 新增 `list_attack_surface_seeds` 分支 |
| `resources/intel-providers/{fofa,quake,hunter}.json` | provider 查询 | 加按域名查询模板（逐 provider 看支持） |
| `resources/harness/stages/external_attack_surface/spec.json` | EAS gate | `coverage_complete` 加 `authoritative_found`（灰度 flag，默认 off） |

---

## Phase 0 · 只读确认（不写代码）

- [ ] 核 `organizations.asns` 的 JSON 形状：是否携带 prefix/CIDR 段（决定 L0a 能否从 asns 抽 CIDR，还是只从 `ip_ranges`）。读 `build_profile_patch_from_entries` / `0-zone.json`、`fofa.json` 的 `asns` profile_field 映射。
- [ ] 核 `organizations.certificates` 的 JSON 形状：SAN/CN 字段名与嵌套（决定 L0b 抽取路径）。读 provider cert 落库的 `profile_fields`。
- [ ] 核 `value_belongs_to_organization`（`organization_recon`）签名与语义（L0b 复用它过滤 owned 域）。
- [ ] 逐 provider 核 `resources/intel-providers/*.json` 哪些支持 domain 查询语法（fofa `domain=`、quake、hunter）；ENScan/0-zone 偏公司名 → 不加。
- **验证**：结论写进本 plan「Phase 0 结果」段。无结论不进 Phase A。

---

## Phase P0 · L0a CIDR 物化 + L1a 加宽投影（地基，先做）

### Task P0-1 — 写失败测：`plan_promotable_cidrs` 抽合法网段
- **文件**：`landing.rs`（tests）
- **步骤**：新增纯函数 `fn plan_promotable_cidrs(profile_ips: &[String]) -> Vec<String>`（保留能 `parse::<ipnet::IpNet>()`、含 `/` 的网段，去重；裸 IP 不在此函数）。先写测：
  ```rust
  #[test]
  fn plan_promotable_cidrs_keeps_networks_drops_bare_and_garbage() {
      let v = plan_promotable_cidrs(&[
          "203.0.113.0/24".into(), "10.0.0.0/8".into(),
          "1.2.3.4".into(),        // 裸 IP 不在此函数
          "not-a-net".into(),
      ]);
      assert_eq!(v, vec!["203.0.113.0/24".to_string(), "10.0.0.0/8".to_string()]);
  }
  ```
- **验证**：`cargo nextest -p golish-recon-app -E 'test(plan_promotable_cidrs)'` → **红**（函数未实现）。
- **提交**：（与 P0-2 合并）

### Task P0-2 — 实现 `plan_promotable_cidrs` + `upsert_target` 加 `cidr` 分支 → 绿
- **文件**：`landing.rs`
- **步骤**：
  1. 实现 `plan_promotable_cidrs`（用已在依赖树的 `ipnet`，无则用裸解析：含 `/` 且 `split('/')` 后 IP 合法 + 前缀位合法）。
  2. `promote_profile_assets_to_targets` 末尾：对 `plan_promotable_cidrs(&profile_ips)` 每个网段 `upsert_target(pool, org, &cidr, "cidr", None)`（`target_type='cidr'`，无 real_ip）。非 fatal（失败只 warn，I9）。
  3. `upsert_target` 的 `target_type` 入参已是 `&str` 直插 `$2::target_type`——`cidr` 是合法枚举值（`20260408000001_initial.sql:172`），无需改函数。
- **验证**：`cargo nextest -p golish-recon-app -E 'test(plan_promotable_cidrs)'` → 绿；`cargo build -p golish-recon-app`。
- **提交**：`feat(recon): materialize owned CIDR ranges as cidr scope targets (L0a)`

### Task P0-3 — 加宽 `list_in_scope_targets` 投影
- **文件**：`golish-agent-app/src/ai/db_bridge/recon.rs`
- **步骤**：`in_scope_targets_impl` 的 `.map(|t| json!{...})` 从 3 字段加宽：
  ```rust
  .map(|t| {
      json!({
          "target_id": t.id,
          "value": t.value,
          "type": t.target_type.as_str(),
          "source": t.source,
          "status": t.status.as_str(),
          "real_ip": t.real_ip,
          "ports": t.ports,
          "organization_id": t.organization_id,
          "http_status": t.http_status,
          "cdn_waf": t.cdn_waf,
      })
  })
  ```
  （字段名以 `TargetRow` 实际为准，Phase 0 已读。）
- **验证**：`cargo nextest -p golish-agent-app`（既有 in_scope 测无回归，必要时更新断言）；`cargo build -p golish-agent-app`。
- **提交**：`feat(harness): widen list_in_scope_targets projection with intel context (L1a)`

### Task P0-4 — P0 收尾
- **验证**：`cargo clippy -p golish-recon-app -p golish-agent-app --all-targets --no-deps -- -D warnings` → exit 0；ReadLints 改动文件无错。
- **提交**：（无额外改动则跳过）

---

## Phase P1 · L0b CT SAN 物化

### Task P1-1 — 写失败测：`hostnames_from_certificates` 抽 SAN/CN
- **文件**：`landing.rs`（tests）
- **步骤**：新增纯函数 `fn hostnames_from_certificates(certificates: &Value) -> Vec<String>`（按 Phase 0 核实的 JSON 形状抽 SAN/CN；去重；丢空 / 泛域 `*.` 前缀归一为根；不在此函数做 owned 过滤）。先写测（形状以 Phase 0 为准）：
  ```rust
  #[test]
  fn hostnames_from_certificates_extracts_san_cn_dedup() {
      let certs = serde_json::json!([
          {"subject":"CN=pingan.com","san":["bank.pingan.com","mail.pingan.com"]},
          {"subject":"CN=bank.pingan.com","san":["bank.pingan.com"]},
      ]);
      let mut v = hostnames_from_certificates(&certs);
      v.sort();
      assert_eq!(v, vec!["bank.pingan.com","mail.pingan.com","pingan.com"]
          .into_iter().map(String::from).collect::<Vec<_>>());
  }
  ```
- **验证**：`cargo nextest -p golish-recon-app -E 'test(hostnames_from_certificates)'` → **红**。
- **提交**：（与 P1-2 合并）

### Task P1-2 — 实现抽取 + landing 串接（过 owned 过滤）→ 绿
- **文件**：`landing.rs`
- **步骤**：
  1. 实现 `hostnames_from_certificates`（最小代码让 P1-1 绿）。
  2. `promote_profile_assets_to_targets`（或 enrich 落库收尾处）：`for host in hostnames_from_certificates(&org.certificates)`，过 `value_belongs_to_organization(org, &host)` 后 `upsert_target(pool, org, &host, "domain", None)`。非 fatal。
- **验证**：`cargo nextest -p golish-recon-app -E 'test(hostnames_from_certificates)'` → 绿；`cargo build -p golish-recon-app`。
- **提交**：`feat(recon): materialize owned CT SAN/CN hosts as domain targets (L0b)`

---

## Phase P2 · b1 按域名查 provider + D2 递归

### Task P2-1 — 写失败测：`{{domain}}` 渲染
- **文件**：`template.rs`（tests）
- **步骤**：先写测，断言 `render_http_template("domain=\"{{domain}}\"", company_name, ...)` 在传入 domain context 时替换为域名：
  ```rust
  #[test]
  fn render_substitutes_domain_placeholder() {
      let out = render_http_template_with_domain(
          "domain=\"{{domain}}\"", "平安", Some("pingan.com"), &cfg(), &HashMap::new());
      assert_eq!(out, "domain=\"pingan.com\"");
  }
  ```
  （新增 `render_http_template_with_domain` 或给现有函数加 `domain: Option<&str>` 参数；3 处 render 同步。）
- **验证**：`cargo nextest -p golish-recon-app -E 'test(render_substitutes_domain)'` → **红**。
- **提交**：（与 P2-2 合并）

### Task P2-2 — 实现 `{{domain}}` 渲染（3 处）→ 绿
- **文件**：`template.rs`
- **步骤**：`render_http_template` / `render_asset_intel_skill_args` / `render_http_json_value` 加 `domain: Option<&str>` 参数，`.replace("{{domain}}", domain.unwrap_or(""))`；所有调用点传 `None`（行为不变），仅 b1 域名分支传 `Some`。
- **验证**：`cargo nextest -p golish-recon-app -E 'test(render)'` → 绿；`cargo build -p golish-recon-app`。
- **提交**：`feat(recon): add {{domain}} placeholder to provider query rendering (b1)`

### Task P2-3 — provider 描述符加按域名查询模板
- **文件**：`resources/intel-providers/{fofa,quake,hunter}.json`（Phase 0 确认支持的那些）
- **步骤**：每个支持的 provider 加一套 `{{domain}}` 查询模板（fofa `domain="{{domain}}"`、quake 等价）；不支持 domain 语法的不动。加 guard 测：`<provider>_config_has_domain_query`（JSON valid + 含 `{{domain}}`）。
- **验证**：`python3 -m json.tool` 每个改动 JSON VALID；`cargo nextest -p golish-recon-app -E 'test(domain_query)'` 绿。
- **提交**：`feat(recon): add domain-keyed query templates to fofa/quake/hunter (b1)`

### Task P2-4 — `recon_map_assets` 加可选 `domain` 入参 + service 域名分支
- **文件**：`agent_tools/mod.rs`、`service/hydrate.rs`
- **步骤**：
  1. `passive_intel_parameters` 加可选 `domain`（org-keyed 仍必填 `organization_id`；传 `domain` 则走域名查询）。
  2. `run_phase` 把 `domain` 透传；`hydrate.rs::run_providers_for_org` 走域名分支：传 `domain` 进 render（而非 company_name），跑支持 domain 的 provider；结果照走 `promote_profile_assets_to_targets` 物化 + source_query_log。
- **验证**：`cargo nextest -p golish-recon-app`（含新 schema 测 + 既有零回归）；`cargo build`。
- **提交**：`feat(recon): recon_map_assets supports domain-keyed provider survey (b1)`

### Task P2-5 — D2 递归（EAS 新顶级域 → b1，1 层）
- **文件**：EAS 主动发现物化点（Phase 0 定位：EAS prober 落新资产处 / `direct/mod.rs` 相应工具臂）
- **步骤**：新资产为「新顶级域」（非任何现有 in-scope 资产的子域）时，触发一次 `recon_map_assets(org, domain=新顶级域)`；标记 `recursion_depth=1`，查回子域不再触发二次 b1（用一个 run 级 visited-apex 集去重）。
- **验证**：纯函数测「判定是否新顶级域」+「递归深度守卫」；`cargo nextest -p golish-recon-app`。
- **提交**：`feat(recon): expand newly-discovered apex domains via b1 (1-layer recursion, D2)`

---

## Phase P3 · L1b list_attack_surface_seeds + 优先级 + D3 cap

### Task P3-1 — 写失败测：`rank_attack_surface_seeds` 优先级排序
- **文件**：`golish-agent-kit`（或 recon-app）新纯函数模块（tests）
- **步骤**：纯函数 `fn rank_attack_surface_seeds(seeds: Vec<Seed>, cap: Option<usize>) -> Vec<Seed>`（高置信 + 已解析 real_ip + 资产类型权重[domain/url>ip>cidr] 排序；cap=Some(n) 截断，None 不截）。先写测覆盖排序 + cap。
- **验证**：`cargo nextest -E 'test(rank_attack_surface_seeds)'` → **红**。
- **提交**：（与 P3-2 合并）

### Task P3-2 — 实现排序 + cap → 绿
- **文件**：同上
- **步骤**：实现 `Seed` 结构 + `rank_attack_surface_seeds`（cap 默认 None=不截，读可配置项）。
- **验证**：`cargo nextest -E 'test(rank_attack_surface_seeds)'` → 绿。
- **提交**：`feat(harness): attack-surface seed ranking + per-org cap (D3)`

### Task P3-3 — `attack_surface_seeds_impl` + `list_attack_surface_seeds` 工具
- **文件**：`db_bridge/recon.rs`、`tool_executors/security.rs`
- **步骤**：
  1. `attack_surface_seeds_impl(org_id)`：join in-scope targets + dns_records + source_query_log，组 `Seed`（target_id/value/type/source/status/real_ip/dns_records/provider_sources/confidence/org_id/known_ports/last_intel_evidence_ids），过 `rank_attack_surface_seeds`。
  2. `security.rs` 加 `list_attack_surface_seeds` 分支（org-subtree 隔离，mirror `list_in_scope_targets`）。
- **验证**：`cargo nextest -p golish-agent-app -p golish-agent-kit`；`cargo build`。
- **提交**：`feat(harness): list_attack_surface_seeds tool (rich intel handoff, L1b)`

---

## Phase P4 · L2 EAS gate authoritative（最后，单独灰度）

### Task P4-1 — EAS coverage_complete 升 authoritative（灰度 flag 默认 off）
- **文件**：`resources/harness/stages/external_attack_surface/spec.json`、gate 读 flag 处（mirror target_intel 的 `authoritative_found`）
- **步骤**：spec `coverage_complete` 加 `"authoritative_found": true` + 一个灰度开关（参考 freshness_window 的 gray-switch 模式），**默认 off**；on 时 EAS found 由 DB truth（targets/fingerprints）决定，自报 cell 不再兜底。
- **验证**：`cargo nextest -p golish-agent-kit`（gate 测：off=byte-for-byte 不变；on=自报 cell 不补 found）；`cargo clippy ... -D warnings`。
- **提交**：`feat(harness): EAS coverage_complete authoritative_found (gray-switch, default off, L2)`

---

## 收尾（每阶段 / 全量）

- [ ] `cargo clippy -p golish-recon-app -p golish-db -p golish-agent-app -p golish-agent-kit --all-targets --no-deps -- -D warnings` → exit 0。
- [ ] `just precommit`（全量）→ OK（动 schema 无、动 ts-rs 无；若 L1b 跨 IPC 需 derive ts-rs 则补）。
- [ ] 活体（用户环境，重启）：跑 target_intel → 只读 DB 复查 `targets` 出现 `cidr` + CT 子域行；`list_attack_surface_seeds` 返回富字段；新顶级域触发 b1；EAS 扫到 cidr/CT 资产。
- [ ] 更新 `agent-progress.md` + `feature_list.json`（本条目）+ 本 plan / 设计状态戳。

## 风险与回滚

- **CIDR 主动扫越权**：D1 — cidr 种子只「可见」，主动端口扫由 EAS `human_approval` 卡（§2.7）。
- **b1 provider 不支持 domain**：逐 provider 看描述符，不支持的不加模板（记 blocked），不硬塞。
- **递归爆炸**：D2 递归 1 层 + run 级 visited-apex 去重 + 新鲜度窗。
- **回滚**：各层加性 / 灰度。L0 物化去掉 landing 两段即恢复；b1 provider 描述符不加 domain 模板即不生效；L1a 投影加宽是展示增强；L2 gate flag 默认 off。**零 schema 改动**（`cidr` 枚举已存在）→ 无 migration 回滚负担。
