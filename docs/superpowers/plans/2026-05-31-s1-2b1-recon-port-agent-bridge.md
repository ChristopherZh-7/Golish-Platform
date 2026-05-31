# S1-2b1 —— ReconPort 骨架 + agent-bridge 迁移（层次 B 首片）

> 父设计：`docs/design/2026-05-30-s1-2b-recon-read-port.md`（§10 决策已由用户拍板：方案 Y / b1→b6 / 不动 DbRepoProvider trait / 镜像 repo 名 / 端口家 = app-core）。
> **关键适配（设计已过期修正）**：S1-2b 设计写于层次 A crate 拆分之前，假设端口放 `golish/src/ports/recon`。现消费方已分散到 4 个 app crate，端口必须放 **`golish-app-core/src/ports/recon/`**（VaultReadPort 同款位置；app-core 依赖 golish-db，PgReconAdapter 可调 `golish_db::repo::recon`）。

## 1. b1 范围
消费方：`golish-agent-app/src/ai/db_bridge/recon.rs`（`GolishDbRepoProvider` 的 5 个 recon 表调用，10 个 repo 调用点）。剔除同文件的 vuln_intel（S1-2c）/ audit（SHARED）调用，不动。

## 2. 端口结构（方案 Y · b1 建 2 个 sub-port）
`golish-app-core/src/ports/recon/`：
- `mod.rs` — `pub mod scans; pub mod assets;` + re-export
- `scans.rs` — `ReconScansPort` + `PgReconScansAdapter`（api_endpoints + js_analysis + fingerprints + passive_scans，10 method）
- `assets.rs` — `ReconAssetsPort` + `PgReconAssetsAdapter`（target_assets，1 method）

（targets/sitemap/directory sub-port 留 b3/b4。）

## 3. 端口方法（逐字镜像 repo 签名，去 pool，返回同 Row 类型；D4 镜像名）
ReconScansPort（method 名 = `<table>_<fn>`）：
- `api_endpoints_insert(target_id, project_path:Option<&str>, url, method, path, params:&Value, headers:&Value, auth_type:Option<&str>, source, risk_level) -> ApiEndpoint`
- `api_endpoints_list_by_target(target_id) -> Vec<ApiEndpoint>`
- `js_analysis_insert(target_id, project_path:Option<&str>, url, filename, size_bytes:Option<i64>, hash_sha256:Option<&str>, frameworks:&Value, libraries:&Value, endpoints_found:&Value, secrets_found:&Value, comments:&Value, source_maps:bool, risk_summary, raw_analysis:&Value) -> JsAnalysisResult`
- `js_analysis_update_file_path(id, file_path) -> ()`
- `js_analysis_list_by_target(target_id) -> Vec<JsAnalysisResult>`
- `fingerprints_upsert(target_id, project_path:Option<&str>, category, name, version:Option<&str>, confidence:f32, evidence:&Value, cpe:Option<&str>, source) -> Fingerprint`
- `fingerprints_list_by_target(target_id) -> Vec<Fingerprint>`
- `passive_scans_insert(target_id, project_path:Option<&str>, test_type, payload, url, parameter, result, evidence, severity, tool_used, tester, notes, detail:&Value) -> PassiveScanLog`
- `passive_scans_list_by_target(target_id, limit:i64) -> Vec<PassiveScanLog>`
- `passive_scans_stats_by_target(target_id) -> serde_json::Value`

ReconAssetsPort：
- `target_assets_list_by_target(target_id) -> Vec<TargetAsset>`

Row 类型（`golish_db::models::*`）全部派生 `Serialize, Deserialize`（remote-ready ✓）。

## 4. 消费方接线（D3：不动 DbRepoProvider trait，只改 impl）
- `GolishDbRepoProvider` 加字段 `recon_scans: Arc<dyn ReconScansPort>` + `recon_assets: Arc<dyn ReconAssetsPort>`；`new(pool)` 内部 `PgReconScansAdapter::new(pool.clone())` 等构造（外部 `new(pool)` 签名不变 → 零调用方改动）。
- `recon.rs` 10 个调用点：`golish_db::repo::<table>::<fn>(&self.pool, args)` → `self.recon_<scans|assets>.<table>_<fn>(args)`（去 pool，返回/`to_value` 处理不变）。

## 5. 守卫
- `check_repo_ownership.py` DOMAIN_RULES 顶部加 `("ports/recon", "recon")`（app-core 的适配器域 = recon，合法）。
- 删 5 条 ALLOWLIST：`("golish-agent-app/ai/db_bridge/recon.rs", "{api_endpoints,fingerprints,js_analysis,passive_scans,target_assets}")`。

## 6. 验证
- `cargo check -p golish-app-core` + `cargo check -p golish-agent-app` + `cargo check -p golish` exit 0
- `cargo nextest run -p golish-app-core ports::recon`（object-safe 测试）
- `rg "golish_db::repo::(api_endpoints|js_analysis|fingerprints|passive_scans|target_assets)" golish-agent-app/src/ai/db_bridge/recon.rs` → 空
- `python3 scripts/check_dag.py` + `check_repo_ownership.py` exit 0
- clippy app-core + agent-app + golish --lib `-D warnings`

## 7. 不在 b1
b2-b6（security_analysis / pentest_bridge / pipeline / audit / vuln matching）；RAW_SQL allowlist 不动（recon.rs 仍有 vuln/audit 调用）。
