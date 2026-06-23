# provider 直接落库到 gate 真值表（DNS/SUBDOMAIN 去桥）实现计划

> **面向 AI 代理的工作者：** 用 executing-plans 逐任务实现。每任务单独 commit，TDD（先红后绿）。

**目标：** 让 provider 测绘的 domain↔IP / 子域在 landing 当场直接写进 `dns_records` / `target_assets`（gate 读的表），消除对 gate-time 实时再解析桥（`land_dns_records` 的 `lookup_host`）的依赖；旧桥靠 `NOT EXISTS` 守卫自动降级为 fallback。
**架构：** 在 `golish-recon-app/src/asset_intel/landing.rs` 的 provider landing 处，拿到 `target_id` 后直接 `dns_records::upsert`（A/AAAA, source='<provider>'）+ 子域 `target_assets::upsert`（root 解析复用）。gate 读点、判定纯函数、schema 全不改。
**技术栈：** Rust（golish-recon-app / golish-db repo）、sqlx、cargo nextest。
**设计：** `docs/design/2026-06-23-provider-direct-landing-gate-tables.md`
**不变量：** I8 / I9（非事务、非 fatal）/ §2.5（gate 不改）/ §2.7（无 schema 改动）。

---

## 文件清单

| 文件 | 职责 | 改动 |
|---|---|---|
| `golish-recon-app/src/asset_intel/landing.rs` | provider landing | `upsert_target` 返回 `Uuid`；新增 `land_provider_dns` + provider 子域直接落；`promote_profile_assets_to_targets` 串接 |
| `golish-recon-app/src/organization_recon/persistence.rs` | 暴露 root 解析复用（可选） | 若需复用 subdomain↔root 配对，`pub(crate)` 化最小 helper |
| （只读确认）`golish-recon-app/src/asset_intel/agent_intel.rs` | 确认是否发 cert 查询（Phase 0） | 不改，仅核 |
| `golish-intel-providers/src/quake/types.rs` | Quake 反序列化 | `QuakeInnerService` 加 `cert: Option<String>`（捕获 `service.cert`） |
| `golish-intel-providers/src/quake/mapper.rs` | Quake → ProviderRecord | `map_cert`/`map_site` 读 `service.cert` 回退顶层 flat（修 CT 不落根因） |

---

## Phase Q · Quake cert 提取修复（2026-06-23 活体根因 · 优先做）

> 根因（§7 设计活体实测）：Quake 返回 cert 在 `service.cert`（+ `service.tls.handshake_log`），无顶层 flat `cert`；`QuakeInnerService` 未捕获 `service.cert`，`map_cert` 读顶层 flat → `None` → Quake CT 永不落库。

### Task Q1 — 写失败测：map_cert 从嵌套 service.cert 提取
- **文件**：`golish-intel-providers/src/quake/mapper.rs`（tests）
- **步骤**：新增测——构造 `QuakeService { cert: None, service: Some(QuakeInnerService { cert: Some("CN=*.pingan.com".into()), ..Default::default() }), .. }`，断言 `map_cert(...).fields.get("cert")` == `"CN=*.pingan.com"`：
  ```rust
  #[test]
  fn cert_mapper_reads_nested_service_cert_when_flat_absent() {
      let svc = QuakeService {
          cert: None,
          service: Some(QuakeInnerService { cert: Some("CN=*.pingan.com".into()), ..Default::default() }),
          domain: Some("pingan.com".into()),
          ..Default::default()
      };
      let rec = map_cert(svc, raw());
      assert_eq!(rec.fields.get("cert").map(String::as_str), Some("CN=*.pingan.com"));
  }
  ```
- **验证**：`cargo nextest -p golish-intel-providers -E 'test(cert_mapper_reads_nested)'` → **红**（QuakeInnerService 无 cert 字段 → 编译失败，先红）。
- **提交**：（与 Q2 合并）

### Task Q2 — 加 `QuakeInnerService.cert` + mapper 读它 → 绿
- **文件**：`quake/types.rs`、`quake/mapper.rs`
- **步骤**：
  1. `types.rs` `QuakeInnerService` 加：
     ```rust
     /// Cert subject when Quake nests it under `service.cert` (not flat top-level).
     #[serde(default)]
     pub cert: Option<String>,
     ```
  2. `mapper.rs` 加 helper + 在 `map_cert`/`map_site` 用：
     ```rust
     fn cert_subject(svc: &QuakeService) -> Option<String> {
         svc.cert.clone()
             .or_else(|| svc.service.as_ref().and_then(|s| s.cert.clone()))
     }
     ```
     `map_cert`/`map_site` 里 `insert_if_present(&mut fields, "cert", cert_subject(&svc).as_ref());`（替换原 `svc.cert.as_ref()`）。
- **验证**：`cargo nextest -p golish-intel-providers`（Q1 绿 + 既有 quake 测无回归）；`cargo clippy -p golish-intel-providers --all-targets --no-deps -- -D warnings`。
- **提交**：`fix(intel): extract Quake cert from nested service.cert (CT was dropped)`

### Task Q2b — ✅ 真正的 active-path 修复：quake.json HttpJson 提取 service.cert（2026-06-23 勘验纠正）
- **纠正**：勘验发现 `quake.json` 用的是 **HttpJson runtime**（config 驱动 + JSONPath 抽取），**不是** native QuakeProvider。故 Q1/Q2（native `map_cert`）修的是 native 路径（防御性、正确，但 quake.json 不走它）。用户的 quake 走 HttpJson，CT 提取由 `quake.json` 的 `normalize.profile_fields` 决定。
- **根因（active path）**：cert 请求 `include` 已含 `service.cert`，但 `profile_fields` **没有** `service.cert → certificates` 映射 → 抓回来但抽取时丢弃 → certificates 永远空。
- **修（已做）**：`resources/intel-providers/quake.json` `profile_fields` 加一条 `{source_field:"service.cert", target_field:"certificates", transform:"trim", when:exists}`。
- **验证（已跑）**：quake.json `python3 -m json.tool` VALID；新 guard 测 `quake_config_maps_service_cert_to_certificates` 绿；`nextest -p golish-recon-app -E 'test(quake)|test(extract)|test(http_json)'` → 13 passed（既有 extraction 引擎测无回归）。
- **提交**：`fix(recon): map quake service.cert -> certificates in HttpJson extraction (active CT path)`

### Task Q3 — enrich cert 查询用域名（可选增强 · 2026-06-23 勘验：非配置小改，可延后）
- **现状勘验（已读真实代码）**：cert 模板在 `resources/intel-providers/{quake,fofa}.json`（`cert: "{{company_name}}"` / `cert="{{company_name}}"`）；渲染器 `asset_intel/template.rs` **只支持 `{{org}}`/`{{company_name}}`/`{{out_dir}}`/`{{config.*}}`，无 `{{domain}}`**，且查询**每 org 渲染一次**（非 per-root 迭代）。故「cert 按域名查」**不是改一行配置**。
- **优先级：可选 / 可延后**。Q1+Q2 修好提取后，company_name 的 cert 查询（活体 71K 命中）即可落 CT；域名查只是更精准（24K，更干净）。recall（company_name）vs precision（domain）权衡——建议先保留 company_name（CT 已能落），域名查作为后续精度增强。
- **若做（独立子任务）**：① `template.rs` 加 `{{domain}}` 变量；② 查询计划对 `org.domains` 每个 owned root 迭代发一条 cert 查询，结果并入同一 `ProfilePatch.certificates`。TDD：renderer `{{domain}}` 替换测 + per-root 迭代测。
- **提交**：`feat(recon): add {{domain}} cert query per owned root (CT precision)`

---

## Phase 0 · 只读确认（不写代码）

- [ ] 核 `agent_intel.rs` / `service/*` 的 provider 调用是否包含 **FOFA cert 查询（QueryType::Cert）**。
- [ ] 核 `land_dns_records` 的 `NOT EXISTS(dns_records)` 守卫（确认直接落后它自动跳过已落行）。
- **验证**：把结论写进本 plan「Phase 0 结果」段；CT=0 归因（没发 cert 查询 vs 没命中）落定。无结论不进 Phase A。

## Phase A · DNS 直接落（核心）

### Task A1 — `upsert_target` 返回 `Uuid`
- **文件**：`landing.rs`
- **步骤**：把 `async fn upsert_target(...) -> Result<bool, GolishError>` 改为 `-> Result<Uuid, GolishError>`：
  - existing 分支：拿到 `id` 后照旧更新 org/real_ip，`return Ok(id)`。
  - insert 分支：`INSERT ... RETURNING id`（已有 `find_or_create_target` 同款），返回新 id。
  - `promote_profile_assets_to_targets` 内调用点把 `Ok(_) => landed += 1` 改为 `Ok(_id) => landed += 1`（暂不用 id，Task A2 用）。
- **验证**：`cargo build -p golish-recon-app`（编译过）。
- **提交**：`refactor(recon): upsert_target returns target id`

### Task A2 — 写失败测：provider 配对应落 dns_records
- **文件**：`landing.rs`（tests）
- **步骤**：新增纯函数 `fn provider_dns_record(host: &str, ip: &str) -> Option<(&'static str, String, String)>`（返回 `(record_type, name, value)`；`ip.parse::<IpAddr>()` 失败→None；IPv4→"A"、IPv6→"AAAA"）。先写测：
  ```rust
  #[test]
  fn provider_dns_record_classifies_a_aaaa_and_rejects_garbage() {
      assert_eq!(provider_dns_record("bank.pingan.com","1.2.3.4"),
                 Some(("A","bank.pingan.com".into(),"1.2.3.4".into())));
      assert_eq!(provider_dns_record("x.com","2400:cb00::1").map(|t|t.0), Some("AAAA"));
      assert_eq!(provider_dns_record("x.com","not-an-ip"), None);
  }
  ```
- **验证**：`cargo nextest -p golish-recon-app -E 'test(provider_dns_record)'` → **红**（函数未实现）。
- **提交**：（与 A3 合并提交）

### Task A3 — 实现 `provider_dns_record` + landing 串接 → 绿
- **文件**：`landing.rs`
- **步骤**：
  1. 实现 `provider_dns_record`（最小代码让 A2 绿）。
  2. `promote_profile_assets_to_targets` 的 domain 循环里，`upsert_target` 拿到 `target_id` 后：
     ```rust
     if let Some(ip) = real_ip.as_deref() {
         if let Some((rt, name, value)) = provider_dns_record(&domain, ip) {
             // 直接落 provider 的 (host->ip) 为 DNS 记录；source 记 provider 名；
             // 非 fatal（失败只 warn，不回滚已提交 enrich，I9）。
             if let Err(e) = golish_db::repo::dns_records::upsert(
                 pool, target_id, org.project_path.as_str(), rt, &name, &value, "provider",
             ).await {
                 tracing::warn!(%domain, %e, "provider dns_records direct-land failed (non-fatal)");
             }
         }
     }
     ```
- **验证**：`cargo nextest -p golish-recon-app -E 'test(provider_dns_record)'` → 绿；`cargo build -p golish-recon-app` 过。
- **提交**：`feat(recon): land provider host↔IP directly into dns_records`

## Phase B · SUBDOMAIN 直接落

### Task B1 — provider 子域 → target_assets（✅ 已实现 2026-06-23：喂对 host 给现有 landing，DRY）
- **文件**：`golish-recon-app/src/asset_intel/agent_intel.rs`（Enrich 分支）
- **实现**：勘验发现 agent enrich 路径**已调** `land_target_intel_coverage`，但 `subdomain_hosts` 传的是 `organizations.domains`——这些**全是 owned root**，`collect_subdomain_pairs` 把「host==owned root」当资产本身跳过 → 自我抵消、0 子域（live DB `target_assets=0` 的 agent 侧根因）。修：先算 `pairs`（provider 真子域），把 `pairs.host` 并入 `subdomain_hosts` 再调 `land_target_intel_coverage` → `collect_subdomain_pairs` 把每个 provider 子域配到 owned root → `target_assets(asset_type='subdomain')`。**复用**既有（已测）pairing + root 解析 + upsert，零重复，不在 landing.rs 另写直接落。
- **验证（已跑）**：`cargo nextest -p golish-recon-app` → **196 passed**（`collect_subdomain_pairs_*` 既有测覆盖 pairing 逻辑）；`clippy -D warnings` exit 0。
- **提交**：`feat(recon): feed provider subdomains to target_intel coverage landing`

### Task B2 — 桥降级 fallback 的回归确认
- **文件**：（无改动，验证）
- **步骤**：确认 `land_dns_records`（`NOT EXISTS`）+ `land_subdomain_assets`（upsert DO NOTHING）在直接落后只补缺口、不重复。
- **验证**：`cargo nextest -p golish-recon-app -p golish-db`（全绿）。

## Phase C · 收尾

- [ ] `cargo clippy -p golish-recon-app -p golish-db --all-targets --no-deps -- -D warnings` → exit 0。
- [ ] 活体（用户环境）：`just dev` 重启 → 跑 target_intel → 只读 DB 复查 `dns_records` / `target_assets` 不再为 0（provider 直接落生效），且行 `created_at` 是本轮（配合 freshness 窗）。
- [ ] 更新 `agent-progress.md` + `feature_list.json`（新增条目）+ 本 plan / 设计状态戳。

---

## 风险与回滚
- provider ip 脏数据 → `provider_dns_record` 的 `parse::<IpAddr>()` 过滤（Task A2 测钉死）。
- 直接落与桥重复 → 唯一键 DO NOTHING 幂等；桥 `NOT EXISTS` 自动让位。
- 回滚：直接落是**新增写**，去掉 landing 内的两段 upsert 即恢复旧行为（桥仍在）；零 schema 改动。
- provider 缓存可能旧 → 由 freshness 窗（2026-06-22）排除上一 run 的行；本 run 重落即新鲜。
