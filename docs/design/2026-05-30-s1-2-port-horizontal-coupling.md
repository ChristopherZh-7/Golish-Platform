# S1-2 端口化横向耦合（provider-side service ports）设计

> 日期：2026-05-30
> 状态：Draft（待用户审查）
> 来源：servitization 路线图阶段 1 第二项；用户指令「开始写 S1-2 计划」（2026-05-30，主控 MCP-1 直接执行——窗口内仅本会话在线）
> Relates to:
> - `docs/design/2026-05-30-servitization-readiness.md`（§3.1/§3.3 阻碍、§4 目标架构、§6 阶段1 S1-2，本文是其 S1-2 的细化设计）
> - `docs/superpowers/plans/2026-05-30-s1-1-repo-data-ownership-boundary.md`（S1-1，本文的前置）
> - `scripts/check_repo_ownership.py`（S1-1 守卫；其 `ALLOWLIST` 就是本文的待消除清单）
> - `AGENTS.md` §5 不变量 I1（错误码契约）/ I2（IDOR）/ I5（ts-rs 同步）/ I9（事务内不调外部）
> 范围：`backend/crates/golish/src`（命令/适配层）+ `golish-db` repo（被读方），零业务语义变更。
> **本文件只记录设计，不改任何代码。所有结论均带 `文件:行号` 证据。**

---

## 1. 背景与目标

### 1.1 S1-1 留下了什么

S1-1 建立了**数据所有权守卫** `scripts/check_repo_ownership.py`：每个 `golish-db` repo 归属唯一服务（recon / vuln / pentest / agent / platform），命令层模块只能读自己服务的 repo，现有跨服务直读冻结进 `ALLOWLIST`（ratchet），CI 只拦新增。S1-1 脚本头注释已写明：

> Each allowlist entry is a future `*Port` extraction candidate (design: docs/design/2026-05-30-servitization-readiness.md §6 S1-2). REMOVING an entry means you introduced the corresponding `*Port`.
> —— `scripts/check_repo_ownership.py:9-10,122-123`

**S1-2 的任务就是逐条拔掉 `ALLOWLIST`**：把命令层的跨服务直读，改为走一个**提供方服务端口（provider-side service port）**，使「读别的服务的表」变成「调别的服务的接口」。每拔一条，ratchet 前进一格，DB-per-service 抽服务（阶段 4）就少一处硬耦合。

### 1.2 目标

1. 把当前 30 条 `ALLOWLIST`（= 29 条真实跨服务读 + 1 条领域映射伪阳性，见 §3）按**提供方服务**归类，定义 5 个 `*Port` trait。
2. 确立 **provider-side port 的落地模式**（trait 位置、in-proc 适配器、组装根注入、守卫如何配合），并按 `remote-ready` 约束（DTO 可序列化、不传 `Arc<PgPool>`/闭包、错误 `{code,message}`）设计，使阶段 4 换网络实现时只改适配器、不动业务。
3. 切出一个**走路骨架（walking skeleton）S1-2a = `VaultReadPort`**，端到端跑通模式（最小、可验证、安全语义敏感），其余 4 个端口按本文路线在各自计划里复用此模式。

### 1.3 非目标

- 不在本文件改任何代码（设计先行，AGENTS.md §1.3）。
- 不改业务语义：所有端口方法**逐字镜像**现有 repo 调用的入参/出参/SQL 语义，只换调用路径。
- 不在本轮把 trait 下沉到独立 contract crate（YAGNI）——端口先放在 `golish` crate 内 `src/ports/<service>/`，等阶段 4 出现真实网络实现时再下沉（§4.4）。
- 不做物理 repo 子模块重排（那是 S1-1b）、不清裸 SQL 基线（那是 P0-3，但 §3.4 说明二者在 vault 切片的交叠处理）。
- 不碰 `asset_intel → organizations/pentest` 的**模块 import 耦合**（servitization §3.3）——那是**另一条轴**（编译期 `use crate::tools::…`，非 repo 读），见 §6.4 单列。

---

## 2. 关键洞察：两层端口，别重复造轮子

阅读现有代码发现 **agent 栈已经有一层端口**，不能把它和 S1-2 要加的端口混为一谈：

### 2.1 已存在的「消费方端口」`DbRepoProvider`（不动它）

`golish-agent-kit`（L4a）定义了 `DbRepoProvider` trait（`golish_agent_kit::db_traits`），让 agent 核心不直接依赖 `golish-db`。app 层（L6）的 `GolishDbRepoProvider` 实现它：

```rust
// backend/crates/golish/src/ai/db_bridge/mod.rs:16,24-35
use golish_agent_kit::db_traits::*;
pub struct GolishDbRepoProvider { pool: Arc<PgPool> }
#[async_trait] impl DbRepoProvider for GolishDbRepoProvider { … }
```

这是**消费方（agent）侧**的端口：agent 核心只见 trait，不见 `golish-db`。**这层是对的，S1-2 不动它。**

### 2.2 缺失的「提供方服务端口」（S1-2 要加的）

问题在于 `GolishDbRepoProvider` 这个**适配器自身**的实现体，直接横跨多个服务的 repo：

```rust
// backend/crates/golish/src/ai/db_bridge/recon.rs:17,189,197 …（agent 域文件）
golish_db::repo::vuln_intel::search_entries(&self.pool, …)      // → vuln
golish_db::repo::target_assets::list_by_target(&self.pool, …)   // → recon
golish_db::repo::api_endpoints::list_by_target(&self.pool, …)   // → recon
```

守卫据此把 `ai/db_bridge/recon.rs`（DOMAIN_RULES `("ai/","agent")`，`scripts/check_repo_ownership.py:114`）判为 agent，读 recon/vuln repo = 跨服务，进 `ALLOWLIST`。

> **纯六边形视角**：组装根的适配器横跨 repo 本身不算错。**但从 DB-per-service 视角**：将来 vuln 库变远程，这个**单一适配器**没法再 `golish_db::repo::vuln_intel::…` 直查——它必须改调一个 **vuln 服务端口**（in-proc impl → network impl）。

**结论**：S1-2 引入的是**提供方服务端口**——按「被读的是哪个服务的表」定义 `ReconReadPort` / `VaultReadPort` / `VulnReadPort` / …，让任意消费方（命令、pentest 工具、agent 适配器、audit 聚合）经它读取，而不是直查别的服务 repo。

---

## 3. 待消除耦合清单（按提供方服务归类）

### 3.1 数据来源

`ALLOWLIST` 当前 30 条（`scripts/check_repo_ownership.py:124-157`）。每条 = `(调用方文件, 被读 repo)`。按 `REPO_OWNER`（同文件 36-84 行）把被读 repo 映射回**提供方服务**，按调用方文件路径（`DOMAIN_RULES`，92-119 行）映射回**消费方服务**：

| # | 调用方文件（消费方域） | 被读 repo（提供方服务） | 归入端口 |
|---|---|---|---|
| 1 | `ai/db_bridge/orchestration.rs`（agent） | `execution_plans`（pentest） | `PentestPlanReadPort` |
| 2-6 | `ai/db_bridge/recon.rs`（agent） | `api_endpoints`/`fingerprints`/`js_analysis`/`passive_scans`/`target_assets`（recon） | `ReconReadPort` |
| 7 | `ai/db_bridge/recon.rs`（agent） | `vuln_intel`（vuln） | `VulnReadPort` |
| 8 | `ai/db_bridge/wiki.rs`（agent） | `wiki_kb`（vuln） | `VulnReadPort`（wiki） |
| 9,11 | `tools/audit.rs`（platform） | `agent_logs`/`search_logs`（agent） | `AgentLogReadPort` |
| 10 | `tools/audit.rs`（platform） | `passive_scans`（recon） | `ReconReadPort` |
| 12 | `tools/pentest_bridge/auth_probe.rs`（pentest） | `targets`（recon） | `ReconReadPort` |
| 13 | `tools/pentest_bridge/auth_probe.rs`（pentest） | `vault`（platform） | **`VaultReadPort`** |
| 14 | `tools/pentest_bridge/js_collect/sitemap.rs`（pentest） | `sitemap_store`（recon） | `ReconReadPort` |
| 15-16 | `tools/pentest_bridge/js_collect/tool_impl.rs`（pentest） | `js_analysis`/`targets`（recon） | `ReconReadPort` |
| 17-18 | `tools/pentest_bridge/js_extract_apis.rs`（pentest） | `js_analysis`/`targets`（recon） | `ReconReadPort` |
| 19 | `tools/pentest_bridge/record_finding.rs`（pentest） | `targets`（recon） | `ReconReadPort` |
| 20 | `tools/pentest_bridge/vault_ops.rs`（pentest） | `vault`（platform） | **`VaultReadPort`** |
| 21-23 | `tools/pipeline/storage.rs`（pentest） | `directory_entries`/`sitemap_store`/`targets`（recon） | `ReconReadPort` |
| 24 | `tools/scan_queue.rs`（recon） | `scan_queue`（vuln） | **映射伪阳性**（§3.3） |
| 25-29 | `tools/security_analysis.rs`（pentest） | `api_endpoints`/`fingerprints`/`js_analysis`/`passive_scans`/`target_assets`（recon） | `ReconReadPort` |
| 30 | `tools/vuln_intel/commands/matching.rs`（vuln） | `targets`（recon） | `ReconReadPort` |

### 3.2 五个端口的规模

| 端口 | 提供方 | 条目数 | 涉及 repo | 消费方 | 复杂度 |
|---|---|---|---|---|---|
| `ReconReadPort` | recon | **22** | targets, target_assets, api_endpoints, js_analysis, fingerprints, passive_scans, sitemap_store, directory_entries | agent / pentest / vuln / platform | 高（最大，建议按消费方再切） |
| `VaultReadPort` | platform | 2 | vault | pentest | 低（**走路骨架**，安全语义敏感） |
| `VulnReadPort` | vuln | 2 | vuln_intel, wiki_kb | agent | 低 |
| `AgentLogReadPort` | agent | 2 | agent_logs, search_logs | platform(audit) | 低 |
| `PentestPlanReadPort` | pentest | 1 | execution_plans | agent | 低 |

> recon 是被依赖最多的服务（22/29），与 servitization §5「最适合**最后**抽 recon、最先抽 vuln」一致——recon 端口最大、最该先把模式跑顺再上。

### 3.3 第 24 条是映射伪阳性，不是端口

`tools/scan_queue.rs` 被 `DOMAIN_RULES ("tools/scan_queue","recon")`（脚本 99 行）判为 recon，但它读的 `scan_queue` repo 被 `REPO_OWNER` 判为 vuln（脚本 54 行）。**同一个 `scan_queue` 概念被一分为二**——文件管理的就是这张表，本质是**领域归属标注冲突**，不是真实跨服务读。

**修法（S1-2f 清理，非端口）**：二选一——① 把 `scan_queue` repo 的 owner 从 vuln 改 recon（若扫描队列属攻面侧）；或 ② 把 `tools/scan_queue.rs` 的 DOMAIN_RULES 改 vuln（若属漏洞侧）。需用户/领域确认归属后单行改 + 删该 allowlist 条目。**因此真实待端口化耦合是 29 条。**

### 3.4 与 P0-3 裸 SQL 基线的交叠（vault 切片特例）

`vault_ops.rs` 的 `store` 动作用裸 `sqlx::query` INSERT（`tools/pentest_bridge/vault_ops.rs:122-137`），故同时在 `RAW_SQL_ALLOWLIST`（脚本 180 行）。S1-2a 迁移 vault 读路径时，顺手把这条裸 INSERT 也改走端口的 `store_entry`（底层调已存在的 `golish_db::repo::vault::insert_full`，`golish-db/src/repo/vault.rs:120`），**一并拔掉 vault_ops.rs 的 RAW_SQL_ALLOWLIST 条目**——一个切片同时演示两类 ratchet 的前进。

---

## 4. 目标架构

### 4.1 端口模式（三件套）

每个提供方服务端口由三部分构成：

```text
  消费方（命令/工具/适配器）        端口 trait（抽象）         in-proc 适配器（实现）
  ───────────────────────         ──────────────────        ───────────────────────
  VaultTool { port: Arc<dyn        trait VaultReadPort {      struct PgVaultAdapter {
    VaultReadPort> }                 async fn …() -> …;          pool: Arc<PgPool> }
        │                          }                          impl VaultReadPort for
        └── port.get_secret(...)        ▲                       PgVaultAdapter {
            （不再 use golish_db）       │ 注入                    golish_db::repo::vault::…
                                        └────────────────────── }（platform 域，合法）
```

- **trait**：只暴露**本服务**的读/写方法。入参/出参用可序列化类型（`String`/`Uuid`/`serde_json::Value`/DTO），**不**出现 `PgPool`/`&self.pool`/闭包 → remote-ready。
- **in-proc 适配器**：唯一允许 `use golish_db::repo::<本服务repo>` 的地方；逐字搬运现有 repo 调用。
- **注入**：消费方持 `Arc<dyn Port>`，在组装根用 in-proc 适配器构造（§4.3）。阶段 4 换成 `NetworkVaultAdapter` 即可。

### 4.2 端口与适配器放哪：`golish/src/ports/<service>/`

- 新目录 `backend/crates/golish/src/ports/`，按提供方服务分子目录：`ports/platform/vault.rs`、`ports/recon/…`、`ports/vuln/…`、`ports/agent/…`、`ports/pentest/…`。
- **trait 与 in-proc 适配器同文件**（小切片）或拆 `trait.rs` + `pg.rs`（大端口如 recon），遵守 architecture.md 500 行/文件预算（`docs/architecture.md:267-269`）。
- **不新建 crate**：消费方都在 `golish`（L6），trait 放 L6 内即可解耦调用点；阶段 4 真要网络实现再下沉到 contract crate（§4.4）。

### 4.3 守卫如何配合（关键机制）

适配器文件会写 `golish_db::repo::vault`，必须让守卫认它**合法**而不是又进 allowlist。机制：**适配器归属它服务的域**。在 `DOMAIN_RULES` 顶部（first-match-wins，需在 `("ai/","agent")` 等泛规则**之前**）按服务加映射：

```python
# scripts/check_repo_ownership.py DOMAIN_RULES 顶部新增（每上线一个端口加一条）
("ports/platform", "platform"),   # S1-2a
("ports/recon",    "recon"),      # S1-2b
("ports/vuln",     "vuln"),       # S1-2c
("ports/agent",    "agent"),      # S1-2e
("ports/pentest",  "pentest"),    # S1-2d
```

于是 `ports/platform/vault.rs → repo::vault` 是 platform→platform = **owner==domain 合法**，无需 allowlist；同时消费方文件不再 `use golish_db::repo::vault` → 其 allowlist 条目删除。**ratchet 净前进。**

> 端口 trait 文件（不碰 repo）落在哪个域都行；只有**适配器**（碰 repo）必须落在提供方域目录下。

### 4.4 何时下沉到 contract crate（阶段 4，非本轮）

当某服务要真正远程化：把它的 `*Port` trait 移到一个 L1/L2 contract crate（如复用 `golish-pentest-domain` / `golish-vuln-intel-domain`，`docs/architecture.md:105-106`），消费方与该服务都依赖 contract crate；in-proc 适配器留桌面端、network 适配器进瘦客户端。本轮**不做**，trait 留 `golish` 内。

---

## 5. 走路骨架 S1-2a：`VaultReadPort`

选 vault 当骨架的理由：① 小（2 条 allowlist + 1 条 raw-sql，1 个 repo）；② 安全语义敏感（凭据访问本就该是受控接缝，契合渗透平台定位）；③ 读写都有，能同时演示两类 ratchet。

### 5.1 端口方法（逐字镜像现有调用）

| 方法 | 镜像的 repo 调用（证据） | 出参 |
|---|---|---|
| `list_name_meta_by_project(project_path)` | `repo::vault::list_name_meta_by_project`（`vault.rs:314`） | `Vec<(String,String,String,String)>` |
| `get_secret_by_name_project(name, project_path)` | `repo::vault::get_secret_by_name_project`（`vault.rs:330`） | `Option<(String,String,String)>` |
| `get_value_by_name_project(name, project_path)` | `repo::vault::get_value_by_name_project`（`vault.rs:347`） | `Option<String>` |
| `store_entry(id,name,entry_type,enc_value,username,notes,project,project_path)` | `repo::vault::insert_full`（`vault.rs:120`），替换 vault_ops.rs 裸 INSERT（`vault_ops.rs:122`） | `()` |

> 加密/解密（`vault::obfuscate_value`/`deobfuscate_value`）仍留消费方，端口只搬「读/写已加密值」——与现状逐字一致，零语义变更。

### 5.2 消费方改造（2 个文件，删 2 条 allowlist + 1 条 raw-sql）

- `tools/pentest_bridge/vault_ops.rs`：`VaultTool` 增 `port: Arc<dyn VaultReadPort>`；`list`/`get` 改 `self.port.…`；`store` 的裸 INSERT 改 `self.port.store_entry(…)`；删 `use golish_db::repo::vault` 直查。
- `tools/pentest_bridge/auth_probe.rs`：token 解析处 `golish_db::repo::vault::get_value_by_name_project`（`auth_probe.rs:253`）改 `self.port.get_value_by_name_project(…)`。

构造点（`VaultTool::new` / auth probe 构造）在组装根注入 `Arc::new(PgVaultAdapter::new(pool))`——具体注入点由实现计划 grep 定位。

### 5.3 守卫与验证

1. `DOMAIN_RULES` 加 `("ports/platform","platform")`；
2. 删 `ALLOWLIST` 两条 vault + `RAW_SQL_ALLOWLIST` 一条 `vault_ops.rs`；
3. `just arch` → exit 0（适配器 platform→platform 合法、消费方不再直查）；
4. `cargo nextest -p golish`（含 `golish-db` vault SQL 一致性测试 `vault.rs:360-378` 不受影响）；
5. 加一条端口契约单测：in-proc 适配器对一个临时 PG 的 round-trip（store→get）等价于旧路径。

---

## 6. 切片路线（每片独立 PR · 各自写实现计划）

| 切片 | 端口 | allowlist 净减 | 依赖 | 说明 |
|---|---|---|---|---|
| **S1-2a** | `VaultReadPort` | 2（+1 raw-sql） | 无 | **本轮出详细计划**；走路骨架，立模式 |
| S1-2b | `ReconReadPort` | 22 | S1-2a 模式 | 最大；建议按消费方子切（agent-bridge / pentest-bridge / security_analysis / pipeline / audit / vuln-matching） |
| S1-2c | `VulnReadPort`(+wiki) | 2 | S1-2a | agent 适配器 → vuln |
| S1-2d | `PentestPlanReadPort` | 1 | S1-2a | agent 适配器 → execution_plans |
| S1-2e | `AgentLogReadPort` | 2 | S1-2a | audit → agent 日志 |
| S1-2f | （清理，非端口） | 1 | 无 | scan_queue 映射归属修正（§3.3），需领域确认 |
| S1-2g | （另一条轴） | 0（不在 allowlist） | 可并 S3 | `asset_intel` 模块 import 解耦（servitization §3.3），见 §6.4 |

**完成判据**：`ALLOWLIST` 由 30 → 1（仅剩或清掉 scan_queue 伪阳性），守卫常绿，零业务语义变更。

### 6.4 模块 import 耦合（S1-2g，单列说明）

servitization §3.3 的 `asset_intel/mod.rs:27,30` `use crate::tools::{organizations,pentest}::…` 是**编译期模块耦合**，守卫（只扫 `golish_db::repo::`）抓不到，与本文 29 条 repo 耦合**不同机制**。它更适合在 S3「碎 god-crate」时随 `golish` 拆分一起处理，或单开小切片用同样的 port 思路（`OrganizationsPort`/`PentestPort`）。本轮**不纳入** S1-2a-f 主线，仅登记备忘。

---

## 7. remote-ready 约束（每个端口都遵守）

| 约束 | 原因 | 检查 |
|---|---|---|
| 端口方法签名只用可序列化类型，禁 `PgPool`/`&Pool`/闭包/`Arc<具体>` | 阶段 4 要能跨网络 | code review + 适配器是唯一碰 `golish-db` 处 |
| 错误统一 `{code,message}`（I1）而非裸 `anyhow` 漏到契约 | 全栈错误码契约 | 端口返回 `Result<_, GolishError>`（骨架可先 `anyhow`，B 切片起收口，见 §9 决策） |
| 跨域读仍保持 IDOR 的 project 作用域（I2） | 渗透平台不能被绕 | 端口方法保留 `project_path` 入参，镜像现有 scoped repo |
| 事务内不经端口发外部调用（I9） | 阶段 4 端口可能变网络 | 端口调用点不得在 `BEGIN…COMMIT` 内 |
| 跨 IPC 的 DTO 用 ts-rs（I5） | 类型单源 | 仅当端口出参回传前端时适用；纯后端端口不强制 |

---

## 8. 风险与回滚

| 风险 | 说明 | 缓解 |
|---|---|---|
| 注入改构造签名波及面 | `VaultTool::new` 等构造点散布 | 端口适配器由现有 `Arc<PgPool>` 就地构造，构造方只多传一个 `Arc<dyn Port>`；逐文件迁移 |
| 端口语义与 repo 漂移 | 手抄方法易错 | 逐字镜像 + round-trip 契约单测 + `golish-db` 既有 SQL 一致性测试兜底 |
| recon 端口过大单 PR | 22 条一次改 | S1-2b 强制按消费方子切，每子片独立 `just arch` 绿 |
| 守卫 DOMAIN_RULES 顺序错 | `ports/` 规则放在泛规则后会被 `ai/` 等抢先匹配 | 必须插在 `DOMAIN_RULES` 顶部（first-match-wins，脚本 92 行注释） |
| scan_queue 归属误判 | 改错方向引入真耦合 | §3.3 标为需用户确认，单行可逆 |

**统一回滚**：每个端口=新增文件（trait+适配器）+ 删 allowlist 条目 + 改少量消费方调用；单 PR revert 即恢复直查，主链路不受影响。

---

## 9. 待用户拍板的决策

1. **切片顺序确认**：是否同意 **S1-2a(Vault) 先行立模式 → S1-2b(Recon, 子切) → c/d/e → f 清理**？（recon 最大放第二，骨架先小）
2. **端口错误类型**：骨架 S1-2a 端口方法返回 `anyhow::Result`（与现有 repo/工具一致，零额外改动），还是从骨架就上 `Result<_, GolishError>`（I1 契约，但要先确认 `GolishError` 在 L6 可用且不污染端口）？建议：**骨架用 `anyhow`，在 S1-2b 起逐步收口到 GolishError**（避免骨架被错误类型设计拖大）。
3. **trait 位置**：同意先放 `golish/src/ports/<service>/`（不新建 crate），阶段 4 再下沉 contract crate？
4. **scan_queue 归属**（§3.3）：`scan_queue` 属 **recon**（攻面扫描队列）还是 **vuln**（漏洞扫描队列）？决定 S1-2f 改哪一行。
5. **焦点冲突**：`feature_list.json` 当前唯一 `in_progress` 是 `target-surface-workbench`（前端工作台，`feature_list.json:69-74`）。S1-2 是否顶上来当 `in_progress`，还是先以 `not_started` 登记、等 workbench 收口？（AGENTS.md §2.1 同时只能一个 in_progress）

---

> 本设计为 S1-2 的细化。审查通过后，按 `docs/superpowers/plans/2026-05-30-s1-2-portification.md` 执行 S1-2a 走路骨架（`.cursor/skills/executing-plans/`），遵守 AGENTS.md §3 完成定义：没有新鲜验证证据不许宣称完成。
