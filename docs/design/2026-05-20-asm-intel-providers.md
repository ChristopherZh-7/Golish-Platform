# ASM 多 Provider 集成 · 架构设计文档

> Superseded by `docs/design/2026-05-21-integrations.md`.
> 保留本文仅作历史决策记录；当前凭据/外部情报源入口以 Settings → Integrations 和 `docs/design/2026-05-21-integrations.md` 为准。

> 日期：2026-05-20
> 状态：Superseded（原 Draft → Approved；用户确认 4 个决策点全选推荐项）
> 关联 baseline：`docs/design/2026-05-20-pentest-fields-tool-mapping.md`
> 旧实施计划：`docs/superpowers/plans/2026-05-20-asm-intel-providers.md`（已删除；由 Integrations 方案取代）
> 分支：`feat/asm-intel-providers`

---

## 0. Why · 上下文与动机

Golish 现状（来自 baseline §0 / §4-§6）：

- `organizations` 表 28 字段中 **11 个**有工具能补，但其中 **3 个部分就绪**（amass intel 能采但无写入路径）、**6 个完全缺工具**（email_domains / certificates / github_orgs / social_accounts / contacts / industry+credit_code）。
- `targets` 表 9 个 recon 字段虽然写入器就绪，但 db_action 配错（16/21 工具走 Unknown 分支）。
- 国内 HVV 红队真实需要：国内网络空间测绘（0.zone / FOFA / 360 Quake / Hunter / ZoomEye）+ 工商情报（天眼查 / 启信宝）+ 备案查询（ICP / 站长之家）的统一接入。

如果每来一个新 ASM provider 都要"写 client + 改 schema + 改 router + 改前端 UI + 改 vault"，工时无法控制。本文档定义**一次性可扩展架构**，以 0.zone 为首个落地实例。

---

## 1. 4 个关键决策（用户已确认）

| 决策 | 选项 A（推荐 · 已选）| 选项 B | 选项 C |
|---|---|---|---|
| 1 · 代码分布 | **新 crate `golish-intel-providers`** | 塞进 `golish-pentest` | - |
| 2 · API Key 存储 | **复用 `vault_entries` 表**（entry_type=ApiKey + tags=["data-source", "{name}"]；兼容旧 tag `intel-provider`）| 新表 `intel_provider_keys` | `settings.json` 明文 |
| 3 · Settings 入口 | **新增 NAV_ITEMS section "data_sources"**（Data Sources / 数据源）| 保留旧名 Intel Providers | 合并到现有 Providers（AI + ASM 混）|
| 4 · db_action 设计 | **单一 `organization_update`**（按 fields key 路由列）| 细粒度 `organization_update_domains/network/...` | - |

> 命名修订（2026-05-21）：面向用户的 Settings 文案统一改为 **Data Sources / 数据源**。
> `IntelProvider` / `golish-intel-providers` 可作为内部实现名短期保留，避免在本分支做大规模重命名。

### 1A · 为什么新 crate

- **独立可测**：mock HTTP 后能完整跑单测，不依赖 golish-pentest 大块依赖
- **依赖清晰**：只依赖 `reqwest` / `serde` / `tokio` / `tracing` / 内部 `golish-db`（读 vault）+ `golish-pentest`（output_store 接口）
- **未来可单独发布**：作为独立 crate.io 包供其他渗透平台复用
- **认知边界清晰**：1 crate = 1 概念（ASM 平台抽象层）

### 1B · 为什么复用 vault

- `VaultEntryType::ApiKey` 已存在（`golish-db/src/models/enums.rs:122-128`）
- `VaultSettings.tsx` 已是完整 CRUD UI（增删改查 + 复制 + 显示/隐藏）
- 已有加密设计（如果 vault crate 内启用）
- 零 schema 迁移 = 零回滚风险

### 1C · 为什么独立 Data Sources section

- AI provider 关心 model / temperature / context_window
- 外部数据源关心 credential / quota / rate_limit / query_type / 本地工具安装状态
- 两者数据模型完全不同，混在一起会让 UI 极度复杂
- 独立 section 也方便未来扩展 quota 监控、凭据健康、调用历史等

### 1D · 为什么单一 db_action

- 加新 provider 只需扩 patterns 抽 fields，不动 Rust 路由代码
- writer 按 fields key 智能识别要更新哪些列（fields["domain"] / ["cidr"] / ["asn"] / ["cert"] / ["email"] / ["contact_name"] / ["github_org"] / ["wechat_id"] / ...）
- 与 `target_update_recon` 设计哲学一致（一个 writer 接管一类资源的所有列）

---

## 2. 架构（4 层）

```
┌─────────────────────────────────────────────────────────────┐
│ Frontend · Settings → Data Sources section                   │
│   - DataSourceCard（每家一张：name/credential/quota/test）     │
│   - CredentialEditor（接 vault.entry CRUD）                    │
│   - CredentialHealth（valid/expired/rate_limited/needs input） │
└─────────────────────┬───────────────────────────────────────┘
                      │ IPC (Tauri command)
                      ↓
┌─────────────────────────────────────────────────────────────┐
│ golish/src/tools/intel_providers/  (IPC facade)              │
│   - intel_query_provider(provider_id, query_type, query)    │
│   - intel_list_providers() -> Vec<ProviderMeta>              │
│   - intel_test_connection(provider_id) -> ConnectionStatus   │
└─────────────────────┬───────────────────────────────────────┘
                      │ trait call
                      ↓
┌─────────────────────────────────────────────────────────────┐
│ golish-intel-providers (核心抽象 + 各 provider 实现)         │
│   IntelProvider trait                                        │
│   ├── id() -> &str                                           │
│   ├── meta() -> ProviderMeta                                 │
│   ├── query(qtype, q, key) -> Vec<ProviderRecord>            │
│   └── test_connection(key) -> ConnectionStatus               │
│                                                              │
│   zone/      0.zone (零零信安) ← 首个                        │
│   fofa/      鹰图 FOFA          ← P1 预留                    │
│   quake/     360 Quake          ← P1 预留                    │
│   hunter/    奇安信 Hunter      ← P2 预留                    │
│   shodan/    国外 Shodan        ← P3 预留                    │
│                                                              │
│   shared/                                                    │
│   ├── api_key (从 vault 读 key + 缓存)                      │
│   ├── rate_limit (per-provider 限速)                         │
│   └── http_client (reqwest 共享配置)                         │
└─────────────────────┬───────────────────────────────────────┘
                      │ ProviderRecord (统一格式)
                      ↓
┌─────────────────────────────────────────────────────────────┐
│ golish-pentest/output_store/                                 │
│   organizations.rs (NEW)                                     │
│     store_organization_update(fields, project_path)         │
│     按 fields key 路由：                                     │
│       fields["domain"]   → organizations.domains 追加         │
│       fields["cidr"]     → organizations.ip_ranges 追加      │
│       fields["asn"]      → organizations.asns 追加           │
│       fields["cert"]     → organizations.certificates 追加   │
│       fields["email"]    → organizations.email_domains 追加  │
│       fields["github"]   → organizations.github_orgs 追加    │
│       fields["contact"]  → organizations.contacts 追加       │
│                                                              │
│   mod.rs match db_action 加：                                │
│     "organization_update" => store_organization_update(...)  │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. 数据流（端到端）

### 3.1 配置 API Key 的流程

```
User → Settings → Data Sources → DataSourceCard("0.zone") → CredentialEditor
  → IPC: vault.create({name:"zone_key_id", value:"xxx", entry_type:"api_key",
                       tags:["data-source","0.zone"]})
  → vault_entries 表写入
  → CredentialHealth 显示已配置 ✅
```

### 3.2 调用 ASM API 的流程

```
User → 在 organizations 详情页 → "用 0.zone 拉取域名" 按钮
  → IPC: intel_query_provider("0.zone", "domain", "example.com")
  → golish/intel_providers/commands.rs
  → ZoneProvider::query()
    → shared::api_key::read("zone_key_id")
    → shared::rate_limit::acquire("0.zone", 2req/s)
    → HTTP POST https://0.zone/api/data/ ...
    → Vec<ProviderRecord>（统一格式）
  → 调用 output_store::maybe_detect_and_store_via()
    （或绕过 detect，直接 store_organization_update）
  → organizations.domains 字段追加新数据
  → UI 刷新展示新值
```

### 3.3 ProviderRecord 统一格式

```rust
pub struct ProviderRecord {
    pub provider: String,           // "0.zone" / "fofa" / ...
    pub query_type: String,         // "site" / "domain" / "email" / ...
    pub fields: HashMap<String, String>,  // 与 OutputConfig.fields 同结构
    pub raw: serde_json::Value,    // 原始响应（兜底，用于 evidence）
    pub fetched_at: DateTime<Utc>,
}
```

### 3.4 公司名驱动的递归 ASM 流程

用户输入公司名（例如“平安”）时，系统不能直接把所有搜索结果写进 `targets`。正确流程是先建立组织画像与授权范围，再把确认属于范围内的资产转成 `targets`。

```text
公司名输入
  → 工商/企业画像解析（企查查 / 天眼查 / 启信宝 / 0.zone org）
  → 根 organization
  → 子公司 / 分支机构 / 曾用名 / 关联公司候选
  → scope 判定（授权 + 股权 + 证据置信度）
  → in-scope organizations
  → ASM provider 查询（0.zone / FOFA / Quake / ICP / amass / subfinder）
  → organizations 字段补全
  → 确认资产写入 targets（带 organization_id）
  → 技术 recon 继续扩展，新增证据反哺 organizations
```

#### 3.4.1 工商数据源职责

企查查 / 天眼查 / 启信宝这类 provider 主要负责“组织关系”，不直接负责技术资产：

| 数据 | 写入位置 | 用途 |
|---|---|---|
| 统一社会信用代码 | `organizations.credit_code` | 消歧，避免同名公司误合并 |
| 法定名称 / 曾用名 / 英文名 | `organizations.name` / `aliases` | 搜索扩展与归并 |
| 行业 / 法人 / 注册地址 / 注册资本 | `organizations.industry` / `intel.records[]` | 画像与风险排序 |
| 子公司 / 分公司 / 控股公司 | `organizations.parent_id` / `subsidiaries` | 建组织树 |
| 股权比例 / 控制关系 | `organizations.intel.records[]` + scope evidence | 判定是否纳入探查 |
| 官网 / 邮箱 / 联系方式 | `domains` / `email_domains` / `contacts` | 后续 ASM 种子 |

#### 3.4.2 scope 判定规则

默认规则应保守，避免把不在授权内的公司误加入攻击面：

| 候选关系 | 默认动作 | 原因 |
|---|---|---|
| 全资子公司 / 分公司 | 标记 `in_scope_candidate` | 组织归属明确 |
| 控股比例 `>= 50%` | 标记 `in_scope_candidate` | 控制权强，符合用户提到的 50% 规则 |
| 控股比例 `< 50%` | 标记 `needs_confirmation` | 参股不等于授权可测 |
| 曾投资 / 历史关联 / 已注销 | 标记 `out_of_scope_candidate` | 不自动进入技术探查 |
| 0.zone `group` / ICP / 证书主体与根组织强一致 | 提升置信度 | 可作为补充证据 |

scope 最终状态不应只靠 provider 自信判断，必须保留证据：来源、时间、字段、原始响应摘要、置信度、判定理由。

#### 3.4.3 organizations 与 targets 的边界

`organizations` 是组织树和情报仓库，`targets` 是实际测试对象。二者关系如下：

- 子公司、分公司、参股公司先写 `organizations`，通过 `parent_id` 形成树。
- 域名、邮箱域、ASN、证书、GitHub org 等先补到 `organizations.*`。
- 只有当资产满足 `in_scope` 且有足够证据时，才生成 `targets`。
- 生成的 `targets` 必须带 `organization_id`，并记录 `source`（如 `0.zone:site` / `icp` / `amass`）。
- 技术扫描发现的新域名/IP 如果反查到新公司，应先回写 organizations，再走 scope 判定。

#### 3.4.4 递归停止条件

递归探查必须有边界：

- 最大组织树深度默认 2：根公司 → 子公司 → 孙公司；更深需用户确认。
- 默认只自动探查 `in_scope_candidate`，`needs_confirmation` 不进入 active recon。
- 每个 provider 有 quota/rate_limit；0.zone 等付费 API 按 provider 配额调度。
- 同一组织/资产的重复证据做幂等合并，不重复写入。
- active scan 前必须再次确认授权范围，不能仅凭工商关系触发高风险扫描。

### 3.5 ENScan_GO 低成本企业画像数据源

ENScan_GO 适合作为低成本的 **Enterprise Intelligence / 企业情报** 数据源：它通过用户自己的登录态 Cookie / token 调用爱企查、天眼查、快查、风鸟、MIIT ICP 等来源，能补齐公司、子公司、控股关系、ICP备案、APP、小程序、公众号、招聘、软件著作权等数据。

#### 3.5.1 接入位置

推荐 **Tool Manager 优先，Golish MCP 包装其次**：

```text
Tool Manager 管 ENScan_GO 安装 / 启动 / 配置 / 运行
  → EnscanDataSource 调 REST API 或 CLI JSON 输出拿结构化结果
  → mapper 写 organizations / targets / evidence
  → Golish 再把规范化后的 enscan_query 暴露成 MCP tool 给 AI 使用
```

不建议把 ENScan_GO 自带 MCP server 作为主数据管道：它返回的是 tool result text 中的 JSON，适合 AI 临时问答；而 Golish 的落库、幂等、scope gate、evidence ledger 更适合走 Tool Manager + REST/CLI 受控路径。

#### 3.5.2 Tool Manager 运行模式

| 模式 | 命令 | 用途 | 推荐级别 |
|---|---|---|---|
| API server | `enscan -api` / `enscan --api` | Golish 后端调用 `/api/info`，返回结构化 JSON | P0 推荐 |
| CLI JSON | `enscan -n <公司> -json -out-dir <dir>` | 离线任务、人工下载结果 | P1 |
| MCP server | `enscan -mcp` / `enscan --mcp` | AI 客户端临时查询、prompt 工作流 | P2 |

第一版推荐调用：

```text
GET /api/info?name=<company>&type=aqc&field=icp,app,wx_app,wechat&invest=51&branch=true&depth=2
```

#### 3.5.3 输出映射

| ENScan 输出 | Golish 写入 |
|---|---|
| `enterprise_info` | 根 `organizations`，补 `name`、`credit_code`、`contacts/intel` |
| `invest` | 子 `organizations`，记录 `parent_id`、持股比例 evidence、scope 候选状态 |
| `holds` | 控股企业，优先标记 `in_scope_candidate` |
| `branch` | 分支机构，优先标记 `in_scope_candidate`，但 active recon 前仍需授权确认 |
| `partner` | 股东信息，进入 `organizations.intel.records[]`，不直接生成 targets |
| `icp` | `organizations.domains`，作为域名归属强证据 |
| `app` / `wx_app` | `organizations.business_systems` |
| `wechat` / `weibo` | `organizations.social_accounts` |
| `supplier` | `organizations.intel.records[]`，默认不进入攻击面 |
| `job` / `copyright` | `organizations.intel.records[]`，用于画像和线索 |

#### 3.5.4 凭据健康与刷新流程

ENScan_GO 的登录态 Cookie/token 可能失效或触发风控，因此凭据刷新必须 human-in-the-loop：

```text
运行前 health check
  → valid：继续任务
  → expired / captcha_required / rate_limited：暂停任务
  → UI/AI 提示用户到 Settings → Data Sources → ENScan_GO 更新凭据
  → 用户在 CredentialEditor 粘贴新 Cookie/token
  → vault 保存 + mask 日志
  → health check 通过后恢复任务
```

约束：

- AI 只负责发现凭据失效、解释原因、引导用户刷新、恢复任务。
- AI 不获取浏览器 Cookie，不把 Cookie/token 放进聊天上下文。
- Cookie/token 只进 `vault_entries`，日志和错误消息必须 mask。
- ENScan_GO 默认 delay 不低于 3 秒，或使用随机 1-5 秒，降低账号异常风险。

---

## 4. 不变量（不能破坏的约束）

| # | 不变量 | 验证方式 |
|---|---|---|
| I1 | API key 永不出现在 settings.json / 日志 / error msg | grep `settings.json` 不含 "_key" / 日志全 mask 中间字符 |
| I2 | rate_limit 必须 per-provider，不可全局共用 | shared::rate_limit 按 provider id 分桶 |
| I3 | `IntelProvider::query()` 必须返回 `IntelResult<Vec<ProviderRecord>>`，错误显式 | 类型签名 + thiserror |
| I4 | 同一 provider 多次调用必须串行（避免触发限速 ban）| shared::rate_limit 内部 mutex |
| I5 | organizations 表更新必须 idempotent（重复 query 不复制数据）| store_organization_update 用 jsonb 去重 |
| I6 | 不修改 vault_entries schema | 0 migration |

---

## 5. 验收标准

- ✅ `cargo nextest run -p golish-intel-providers` 全绿
- ✅ `cargo check --workspace` 无 warning
- ✅ `pnpm vitest run frontend/components/Settings/IntelProvidersSettings/` 全绿
- ✅ 手动 E2E：Settings 配 0.zone key → 在 organizations UI 拉一次 → DB 看 domains/ip_ranges/asns 有新增
- ✅ `just precommit` 全绿
- ✅ 新增任何 provider 只需 1 个 Rust file + 1 个 toolsconfig JSON + 1 个 ProviderCard 行（≤2h）

---

## 6. 后续 provider 接入路径（标准化）

每个新 provider 5 步：

1. 在 `golish-intel-providers/src/<name>/` 建目录
2. 写 `client.rs`（HTTP 调用）+ `types.rs`（响应模型）+ `mapper.rs`（→ ProviderRecord）
3. 在 `lib.rs` 注册：`PROVIDERS.insert("name", Box::new(NameProvider::default()))`
4. 写 `resources/toolsconfig/<name>.json`（含 7-10 个 skill 预设）
5. 在前端 `ProviderCard` 配置数组加一条（id / display_name / quota_url / signup_url）

---

## 7. 风险点

| 风险 | 等级 | 缓解 |
|---|---|---|
| 0.zone API 频率/字段变化 | 中 | shared::http_client 做版本化 user-agent + 失败上报 + 单测用 mock HTTP |
| API key 泄漏 | 高 | 日志 mask + 不出现在 git diff |
| organizations 表 jsonb 列冲突 | 低 | store_organization_update 内部 transaction + jsonb_agg 去重 |
| Settings UI 配错 key 导致重复调用 | 低 | test_connection 在保存前可一键验 |
| 多 provider 同时调用拖慢 IO | 低 | 各 provider 独立 rate_limit |

---

## 8. 不变更项（避免范围漂移）

本设计 **不** 触及：

- targets 表 9 个 recon 字段的 db_action 修复（属于 baseline §9.1，单独 PR）
- credentials 表 schema + credential_add writer（属于 baseline §9.6，单独 PR）
- api_endpoint_add db_action（属于 baseline §9.4，单独 PR）
- agent loop 集成 ASM provider（属于 Phase 5+，单独 PR）

本设计 **只** 实现：

- ASM provider 抽象层 + 0.zone 首发
- vault 复用做 key 管理
- organizations 表的 `organization_update` writer + db_action 分支
- Settings UI 的 Intel Providers section

---

## 9. 引用

- baseline 字段映射：`docs/design/2026-05-20-pentest-fields-tool-mapping.md`
- vault entry type 枚举：`backend/crates/golish-db/src/models/enums.rs:122-128`
- vault repo：`backend/crates/golish-db/src/repo/vault.rs`
- VaultSettings UI 参考：`frontend/components/Settings/VaultSettings.tsx`
- ProviderSettings UI 参考：`frontend/components/Settings/ProviderSettings/`
- output_store dispatch：`backend/crates/golish-pentest/src/output_store/mod.rs:178-198`
- organizations schema：`backend/crates/golish-db/src/models/pentest.rs:41-93`

---

**作者**：架构组
**审阅**：用户已批准 4 个决策点
