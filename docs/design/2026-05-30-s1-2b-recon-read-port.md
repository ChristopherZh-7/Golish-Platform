# S1-2b ReconPort 端口化设计

> 日期：2026-05-30
> 状态：Draft（待用户审查）
> 父设计：`docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`（本文是其 §6.1 路线图项 **b** 的细化）
> 前置：S1-2a `VaultReadPort` 已完成（commit `6abaec8` → `23e47a6` · feat/recon-service · 未 push）
> 范围：22 条 recon-related ALLOWLIST 跨 8 文件 6 消费方，按 plan §6 「按消费方子切」原则拆 6 子片
> **本文件只记录设计，不改任何代码。所有结论均带 `文件:行号` 证据。**

---

## 1. 背景与目标

### 1.1 S1-2a 留下的模式（可复用）

- **端口三件套**：trait（remote-ready 契约）+ in-proc 适配器（唯一调 `golish_db::repo::<svc>` 处）+ 消费方持 `Arc<dyn Port>`（构造点注入）
- **守卫配合**：`DOMAIN_RULES` 顶部加 `("ports/<svc>", "<svc>")` → 适配器域 = 提供方域，合法；同时删消费方 allowlist 条目 → ratchet 净前进
- **零业务语义变更**：端口方法逐字镜像现有 repo 调用，签名/出参一致
- **remote-ready 约束**：trait 入参/出参只用可序列化类型，禁 `PgPool`/闭包/`Arc<具体>`

### 1.2 S1-2b 的任务

按 `scripts/check_repo_ownership.py:130-156` 当前 ALLOWLIST 28 条，剔除 vault 已拔 2 条与 vuln/agent/pentest_plan 共 4 条（属 c/d/e），剩 **22 条 = recon repo 跨服务读 + 跨服务写**。引入 **`ReconPort`** trait + `PgReconAdapter` + DOMAIN_RULES `("ports/recon","recon")`，按 6 子片渐进消除，每子片独立 `just arch` 绿。

### 1.3 关键命名差异（与 S1-2a 不一致 · 必须明示）

S1-2a 的 `VaultReadPort` 因 store 路径 `ON CONFLICT DO NOTHING` 语义不可迁导致**端口降为只读**。S1-2b 不同：**`ai/db_bridge/recon.rs` 含 5 个 insert/upsert/update**（详 §3 证据），写路径就是 ALLOWLIST 命中源——端口必须含写方法，否则迁完不掉 allowlist。

**结论**：本端口命名为 **`ReconPort`**（不带 Read 后缀，含读+写）。S1-2c/d/e 的端口命名按各自实际范围再定（c/d 偏读 → `ReadPort`；e 含写 → 同 `Port`）。

### 1.4 非目标（本设计不做）

- 不动 recon 域内部读写（`tools/targets/{cmds,db,directory}.rs`、`tools/sensitive_scan.rs` 等 recon 域文件读 recon repo 不算跨服务，不在 ALLOWLIST）
- 不改 SQL / 业务语义 / `DbRepoProvider`（消费方端口已存在）
- 不下沉 trait 到 contract crate（YAGNI，阶段 4 再说）
- 不动 `tools/scan_queue.rs → scan_queue`（领域映射伪阳性，归 S1-2f）

---

## 2. 待消除耦合精确清单（证据：`scripts/check_repo_ownership.py:130-156`）

| # | 消费方文件 (域) | repo (recon) | 实际调用 (grep 实证) |
|---|---|---|---|
| 1 | `ai/db_bridge/recon.rs` (agent) | `api_endpoints` | `:66 insert`, `:197 list_by_target` |
| 2 | `ai/db_bridge/recon.rs` (agent) | `fingerprints` | `:132 upsert`, `:205 list_by_target` |
| 3 | `ai/db_bridge/recon.rs` (agent) | `js_analysis` | `:91 insert`, `:117 update_file_path`, `:212 list_by_target` |
| 4 | `ai/db_bridge/recon.rs` (agent) | `passive_scans` | `:159 insert`, `:219 list_by_target`, `:223 stats_by_target` |
| 5 | `ai/db_bridge/recon.rs` (agent) | `target_assets` | `:189 list_by_target` |
| 6 | `tools/security_analysis.rs` (pentest) | `api_endpoints` | `:150 list_by_target`, `:161 list_untested`, `:255 count_by_target` |
| 7 | `tools/security_analysis.rs` (pentest) | `fingerprints` | `:174 list_by_target` |
| 8 | `tools/security_analysis.rs` (pentest) | `js_analysis` | `:187 list_by_target` |
| 9 | `tools/security_analysis.rs` (pentest) | `passive_scans` | `:202 list_by_target`, `:214 list_by_url`, `:225 list_vulnerable`, `:236/:258 stats_by_target` |
| 10 | `tools/security_analysis.rs` (pentest) | `target_assets` | `:137 list_by_target`, `:251 count_by_target` |
| 11 | `tools/audit.rs` (platform) | `passive_scans` | `:134 list_global_by_project` |
| 12 | `tools/pentest_bridge/auth_probe.rs` (pentest) | `targets` | `:143 find_id_by_value_pair` |
| 13 | `tools/pentest_bridge/record_finding.rs` (pentest) | `targets` | `:128 find_id_by_value_or_name` |
| 14 | `tools/pentest_bridge/js_collect/sitemap.rs` (pentest) | `sitemap_store` | `:23 read_zap_sitemap`, `:86 delete_zap_sitemap` |
| 15 | `tools/pentest_bridge/js_collect/tool_impl.rs` (pentest) | `js_analysis` | `:343 update_file_path_by_url` |
| 16 | `tools/pentest_bridge/js_collect/tool_impl.rs` (pentest) | `targets` | `:86 find_id_by_value_pair` |
| 17 | `tools/pentest_bridge/js_extract_apis.rs` (pentest) | `js_analysis` | `:220 insert` |
| 18 | `tools/pentest_bridge/js_extract_apis.rs` (pentest) | `targets` | `:116 find_id_by_value_pair` |
| 19 | `tools/pipeline/storage.rs` (pentest) | `directory_entries` | `:182 exists_by_url_project` |
| 20 | `tools/pipeline/storage.rs` (pentest) | `sitemap_store` | `:270 read_zap_sitemap`, `:338 delete_zap_sitemap` |
| 21 | `tools/pipeline/storage.rs` (pentest) | `targets` | `:43 exists_by_value_exact` |
| 22 | `tools/vuln_intel/commands/matching.rs` (vuln) | `targets` | `:18 match_rows_legacy` |

**合计**：8 表 / 9 文件 / 6 域 / 22 条。

---

## 3. 端口方法清单（按表分组 · 唯一 `ReconPort` trait）

按 §2 实际命中的 repo 方法，端口需镜像 **18 个方法**（含读+写）。`golish-db/src/repo/<table>.rs` 各表方法清单见 `cargo doc` 或 grep。

| 表 | 端口方法（镜像 `golish_db::repo::<table>::*`） | 读/写 | 用于子片 |
|---|---|---|---|
| `targets` | `find_id_by_value_pair` | 读 | b3 |
| `targets` | `find_id_by_value_or_name` | 读 | b3 |
| `targets` | `match_rows_legacy` | 读 | b6 |
| `targets` | `exists_by_value_exact` | 读 | b4 |
| `target_assets` | `list_by_target` | 读 | b1 b2 |
| `target_assets` | `count_by_target` | 读 | b2 |
| `api_endpoints` | `insert` | 写 | b1 |
| `api_endpoints` | `list_by_target` | 读 | b1 b2 |
| `api_endpoints` | `list_untested` | 读 | b2 |
| `api_endpoints` | `count_by_target` | 读 | b2 |
| `js_analysis` | `insert` | 写 | b1 b3 |
| `js_analysis` | `update_file_path` | 写 | b1 |
| `js_analysis` | `update_file_path_by_url` | 写 | b3 |
| `js_analysis` | `list_by_target` | 读 | b1 b2 |
| `fingerprints` | `upsert` | 写 | b1 |
| `fingerprints` | `list_by_target` | 读 | b1 b2 |
| `passive_scans` | `insert` | 写 | b1 |
| `passive_scans` | `list_by_target` | 读 | b1 b2 |
| `passive_scans` | `list_by_url` | 读 | b2 |
| `passive_scans` | `list_vulnerable` | 读 | b2 |
| `passive_scans` | `stats_by_target` | 读 | b1 b2 |
| `passive_scans` | `list_global_by_project` | 读 | b5 |
| `sitemap_store` | `read_zap_sitemap` | 读 | b3 b4 |
| `sitemap_store` | `delete_zap_sitemap` | 写 | b3 b4 |
| `directory_entries` | `exists_by_url_project` | 读 | b4 |

**实际去重后 = 25 method（部分跨子片复用）**。端口 trait 文件预算 ~600 行（含 doc + tests），超过 architecture.md 500 行/文件预算时考虑按表拆 `ports/recon/{targets,assets,scans,…}.rs` 多文件。

---

## 4. 6 子片划分（每子片独立 PR + 独立 `just arch` 绿）

每子片单独 commit，按 plan §6.1 模式：①消费方迁端口 → ②（若新增 method）端口扩 trait + adapter → ③守卫删对应 ALLOWLIST 条目 → ④ `cargo check` + `just arch` 绿。

| 子片 | 消费方 | 涉及表 | ALLOWLIST 减 | 端口 method 净增 | 复杂度 | 备注 |
|---|---|---|---|---|---|---|
| **b1** | `ai/db_bridge/recon.rs` (agent) | api_endpoints/fingerprints/js_analysis/passive_scans/target_assets | 5 | +11 (5 表初始覆盖：5 读 + 4 写 + 2 stats) | 中 | **建端口骨架**+端口适配器+守卫 `("ports/recon","recon")`；DbRepoProvider 适配器内部，注入要看 trait 边界 |
| **b2** | `tools/security_analysis.rs` (pentest) | api_endpoints/fingerprints/js_analysis/passive_scans/target_assets (同 b1) | 5 | +5 (count/untested/list_by_url/list_vulnerable/count_by_target) | 中 | 大量复用 b1 读方法 |
| **b3** | `tools/pentest_bridge/{auth_probe,record_finding,js_collect/sitemap,js_collect/tool_impl,js_extract_apis}.rs` (pentest) | targets/sitemap_store/js_analysis | 6 | +5 (targets 3 find / sitemap 读+写 / js_analysis update_file_path_by_url) | **高（5 文件）** | 子片最大；端口写方法首次出现 |
| **b4** | `tools/pipeline/storage.rs` (pentest) | directory_entries/sitemap_store/targets | 3 | +2 (directory_entries::exists / targets::exists_by_value_exact)，sitemap 复用 b3 | 低 | 单文件 |
| **b5** | `tools/audit.rs` (platform) | passive_scans | 1 | +1 (list_global_by_project) | 低 | 单方法 |
| **b6** | `tools/vuln_intel/commands/matching.rs` (vuln) | targets | 1 | 0 (复用 b3 targets) | 低 | 跨域复用佐证（vuln 域文件经端口读 recon） |

**总计**：6 子片 / 22 条 allowlist / 24 method（去重）/ 9 文件。ALLOWLIST 28→6（剩 vuln 2 + agent_log 2 + pentest_plan 1 + scan_queue 1，归 c/d/e/f）。

> **建议执行顺序**：b1→b2→b3→b4→b5→b6。理由：① b1 立模式（最高 cognitive load）；② b2 大量复用 b1 method（信心建立）；③ b3 是 5 文件大片，等 b1/b2 模式稳后做；④ b4-b6 小片快速收尾。

---

## 5. ReconPort trait 雏形（架构层 · b1 plan 时定稿）

```rust
// backend/crates/golish/src/ports/recon/mod.rs
pub mod targets;
pub mod assets;
pub mod scans;
pub mod sitemap;

pub use targets::{ReconTargetsPort, PgReconTargetsAdapter};
// ... 同样导出 assets/scans/sitemap 子 port + adapter
```

**考量两个方案**（待用户拍板，§10 决策 1）：

- **方案 X · 单 `ReconPort` trait（25 method 平铺）**：复用 S1-2a 模式；消费方持 `Arc<dyn ReconPort>` 一个引用；缺点 trait 600 行
- **方案 Y · 按表分多个子 port（5-8 个 sub trait）**：`ReconTargetsPort` / `ReconAssetsPort` / `ReconScansPort` / `ReconSitemapPort` / `ReconDirectoryPort`；消费方按需注入 1-N 个；优点单文件小、职责清；缺点构造点要建多个 adapter

**推荐方案 Y**（5 个子 port，6 文件，每文件 < 200 行；构造点统一在 `pentest_bridge/mod.rs` 与 `ai/db_bridge/mod.rs` 各一处）。Y 更符合 plan §6 「按 consumer 子切」的精神（每子片只动相关 sub port）。

---

## 6. 守卫配合（继承 S1-2a 模式）

### 6.1 一次性加入（b1 commit 时）

`scripts/check_repo_ownership.py` `DOMAIN_RULES` 顶部加：

```python
("ports/recon",    "recon"),    # S1-2b
```

### 6.2 每子片删 ALLOWLIST 条目

| 子片 | 删 ALLOWLIST 条目 |
|---|---|
| b1 | 5 条 `("ai/db_bridge/recon.rs", "<table>")` |
| b2 | 5 条 `("tools/security_analysis.rs", "<table>")` |
| b3 | 6 条 `("tools/pentest_bridge/<file>.rs", "<table>")` |
| b4 | 3 条 `("tools/pipeline/storage.rs", "<table>")` |
| b5 | 1 条 `("tools/audit.rs", "passive_scans")` |
| b6 | 1 条 `("tools/vuln_intel/commands/matching.rs", "targets")` |

### 6.3 RAW_SQL_ALLOWLIST 影响

b 范围消费方多有 raw `sqlx::query`（属 P0-3），本片**不动**——继续保 `RAW_SQL_ALLOWLIST`。仅当某消费方所有 sqlx::query 都已被 repo 调用取代时才能从 RAW_SQL 删（如 b2 完成后 security_analysis.rs 若无残留 raw sql 则删；b1 后 recon.rs 若已无 sqlx 则删）。每子片实施计划单独评估。

---

## 7. 验证矩阵（每子片）

```bash
cd backend && cargo check -p golish              # 编译 (Tasks 中间态会 RED，子片末闭合)
cargo nextest run -p golish ports::recon         # 端口契约单测
grep -rn "golish_db::repo::<迁过的 table>" backend/crates/golish/src/<本子片文件>  # 应空
python3 scripts/check_repo_ownership.py          # OK clean
just arch                                         # 双守卫绿
just precommit                                    # 完成定义全套（仅最后一片或合并 PR 时跑，子片可暂用 cargo nextest -p golish）
```

---

## 8. 风险与缓解

| 风险 | 说明 | 缓解 |
|---|---|---|
| trait 过大（25 method） | 单 trait 600 行超预算 | 方案 Y 按表分子 port（推荐） |
| agent-bridge 是 DbRepoProvider 内部 | b1 改 ai/db_bridge/recon.rs 时要看是否影响外部 trait | b1 plan 阶段读 `golish_agent_kit::db_traits` 边界确认 |
| pentest_bridge 已有 PgVaultAdapter 注入 | b3 加 PgReconAdapter 注入时构造点已被 vault 改过，diff 集中 | `pentest_bridge/mod.rs:34-53` 加 `let recon_port =` 与 `vault_port` 并列，逐工具传 |
| sitemap_store 跨 b3/b4 | 同 read/delete 方法被两片消费方共用 | b3 实现 + b4 复用；端口 method 在 b3 加 |
| 命名差异（a 用 Read, b 不用）困惑后续 | c/d/e 命名规则需统一 | 本文 §1.3 + 父设计 §6 路线图各片备注 |
| b1 切到 PgReconAdapter 注入到 GolishDbRepoProvider 触发 trait 签名修改 | DbRepoProvider 是 trait（agent-kit L4a），实现 GolishDbRepoProvider 在 L6；如果只是 `impl GolishDbRepoProvider` 内部改走端口，**不动 trait 本身** | b1 实施时只改 `impl` 体；保留对外 trait 不变 |

---

## 9. 路线（b1 详细计划另文写）

b 完成后 ALLOWLIST 净减 22 条（28→6）。剩余：
- vuln (b/c 边界)：2 条 `ai/db_bridge/recon.rs → vuln_intel` + `ai/db_bridge/wiki.rs → wiki_kb` → **S1-2c**
- agent_log：2 条 `tools/audit.rs → agent_logs/search_logs` → **S1-2e**
- pentest_plan：1 条 `ai/db_bridge/orchestration.rs → execution_plans` → **S1-2d**
- scan_queue：1 条（伪阳性归属修正） → **S1-2f**

S1-2 总进度：a 完成（28→ S1-2 范围内 22）→ b 完成（22→0 实际 →0 真跨服务，6 剩属 c/d/e/f）。

---

## 10. 待用户拍板决策

1. **端口结构**（§5）：方案 X 单 `ReconPort` 25 method 平铺 / **方案 Y 按表分 5 子 port**（推荐）
2. **子片顺序**（§4 表）：建议 b1→b2→b3→b4→b5→b6
3. **trait 边界**（§8 风险 7）：b1 在 `impl GolishDbRepoProvider` 内部走端口，**不动 `DbRepoProvider` trait 本身**（推荐）
4. **写 method 命名**（§3）：直接镜像 repo 函数名（`insert` / `upsert` / `update_file_path` …）vs 端口语义化命名（`record_api_endpoint` / `upsert_fingerprint` …）。推荐直接镜像，与 S1-2a 模式一致、零业务语义变更
5. **b1 实施计划**：审过本设计后写 `docs/superpowers/plans/2026-05-30-s1-2b1-recon-port-agent-bridge.md`（5 条 allowlist · 5 表初始覆盖 · 端口骨架 + agent-bridge 迁移 + 守卫 · 4-5 Task code-complete）

---

## 11. 验证证据（设计阶段已完成）

- ALLOWLIST 22 条精确确认：`scripts/check_repo_ownership.py:130-156` × `rg golish_db::repo::(targets|target_assets|...) backend/crates/golish/src` 双向比对
- 消费方调用方法清单：上述 grep 实证（§2 表行号）
- recon repo 8 表的 `pub async fn` 清单：`rg '^pub async fn' backend/crates/golish-db/src/repo/{8 文件}` 实证（§3 表方法存在性）
- 命名差异（ReadPort vs Port）来源：S1-2a vault.rs:1-9 + agent-bridge writes grep 实证

---

> 本设计为 S1-2b 整体架构。审过后按本文 §10 决策结果，写 `docs/superpowers/plans/2026-05-30-s1-2b1-recon-port-agent-bridge.md` 执行 b1（`.cursor/skills/executing-plans/`），遵守 AGENTS.md §3：每子片有 evidence、`just arch` 绿、`just precommit` 在合并 PR 前全绿。
