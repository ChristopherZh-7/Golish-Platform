# PR-A · 规范资产身份 `canonical_asset_key` 实现计划

> **状态更新（2026-06-22 · 核当前代码 + git log）**：✅ **已落地**。commits `39bd87d6`（canonical_asset_key + migrate AssetClass）+ `af06f91b`（re-export AssetClass from pentest-domain）。**注意**：本 PR 故意零接入 → PR-B（接 `in_scope_assets`/evidence 落库/gate join）+ PR-C/D（`technique_outcomes` 物化表）**仍未做**，身份漂移死循环 bug 需 PR-B 才消除（`canonical_asset_key` 当前零调用方）。

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans` 逐任务实现此计划；按 `.cursor/skills/test-driven-development` 先写测试。
> 关联设计：`docs/design/2026-06-18-canonical-asset-identity-and-coverage-join-key.md`（§3.1 函数规则、§4 红线、§5 PR-A、§8 决策 D1/D5）。
> 决策落地：**D1** = 函数与 `AssetClass` 同住 `golish-pentest-domain`（纯逻辑零 I/O，无循环依赖）。**D5** = E1 先行；本计划是整个序列的第一步（PR-B 边界接入、PR-C/D/E 物化表为后续独立计划，见文末「后续计划」）。

**目标：** 在 `golish-pentest-domain` 新增一个确定性纯函数 `canonical_asset_key(value) -> Option<AssetKey>`，把「一个资产的规范字符串身份」收成唯一一把钥匙；并把现住 `golish-agent-kit` 的 `AssetClass`（枚举 + 纯分类方法）迁入同 crate，原处改为 `pub use` 重导出以保证零调用方改动。
**架构：** `canonical_asset_key` 把任意资产值归一为「主机身份」（域名/IP/CIDR）：小写、去 FQDN 尾点、URL 取 host、IP/CIDR 规范文本。**故意不做** apex 截断、**不剥** `www.`（否则把不同资产并成一格 = 新漏报）。URL 折叠到其 host 身份（intel 技术按 host 判定）。本 PR **不接任何调用方**（PR-B 才接）→ 除「`AssetClass` 换源 + 重导出」外零行为变化。
**技术栈：** Rust 2021、crate `golish-pentest-domain`（仅依赖 serde/serde_json/ts-rs）；`cargo nextest`。

---

## 文件结构（创建/修改 + 职责）

| 文件 | 改动 | 职责 |
|---|---|---|
| `backend/crates/golish-pentest-domain/src/asset_id.rs` | **新建** | `AssetClass` 枚举 + 纯分类方法（`from_target_type`/`from_value`/`is_url_wrapped_ip`/`classify`/`maybe_web`，自 agent-kit 迁入）；`AssetKey` 结构；`canonical_asset_key` + 私有 `extract_host`；全部单测 |
| `backend/crates/golish-pentest-domain/src/lib.rs` | 改 | 加 `pub mod asset_id;` + `pub use asset_id::{AssetClass, AssetKey, canonical_asset_key};` |
| `backend/crates/golish-agent-kit/Cargo.toml` | 改 | 加依赖 `golish-pentest-domain = { workspace = true }`（已经 workspace 成员，仅声明直接依赖） |
| `backend/crates/golish-agent-kit/src/harness/technique_resolver.rs` | 改 | 删除本地 `AssetClass` 定义（enum + 那 5 个方法 + `from_value` 单测），改为 `pub use golish_pentest_domain::AssetClass;`；保留 `StageKind`-耦合的 `stage_baseline`/`resolve_expected_techniques`/`technique_applies` 及其单测不动 |

> 不改 DB schema、不改 ts-rs/IPC（`AssetClass`/`AssetKey` 为后端内部类型；`AssetClass` 本就未跨 IPC）。`golish-db`/`golish-recon-app` 的 pentest-domain 依赖在 **PR-B** 真正使用时再加（本 PR 不需要）。

---

## Task 1 · `golish-pentest-domain` 新增 `asset_id` 模块（TDD）

**文件：** 新建 `backend/crates/golish-pentest-domain/src/asset_id.rs`；改 `backend/crates/golish-pentest-domain/src/lib.rs`。

**步骤 1.1（先写实现骨架 + 迁入 AssetClass）** 新建 `asset_id.rs`，把 agent-kit `technique_resolver.rs:9-105` 的 `AssetClass` 原样迁入（逻辑逐字不变），再加 `AssetKey` + `canonical_asset_key`：

```rust
//! 资产的规范身份（设计 2026-06-18-canonical-asset-identity §3.1）。
//! coverage join / evidence 落库 / 真值读取四处统一用 `canonical_asset_key` 这把钥匙。
//! 纯函数、无 I/O。`AssetClass` 自 golish-agent-kit 迁入（D1）。

use std::net::IpAddr;

/// in-scope 资产的粗分类（来自 `targets.type` 或值推断），决定哪些技术适用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Domain,
    Ip,
    Url,
    Cidr,
    /// 未知/其它：保守当作"可能含 web"，不缩小技术集。
    Other,
}

impl AssetClass {
    /// 映射 `targets.type` 字符串（与本 crate TargetType 对齐）。
    pub fn from_target_type(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "domain" | "subdomain" | "host" => Self::Domain,
            "ip" | "ip_address" | "ipv4" | "ipv6" => Self::Ip,
            "url" | "endpoint" | "web" => Self::Url,
            "cidr" | "range" | "netblock" => Self::Cidr,
            _ => Self::Other,
        }
    }

    /// 从资产 **值** 推断类别（gate 轴无权威 `targets.type` 时用）。
    /// 保守：无法识别的非空值落 `Domain`（intel 严格全技术集），空 → `Other`。
    pub fn from_value(value: &str) -> Self {
        let v = value.trim();
        if v.is_empty() {
            return Self::Other;
        }
        if Self::is_url_wrapped_ip(v) {
            return Self::Ip;
        }
        let lower = v.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Self::Url;
        }
        if v.parse::<IpAddr>().is_ok() {
            return Self::Ip;
        }
        if let Some((addr, prefix)) = v.split_once('/') {
            if addr.parse::<IpAddr>().is_ok() && prefix.parse::<u8>().is_ok() {
                return Self::Cidr;
            }
        }
        Self::Domain
    }

    /// `value` 是否为 host 是裸 IP 的 http(s) URL（如 `http://124.196.77.48`）。
    pub fn is_url_wrapped_ip(value: &str) -> bool {
        let lower = value.trim().to_ascii_lowercase();
        let Some(rest) = lower
            .strip_prefix("http://")
            .or_else(|| lower.strip_prefix("https://"))
        else {
            return false;
        };
        let authority = rest.split('/').next().unwrap_or("");
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        let host = if let Some(stripped) = authority.strip_prefix('[') {
            stripped.split(']').next().unwrap_or(stripped)
        } else {
            authority.split(':').next().unwrap_or(authority)
        };
        host.parse::<IpAddr>().is_ok()
    }

    /// 权威 `targets.type`（已知时）+ 值共同定类；URL-wrapped IP 永远 `Ip`。
    pub fn classify(target_type: Option<&str>, value: &str) -> Self {
        if Self::is_url_wrapped_ip(value) {
            return Self::Ip;
        }
        match target_type {
            Some(ty) => Self::from_target_type(ty),
            None => Self::from_value(value),
        }
    }

    /// 该资产是否可能承载 web 服务（PARAM/JSAPI/DIR 等是否要求）。
    pub fn maybe_web(self) -> bool {
        matches!(self, Self::Domain | Self::Url | Self::Other)
    }
}

/// 资产的规范身份：`key` 是统一 join 钥匙，`class` 是主机类别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetKey {
    pub key: String,
    pub class: AssetClass,
}

/// 把任意资产值归一为「主机身份」。规则（设计 §3.1）：
/// 1. trim；空 → None。 2. ASCII 小写。 3. URL 取 host（去 scheme/userinfo/port、
///    解 `[]` IPv6）。 4. 去 FQDN 尾点。 5. IP/CIDR → 规范文本。
/// **不做**：apex 截断、剥 `www.`（会把不同资产并成一格）。
/// URL 折叠到其 host 身份（intel 技术按 host 判定）；class 取主机类别。
pub fn canonical_asset_key(value: &str) -> Option<AssetKey> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    // URL → host；否则整值当 host 候选。
    let host_candidate = extract_host(&lower).unwrap_or_else(|| lower.clone());
    let host = host_candidate.trim_end_matches('.').to_string();
    if host.is_empty() {
        return None;
    }

    // IP 字面量 → 规范文本（压缩 IPv6 零段、消前导零）。
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(AssetKey { key: ip.to_string(), class: AssetClass::Ip });
    }
    // CIDR → 规范网络文本。
    if let Some((addr, prefix)) = host.split_once('/') {
        if let (Ok(ip), Ok(p)) = (addr.parse::<IpAddr>(), prefix.parse::<u8>()) {
            return Some(AssetKey { key: format!("{ip}/{p}"), class: AssetClass::Cidr });
        }
    }
    // 域名（含从 URL 折叠来的 host）。
    Some(AssetKey { key: host, class: AssetClass::Domain })
}

/// http(s) URL → host（去 scheme/userinfo/port，解 `[]` IPv6）；非 URL → None。
fn extract_host(lower: &str) -> Option<String> {
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))?;
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or(stripped)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    (!host.is_empty()).then(|| host.to_string())
}
```

**步骤 1.2** 在 `src/lib.rs` 顶部模块声明区加：

```rust
pub mod asset_id;
pub use asset_id::{canonical_asset_key, AssetClass, AssetKey};
```

**步骤 1.3（测试先行：把验收用例钉死）** 在 `asset_id.rs` 末尾加测试模块。这些用例直接锁住设计 §4 红线（www 保留、apex 不截断）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn key_of(v: &str) -> Option<(String, AssetClass)> {
        canonical_asset_key(v).map(|k| (k.key, k.class))
    }

    #[test]
    fn lowercases_and_trims() {
        assert_eq!(key_of("  Pingan.COM "), Some(("pingan.com".into(), AssetClass::Domain)));
    }

    #[test]
    fn strips_trailing_fqdn_dot() {
        assert_eq!(key_of("pingan.com."), Some(("pingan.com".into(), AssetClass::Domain)));
    }

    #[test]
    fn www_is_preserved_distinct_asset() {
        // 红线：www.x 与 x 是不同资产，绝不合并。
        assert_eq!(key_of("www.pingan.com"), Some(("www.pingan.com".into(), AssetClass::Domain)));
    }

    #[test]
    fn apex_is_never_truncated() {
        // 红线：绝不截断到注册 apex（registrable_domain 的事故根因）。
        assert_eq!(key_of("a.b.pingan.com"), Some(("a.b.pingan.com".into(), AssetClass::Domain)));
    }

    #[test]
    fn url_collapses_to_host() {
        assert_eq!(key_of("https://api.pingan.com/login?x=1"),
                   Some(("api.pingan.com".into(), AssetClass::Domain)));
    }

    #[test]
    fn url_wrapped_ip_is_ip() {
        assert_eq!(key_of("http://124.196.77.48"), Some(("124.196.77.48".into(), AssetClass::Ip)));
        assert_eq!(key_of("https://1.2.3.4:8443/x"), Some(("1.2.3.4".into(), AssetClass::Ip)));
    }

    #[test]
    fn bare_ip_canonicalizes() {
        assert_eq!(key_of("1.2.3.4"), Some(("1.2.3.4".into(), AssetClass::Ip)));
        // IPv6 规范化（压缩零段 + 小写）。
        assert_eq!(key_of("2001:0DB8::0001"), Some(("2001:db8::1".into(), AssetClass::Ip)));
    }

    #[test]
    fn cidr_canonicalizes() {
        assert_eq!(key_of("10.0.0.0/8"), Some(("10.0.0.0/8".into(), AssetClass::Cidr)));
    }

    #[test]
    fn empty_or_blank_is_none() {
        assert_eq!(canonical_asset_key(""), None);
        assert_eq!(canonical_asset_key("   "), None);
    }

    #[test]
    fn drift_pair_canonicalizes_equal() {
        // 这是整个改动的核心断言：漂移的两种写法归一后必须相等（才 join 得上）。
        let a = canonical_asset_key("Pingan.com.").unwrap().key;
        let b = canonical_asset_key("pingan.com").unwrap().key;
        assert_eq!(a, b, "身份漂移的两种写法归一后必须相等");
    }

    // —— 迁入的 AssetClass 行为回归（自 agent-kit 搬来，逐字不变）——
    #[test]
    fn asset_class_from_value_classifies() {
        assert_eq!(AssetClass::from_value("1.2.3.4"), AssetClass::Ip);
        assert_eq!(AssetClass::from_value("http://1.2.3.4"), AssetClass::Ip);
        assert_eq!(AssetClass::from_value("https://x.com/a"), AssetClass::Url);
        assert_eq!(AssetClass::from_value("10.0.0.0/24"), AssetClass::Cidr);
        assert_eq!(AssetClass::from_value("x.com"), AssetClass::Domain);
        assert_eq!(AssetClass::from_value(""), AssetClass::Other);
    }
}
```

**验证：**
```bash
cd backend && cargo nextest run -p golish-pentest-domain
```
预期：新增 11 个测试全过（`asset_id::tests::*`）；crate 编译 0 警告。

**提交：** `git add backend/crates/golish-pentest-domain && git commit -m "feat(pentest-domain): canonical_asset_key + migrate AssetClass (E1 PR-A)"`

---

## Task 2 · `golish-agent-kit` 改 `AssetClass` 为重导出（零行为变化）

**文件：** `backend/crates/golish-agent-kit/Cargo.toml`、`backend/crates/golish-agent-kit/src/harness/technique_resolver.rs`。

**步骤 2.1** `Cargo.toml` 的 `[dependencies]` 加（紧挨现有 `golish-pentest` 行）：

```toml
# AssetClass + canonical_asset_key 的单一来源（design 2026-06-18 D1）。
golish-pentest-domain = { workspace = true }
```

**步骤 2.2** `technique_resolver.rs`：删掉本地 `AssetClass`（当前 `:8` 的 `#[derive...] pub enum AssetClass {...}` 到 `:105` `impl AssetClass` 结束的整段，连同 `mod tests` 里仅测 `from_value`/`classify` 的 `from_value_classifies_ip_url_cidr_domain`（`:303`）一并删除——它已迁到 pentest-domain），在文件顶部 `use crate::harness::types::StageKind;` 下方加：

```rust
// AssetClass 的单一来源已迁到 golish-pentest-domain（design 2026-06-18 D1）；
// 此处重导出，保持 `technique_resolver::AssetClass` 既有引用零改动。
pub use golish_pentest_domain::AssetClass;
```

保留 `stage_baseline` / `resolve_expected_techniques` / `technique_applies` 及其它单测**原样不动**（它们用 `AssetClass` 的方式不变，方法签名一致）。

**步骤 2.3（确认零行为变化）** 全 crate 跑测试——`technique_applies` / `resolve_expected_techniques` 等既有测试必须全绿（证明重导出后行为逐字节一致）：

**验证：**
```bash
cd backend && cargo nextest run -p golish-agent-kit
# 重点确认 technique_resolver 既有测试仍全过：
cargo nextest run -p golish-agent-kit technique_resolver 2>&1 | tail -20
```
预期：agent-kit 全部测试通过；无 `unresolved import` / `AssetClass` 相关编译错误。

**提交：** `git add backend/crates/golish-agent-kit && git commit -m "refactor(agent-kit): re-export AssetClass from pentest-domain (E1 PR-A)"`

---

## Task 3 · 工作区编译 + 收口

**步骤 3.1** 全工作区编译（确认没有别的 crate 直接引用了被移动的内部项）：

**验证：**
```bash
cd backend && cargo check --workspace 2>&1 | tail -20
just lint-rust   # clippy 零 warning
```
预期：`cargo check --workspace` 0 错误；clippy 0 warning。若有别的 crate 直接 `use golish_agent_kit::harness::technique_resolver::AssetClass`，重导出已使其继续可用——无需改动；若报 `AssetClass` 不可达，检查重导出 `pub use` 是否落在模块顶层。

**提交：** 本 task 无代码改动则不单独 commit（仅验证）。

---

## 自检（对照设计 §3.1/§4 + writing-plans 红旗）

**1. 规格覆盖度**
- §3.1 规则 1（trim/空→None）→ `empty_or_blank_is_none` ✅
- 规则 2（小写）→ `lowercases_and_trims` ✅
- 规则 3（URL→host、IPv6 解 `[]`、去 port/userinfo）→ `url_collapses_to_host` / `url_wrapped_ip_is_ip`（`:8443` 端口）✅
- 规则 4（去尾点）→ `strips_trailing_fqdn_dot` ✅
- 规则 5（IP/CIDR 规范）→ `bare_ip_canonicalizes`（含 IPv6）/ `cidr_canonicalizes` ✅
- 规则 7 红线（不去 www / 不截 apex）→ `www_is_preserved_distinct_asset` / `apex_is_never_truncated` ✅
- §4 红线1（gate 纯函数不变）→ 本 PR 不接调用方 + Task 2 重导出零行为变化（既有 agent-kit 测试守）✅
- D1（AssetClass 同住 + 零循环）→ Task 1 迁入、Task 2 重导出、Task 3 workspace check ✅

**2. 占位符扫描**：无 TODO/待定；每步均含完整代码与精确命令。✅

**3. 类型一致性**：`AssetKey{key, class}`、`AssetClass`、`canonical_asset_key`、`extract_host` 命名在 Task 1/2/3 与测试中一致；`AssetClass` 方法签名迁移前后逐字一致（直接复制 agent-kit 现有实现）。✅

---

## 后续计划（独立文件，按 writing-plans「每子系统一计划」拆，落 PR-A 后再写）

> 本 PR-A 是独立可发布、可测试的单元（纯函数 + 重导出，零行为变化）。下列为路线图，**不在本计划内写实现代码**，待 PR-A 合并后各自成文：

- **PR-B · E1 边界接入**（`docs/superpowers/plans/2026-06-18-pr-b-canonical-key-wiring.md`）：把 `canonical_asset_key` 接到 ① `in_scope_assets` 读出归一、② `evidence_facts` 落 `evidence_asset` 前归一、③ 删 `normalized_host` 两份重复副本。验证：`rule_engine` 既有 PASS/BLOCK parity 全绿 + 新增「身份漂移 join 命中」回归 + 活体 pingan target_intel 漂移缺口消失。**实现前需读** `in_scope_assets` 真实 impl（golish-db repo）与 evidence 落库点。
- **PR-C · E3 建表 + 写路径**（设计 §5 PR-C）：migration `technique_outcomes`（设计 §3.3）+ golish-db repo（upsert/按 run+org 读，IDOR 过滤）+ 落库点同步 upsert（asset 走规范键、seq 每 run 从 1 = D2）。**走 §2.7 高风险确认**（建表）。
- **PR-D · E3 gate 读路径切换**（设计 §5 PR-D）：从 `technique_outcomes` 投影 `EvidenceFact` 注入 `GateContext`，灰度 dual-read + 新旧真值比对断言（D4），opt-in 先 target_intel。
- **PR-E ·（可选）清旧 union**（设计 §5 PR-E）：读路径全切表、活体稳定后下线 `coverage_truth` 业务表 union 读法。

> 排期（D5）：PR-A → PR-B（E1 完成）→ PR-C → PR-D →（PR-E）。E3 的表 `asset` 依赖 E1 规范键，故 E1 必须先落。
