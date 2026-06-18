# Recon 工具拆解：删 enrich → recon_map_assets（测绘）+ recon_lookup_whois（RDAP）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现；每个任务单独 commit、单独验证（TDD）。
> **本计划取代同名前一版（「内部瘦身」思路）。** 用户 2026-06-18 拍板：**删掉 enrich，把测绘包成一个工具、RDAP 包成另一个工具**。

**目标：** 删除大包大揽的 `recon_enrich_assets`，换成两个职责单一、AI 自己按需调用的工具：
- **`recon_map_assets`（测绘）**：fofa/quake/hunter/shodan/0.zone 的 provider 采集 + 把 provider 结果落库（子域→`target_assets`、域名/IP/ASN/intel/证书→`organizations` 列、host↔IP 对带 `real_ip`）。**不**做 crt.sh / 逐 host DNS 解析 / 逐 IP 反查。
- **`recon_lookup_whois`（RDAP）**：按 org 一次性 RDAP 查 whois → 落 `organizations.whois`。

**架构：** enrich 的「provider 采集」逻辑（`run_passive_intel(Enrich)`）保留进 `recon_map_assets`；enrich landing 里只留 `land_subdomain_assets` + 配对闭包；`land_whois`（RDAP）抽成 `recon_lookup_whois` 工具的实现；`land_ct_and_whois` 的 crt.sh 段、`land_dns_records`、`land_rdns`、`land_ip_whois` 全删。CT/DNS 缺口由现有 `ctfr`/`dig` 工具 + provider cert/host↔IP 对补。gate 判定**一行不改**（WHOIS 覆盖走 `coverage_truth` 的 whois 列投影；其余走命令事实）。
**技术栈：** Rust（golish-recon-app / golish-agent-runtime / golish-sub-agents / golish）+ resources JSON + cargo nextest。

---

## 背景（实测证据，2026-06-18 直查内嵌 PG · org 65b64a60 中国平安集团）

| organizations 列 | 实测 | 谁填 | 新归属 |
|---|---|---|---|
| domains(202) | 混 URL-IP | provider | recon_map_assets |
| ip_ranges(133) | 干净 | provider | recon_map_assets |
| asns(21) | 真实 ASN | provider | recon_map_assets（已覆盖 ASN） |
| certificates(20) | **垃圾**(volcsiriusbd.com) | enrich crt.sh | **删**；改 ctfr + fofa cert |
| whois(obj) | **真实 RDAP** | enrich RDAP | recon_lookup_whois |
| intel | 工商/ICP/OSINT 齐 | provider | recon_map_assets |

护栏：I7（交付有 evidence）、I8（不新增任何「缺数据→checked_empty」推断）、I2（org 隔离）、§2.5（gate 判定确定性、**本计划不改 rule_engine**）、I4（命令/工具命名 `<domain>_<verb>_<object>`）。

---

## 文件结构（改动清单）

**A. 核心逻辑（golish-recon-app）**
| 文件 | 改动 |
|---|---|
| `src/organization_recon/persistence.rs` | `land_ct_and_whois`→只留 RDAP 的 `pub(crate) land_whois`；`land_target_intel_coverage` 只调 `land_subdomain_assets`（whois 不再在此）；`CoverageLandingSummary`={subdomains}；删 `land_dns_records`/`land_rdns`/`land_ip_whois`/`dig_ptr` |
| `src/organization_recon/mod.rs` | re-export `land_whois`（供 whois 工具用）；清掉被删函数的 re-export |
| `src/agent_tools/mod.rs` | `ReconEnrichAssetsTool`→`ReconMapAssetsTool`(name `recon_map_assets`)；新增 `ReconLookupWhoisTool`(name `recon_lookup_whois`)；改 `ReconListProvidersTool` 描述里的 enrich 字样 |
| `src/asset_intel/agent_intel.rs` | enrich landing 调用点：landing 日志字段收窄；whois 不在此（移到工具） |

**B. 工具注册 / 策略 / 提示（golish + golish-agent-runtime + golish-sub-agents）**
| 文件 | 改动（rename `recon_enrich_assets`→`recon_map_assets`，+ 新增 `recon_lookup_whois`） |
|---|---|
| `golish/src/pentest_tool_factory.rs:74-79` | 注册 `ReconMapAssetsTool` + `ReconLookupWhoisTool`（替换 ReconEnrichAssetsTool） |
| `golish-agent-runtime/src/execution_mode/policy.rs:103/134/154/180-182` | `BridgeToolSelection`：字段 `recon_enrich_assets`→`recon_map_assets`，新增 `recon_lookup_whois: bool`；三处（struct/all_enabled/none）+ enabled_tool_names + 任何 `.recon_enrich_assets` 读点 |
| `golish-agent-runtime/src/execution_mode/prompt_render.rs:196` | ToolRow：换成两行（map_assets + lookup_whois） |
| `golish-agent-runtime/src/execution_mode/modes/task.rs:74` | `recon_enrich_assets: true`→`recon_map_assets: true, recon_lookup_whois: true` |
| `golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs:467-471` | 证据记账 match 臂：`"recon_enrich_assets"`→`"recon_map_assets"`，并加 `"recon_lookup_whois"`（都返回 JSON 摘要，需 book ledger） |
| `golish-agent-runtime/src/agentic_loop/tool_list.rs:150` | 注释引用更新 |
| `golish-sub-agents/src/defaults/builder/mod.rs:99/131`、`registry.rs:100/137` | recon + pentester 工具清单：`"recon_enrich_assets"`→`"recon_map_assets"`，加 `"recon_lookup_whois"` |
| `golish-sub-agents/src/defaults/prompts/execution_planning.rs:102/112/113` | recon prompt 方法论改写（见 Task 5） |
| `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs:2652` | 规划提示文案：`recon_enrich_assets`→`recon_map_assets`（+ 提 recon_lookup_whois） |
| `golish-recon-app/src/asset_intel/availability.rs`、`db_bridge/recon.rs`、`persistence.rs:249` 注释 | 字样更新（非功能） |
| `resources/harness/stages/target_intel.json` methodology | 顺序化方法论（见 Task 5） |

**C. 测试（随各自改动同步）**
`golish-sub-agents/src/defaults/tests.rs:120/144/167`、`golish-agent-runtime/src/execution_mode/prompt_render_tests.rs:24/81`、`golish/src/stage_run/mod.rs:958/975`、`execute.rs:2943/2955`。

---

## Phase 1 · persistence：抽 land_whois + 删慢 landing

### Task 1.1 · `land_ct_and_whois` → 只留 RDAP 的 `pub(crate) land_whois`
**文件：** `golish-recon-app/src/organization_recon/persistence.rs`
**步骤：** 用下列函数整体替换 `land_ct_and_whois`（568–691 行）。改 `pub(crate)`（供 whois 工具调用），删 crt.sh 全段，返回 `bool`：

```rust
/// Land WHOIS (RDAP) → `organizations.whois` — org-level column the target_intel
/// WHOIS coverage cell reads. HTTP, bounded, best-effort: only fills when empty;
/// failures skipped, never fatal. (`whois` is schema-ahead → direct SQL.)
/// CT intentionally removed (was the 300s-timeout culprit + only produced junk);
/// CT now comes from the `ctfr` tool + fofa native cert. (plan 2026-06-18)
pub(crate) async fn land_whois(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
) -> Result<bool, GolishError> {
    let whois_existing: Option<Value> =
        sqlx::query_scalar::<_, Option<Value>>("SELECT whois FROM organizations WHERE id = $1")
            .bind(organization.id)
            .fetch_one(pool)
            .await?;
    if whois_existing.as_ref().is_some_and(|v| !json_value_is_empty(v)) {
        return Ok(false);
    }
    let domains = registrable_domains(organization);
    if domains.is_empty() {
        return Ok(false);
    }
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("golish-recon/1.0")
        .build()
    else {
        return Ok(false);
    };
    let mut whois_value: Option<Value> = None;
    for domain in &domains {
        let url = format!("https://rdap.org/domain/{domain}");
        let Ok(resp) = client.get(&url).send().await else { continue };
        if !resp.status().is_success() { continue }
        let Ok(text) = resp.text().await else { continue };
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if value.is_object() && !json_value_is_empty(&value) {
                whois_value = Some(value);
                break;
            }
        }
    }
    let landed = whois_value.is_some();
    if let Some(value) = whois_value {
        sqlx::query("UPDATE organizations SET whois = $1, updated_at = NOW() WHERE id = $2")
            .bind(value).bind(organization.id).execute(pool).await?;
    }
    Ok(landed)
}
```

**验证：** `cargo check -p golish-recon-app` → 预期仅 `land_target_intel_coverage` 旧调用、`land_dns_records/land_rdns/land_ip_whois` unused 报错（后续 Task 修）。
**提交：** `refactor(recon): extract land_whois (RDAP-only), drop crt.sh CT`

### Task 1.2 · 收窄 `CoverageLandingSummary` + `land_target_intel_coverage`
**文件：** 同上。
**步骤：** `CoverageLandingSummary` 改为 `{ pub subdomains: usize }`（删 dns_records/certificates/whois/rdns/ip_whois）。`land_target_intel_coverage`（182–267）函数体改为只调 `land_subdomain_assets`：

```rust
pub(crate) async fn land_target_intel_coverage(
    pool: &sqlx::PgPool,
    organization: &golish_db::models::Organization,
    run_id: &str,
    subdomain_hosts: &[String],
) -> CoverageLandingSummary {
    let subdomains = land_subdomain_assets(pool, organization, run_id, subdomain_hosts)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(organization_id = %organization.id, %error,
                "subdomain target_assets landing failed (recon persistence already committed)");
            0
        });
    CoverageLandingSummary { subdomains }
}
```
同步两个调用点日志（`persist_normalized_records` 149–158、`agent_intel.rs` ~235–244）只留 `subdomains`。
**验证：** `cargo check -p golish-recon-app`（DNS/rDNS/IP-WHOIS 现 unused → Task 1.3）。
**提交：** `refactor(recon): coverage landing = subdomains only`

### Task 1.3 · 删死 DNS/rDNS/IP-WHOIS landing
**文件：** 同上（+ `mod.rs` re-export）。
**步骤：** 删 `land_dns_records`(~434)、`land_rdns`(~699)+`dig_ptr`(~748)、`land_ip_whois`(~777)。保留 `registrable_domains`/`json_value_is_empty`/`land_subdomain_assets`/`normalized_host`。`mod.rs` 加 `pub(crate) use ...::land_whois;`。
**验证：** `cargo clippy -p golish-recon-app --all-targets -- -D warnings` → exit 0；`rg "crt\.sh|lookup_host|dig -x" .../persistence.rs` → 零命中。
**提交：** `refactor(recon): remove dead DNS/rDNS/IP-WHOIS landing`

---

## Phase 2 · 两个新工具

### Task 2.1 · `recon_map_assets`（由 ReconEnrichAssetsTool 改名 + 收窄）
**文件：** `golish-recon-app/src/agent_tools/mod.rs`
**步骤：** 把 `ReconEnrichAssetsTool` 改名 `ReconMapAssetsTool`，`name()` 返回 `"recon_map_assets"`，描述改为「测绘」语义；`execute` 仍走 `PassiveIntelPhase::Enrich`（provider 采集 + 现已收窄的 landing 只落子域 + 配对）：

```rust
pub struct ReconMapAssetsTool { pool: Arc<PgPool>, tools: ToolsConfigState }
impl ReconMapAssetsTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self { Self { pool, tools } }
}
#[async_trait::async_trait]
impl Tool for ReconMapAssetsTool {
    fn name(&self) -> &'static str { "recon_map_assets" }
    fn description(&self) -> &'static str {
        "Survey an organization's external footprint via cyberspace/intel providers \
         (0.zone / quake / fofa / hunter / shodan / ENScan): domains, IP ranges, ASN, \
         subdomains, certificates, ICP, apps, emails, OSINT — landed to the org profile \
         + target_assets (host↔IP pairs carry real_ip). Zero-touch. Use during \
         target_intel. WHOIS is a separate tool (recon_lookup_whois); CT certs also via \
         the ctfr tool. Returns a summary with counts + provider ids."
    }
    fn parameters(&self) -> Value { passive_intel_parameters("to survey assets for") }
    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        run_phase(&self.pool, &self.tools, &args, workspace, PassiveIntelPhase::Enrich, "map_assets").await
    }
}
```
**验证：** `cargo check -p golish-recon-app` exit 0（pentest_tool_factory 仍引用旧名 → Task 3.1 修）。
**提交：** `feat(recon): recon_map_assets tool (provider survey, renamed from enrich)`

### Task 2.2 · `recon_lookup_whois`（新工具，RDAP per-org）
**文件：** `golish-recon-app/src/agent_tools/mod.rs`
**步骤：** 新增工具：按 `organization_id` 加载 org → 调 `land_whois` → 返回摘要：

```rust
pub struct ReconLookupWhoisTool { pool: Arc<PgPool> }
impl ReconLookupWhoisTool {
    pub fn new(pool: Arc<PgPool>) -> Self { Self { pool } }
}
#[async_trait::async_trait]
impl Tool for ReconLookupWhoisTool {
    fn name(&self) -> &'static str { "recon_lookup_whois" }
    fn description(&self) -> &'static str {
        "Look up domain registration (WHOIS via RDAP) for an organization, once per org \
         across its registrable domains, and land it to organizations.whois (the \
         target_intel WHOIS coverage cell). Zero-touch HTTP. Args: organization_id. \
         Returns whether a whois record landed."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "organization_id":{"type":"string","description":"UUID of the org to look up WHOIS for"}},
            "required":["organization_id"]})
    }
    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let org_id = args.get("organization_id").and_then(Value::as_str)
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("organization_id (uuid) required"))?;
        let org = golish_db::repo::organizations::get_one(self.pool.as_ref(), org_id).await?
            .ok_or_else(|| anyhow::anyhow!("organization {org_id} not found"))?;
        let landed = crate::organization_recon::land_whois(self.pool.as_ref(), &org).await?;
        Ok(json!({"organization_id": org_id, "whois_landed": landed}))
    }
}
```
> 注：WHOIS 覆盖事实由 `coverage_truth` 读 whois 列投影（has_whois），故工具不需手写 coverage fact；落库即覆盖。
**验证：** `cargo check -p golish-recon-app` exit 0。
**提交：** `feat(recon): recon_lookup_whois tool (RDAP per-org)`

---

## Phase 3 · 注册 + 策略 + 删 enrich 引用

### Task 3.1 · 工具工厂
**文件：** `golish/src/pentest_tool_factory.rs:74-79` → 替换为：
```rust
tools.push(Arc::new(golish_recon_app::agent_tools::ReconMapAssetsTool::new(
    pool.clone(), tools_state.clone())));
tools.push(Arc::new(golish_recon_app::agent_tools::ReconLookupWhoisTool::new(pool.clone())));
```
**提交：** `feat(golish): register recon_map_assets + recon_lookup_whois, drop enrich`

### Task 3.2 · BridgeToolSelection
**文件：** `golish-agent-runtime/src/execution_mode/policy.rs`：字段 `recon_enrich_assets`→`recon_map_assets`，新增 `pub recon_lookup_whois: bool`；在 `all_enabled`(134)/`none`(154) 各加；`enabled_tool_names`(180) 两个 push；`modes/task.rs:74` 两个置 true；`prompt_render.rs:196` ToolRow 拆两行。
**验证：** `cargo check -p golish-agent-runtime` exit 0。
**提交：** `refactor(runtime): bridge tools = map_assets + lookup_whois`

### Task 3.3 · 证据记账 + 子代理工具清单
**文件：** `direct/mod.rs:467-471` match 臂改 `"recon_discover_subsidiaries" | "recon_map_assets" | "recon_lookup_whois"`；`builder/mod.rs:99/131` + `registry.rs:100/137` recon/pentester 清单换名 + 加 lookup_whois。
**验证：** `cargo check -p golish-agent-runtime -p golish-sub-agents` exit 0。
**提交：** `refactor(runtime,sub-agents): wire new recon tools, drop enrich refs`

---

## Phase 4 · 提示词 / 方法论

### Task 5 · recon 方法论改写
**文件：** `golish-sub-agents/src/defaults/prompts/execution_planning.rs`（102/112/113）+ `resources/harness/stages/target_intel.json`（methodology）。
**步骤：** 用顺序化流程替换「enrich 一调落全 6」表述：
```text
被动情报（每 org，按序）：
1. 基础画像：ENScan/0.zone → 公司信息/ICP/子公司/OSINT（intel）
2. 测绘：recon_map_assets（fofa/quake/hunter/shodan/0.zone）→ 域名/IP/ASN/子域/证书 + host↔IP 对（real_ip 落库）
3. WHOIS：recon_lookup_whois（按 org 一次 RDAP）→ organizations.whois
4. 补缺口：CT 用 ctfr（crt.sh）；DNS 用 dig；ASN 已由 map_assets 覆盖
5. submit_stage_deliverable 一次
注意：不再有 recon_enrich_assets；测绘与 WHOIS 是两个独立工具，各自有界、各自超时。
```
**验证：** `python3 -c "import json;json.load(open('resources/harness/stages/target_intel.json'))"` exit 0。
**提交：** `docs(harness): recon flow = map_assets + lookup_whois + ctfr/dig`

---

## Phase 5 · 测试 + 收口

### Task 6.1 · 改测试断言
**文件：** `defaults/tests.rs:120/144/167`、`prompt_render_tests.rs:24/81`、`stage_run/mod.rs:958/975`、`execute.rs:2943/2955`。
**步骤：** 把所有 `recon_enrich_assets` 断言改为 `recon_map_assets`（并在工具清单测里加 `recon_lookup_whois`）。新增一条 recon 工具集断言：`has_tool(recon,"recon_map_assets") && has_tool(recon,"recon_lookup_whois") && !has_tool(recon,"recon_enrich_assets")`。
**验证：** `cargo nextest run -p golish-recon-app -p golish-agent-runtime -p golish-sub-agents -p golish-agent-kit` 全绿。
**提交：** `test: update recon tool assertions (map_assets/lookup_whois)`

### Task 6.2 · 全局确认无残留 + 收口
**步骤：**
- `rg "recon_enrich_assets|ReconEnrichAssetsTool" backend/` → **零命中**（注释也清掉）。
- `just precommit` → 全绿。
- 活体（用户环境）：GUI 重跑「中国平安」target_intel：backend.log 无 `recon_enrich_assets timed out after 300s`；agent 依次调 `recon_map_assets`→`ctfr`→`dig`→`recon_lookup_whois`→submit；提交次数下降；DB 中 whois 仍真实 RDAP、asns 仍 provider 填、certificates 不再写垃圾。
**提交：** `chore: progress + feature_list for recon tool decomposition`

---

## 自检
1. **覆盖度：** 删 enrich(Task1.1/3.1/3.2/3.3/6.1) / 测绘工具(2.1) / RDAP 工具(2.2) / 注册(3.1) / 策略(3.2) / 记账+清单(3.3) / 提示(5) / 测试(6.1) / 残留扫描(6.2) 全有任务。
2. **占位符：** 无 TODO / 空测试。
3. **类型一致：** `land_whois`→`bool`（1.1）被 2.2 用；`CoverageLandingSummary{subdomains}`（1.2）一致；工具名 `recon_map_assets`/`recon_lookup_whois` 全程一致。

### Follow-up（独立小任务，本计划不含）
- domains 的 URL-IP 去重归一（属「规范资产身份」`docs/design/2026-06-18-canonical-asset-identity-and-coverage-join-key.md`，已设计未实现）。删 enrich crt.sh 后其主要危害（垃圾证书）已消除，故降为后续。
