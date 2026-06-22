# PR-B · `canonical_asset_key` 边界接入（E1 第二步）实现计划

> **状态**：实现中（2026-06-22 BajieAsk-agent-1 起）。**B0（gate 侧 join 归一）已实现 + 单测**；B1/B2 后续。
> 关联设计：`docs/design/2026-06-18-canonical-asset-identity-and-coverage-join-key.md` §5 PR-B。
> 前置：PR-A（`canonical_asset_key` 纯函数 + `AssetClass` 迁入 `golish-pentest-domain`，已落 `39bd87d6` / `af06f91b`）。
> 不变量：AGENTS.md I7 / I8（gate 判定不放松、checked_empty 仍要真证据）、I2（org 隔离不破）、§2.5（gate 语义变更先设计）。

---

## 0. 目标（一句话）

根治「**资产身份漂移 → coverage join 静默不命中 → 永判 `not_attempted` → 无限 `needs_fix`**」死循环（CT 死循环 / `http://x` ≠ `https://x` / 大小写 / FQDN 尾点都是它的子集），办法是让 join 两侧用**同一把规范钥匙** `canonical_asset_key` 对齐。

---

## 1. 切片划分（每片独立可回滚 + 单测可验）

- **B0 · gate 侧 join 归一（本计划首片，✅ 已实现 + 单测验证，672/672 绿）**：在 `golish-agent-kit/src/harness/gate/rule_engine.rs` 的 coverage join 两侧统一过一个**路径保留**的归一函数 `canon_asset`。**最小、最安全、纯单测可验（无需活体）**——任何上游写法都能对上，立即止血死循环。
  > **实现勘误（2026-06-22，单测实证）**：join **不能**直接用 `canonical_asset_key`——它把 URL 折叠到 host（`https://a.com/login` → `a.com`），会把 EAS/enumeration 的「URL 端点」与其「主机」**错误合并成一格**（`host_aware_coverage_relaxes_url_not_host_in_eas` 测当场红）。故 join 用 `canon_asset`：只抹 scheme/大小写/尾点，**保留 URL 路径**，资产粒度不变。`canonical_asset_key`（host-fold）仍用于 B1 的 intel 主机级源头归一。
- **B1 · 轴归一 + 去重**：
  - **B1a · gate 侧 in-scope 轴归一 + 去重（✅ 已实现 + 单测，674/674 绿）**：在 `coverage_complete` 构造资产轴后，按 `canon_asset` **去重**——把同一资产的漂移写法折叠成一行（保留首个原串供 `asset_types` 查表 + gap 消息）。EAS 的 URL 端点因 canon 保留路径仍与主机区分（含反作弊单测 `coverage_complete_dedup_preserves_distinct_eas_endpoint`）。
  - **B1b · DB 写路径 / 真值读取归一（后续，需 DB 谨慎 + 活体）**：`evidence_asset` 落库前归一、`coverage_truth` 投影按规范键 join，让**存库数据**本身就干净（为 PR-C/D `technique_outcomes` 物化表铺路）。
    > **勘误（2026-06-22）**：`in_scope_assets_impl`（`golish-agent-app/.../db_bridge/recon.rs:248`）的返回值**同时**喂给 gate 轴 **和** `coverage_truth` SQL（后者按 `targets.value` **原串** join 业务表）。所以**不能**在这一层直接归一（会打断 SQL join）。B1b 的正确做法是在**写入**目标/证据时落规范键（与 PR-C/D 同一刀），属 DB 写路径改动，需活体核 coverage_truth join 不回退——故本轮不与 gate-side B1a 混做。
- **B2 · recon 去重（✅ 已实现 + 验证，golish-recon-app 195/195 绿 + clippy 0）**：`normalized_host` 在 `runner.rs`（A）与 `persistence.rs`（B）**逐字节重复**——已把 A 改为**薄封装**委托 B（`super::persistence::normalized_host`），消除重复逻辑、行为逐字节不变。`persistence.rs` 那份是唯一真相源（含 www-去重语义，刻意不替换成 `canonical_asset_key`——后者红线不剥 `www.`，二者语义不同）。**范围外未动**：`is_known_public_non_asset_host` / `looks_like_domain` 在两文件也各有副本，属另一处 dedup，本片不扩范围。

---

## 2. B0 改动点（rule_engine.rs · 已实现）

join 的 6 处字面资产相等（identity drift 在此断裂）：

| # | 位置 | 原比较 |
|---|---|---|
| 1 | `coverage_complete` · `cell_status` | `c.asset == *asset` |
| 2 | `coverage_complete` · `has_fact` | `f.asset == *asset` |
| 3 | `coverage_complete` · `tagged_found`(claim) | `c.subject == *asset` |
| 4 | `coverage_complete` · `tagged_found`(finding) | `f.subject == *asset` |
| 5 | `coverage_corroborated`(claim) | `c.subject == cell.asset` |
| 6 | `coverage_corroborated`(finding) | `f.subject == cell.asset` |

→ 全部改为 `canon_asset(&x) == asset_key`。新增纯 helper `canon_asset(s)`：`trim` + `ascii_lowercase` + 去单个前导 scheme（`http(s)://…`，保留 host+path）+ 去 FQDN 尾点。**保留 URL 路径**（见上勘误），确定性。

**刻意不归一 axis 本身**：`assets` 轴保持原串（让 `asset_types.get(*asset)` 查表 + gap 消息原样），只在**比较点**两侧归一——把改动面收到最小。

---

## 3. 红线 / parity

1. `canon_asset` **只抹书写差异**（scheme/大小写/尾点）、保留路径、不截 apex、不剥 `www.` → **绝不把不同资产并成一格**（否则 = 新漏报）。`B0` 含反作弊单测 `coverage_complete_drift_does_not_over_merge_distinct_assets`：另一资产（`other.com`）的 fact 不满足本资产（`pingan.com`）。
2. 对已干净的资产串归一是**幂等**（`example.com` → `example.com`）→ 既有 663 测零回归（parity）。
3. checked_empty 仍要真 Empty 事实（I8 不破）；authoritative found 仍只认真值（仅匹配方式从字面等改为规范等）。

---

## 4. 验证

- `cargo nextest -p golish-agent-kit`（parity 全绿 + 新增 3 测）。
- `cargo clippy -p golish-agent-kit --all-targets -- -D warnings`。
- 活体（用户环境，B0+B1 后）：pingan `target_intel`，因身份漂移产生的 `never attempted (CT/SUBDOMAIN)` 消失、死循环不复发。

## 5. 回滚

B0 纯比较点归一：回退 = 还原 6 处 `==` 即可；无 schema、无接口、无跨 crate API 改动（`golish-pentest-domain` 依赖 PR-A 已加）。
