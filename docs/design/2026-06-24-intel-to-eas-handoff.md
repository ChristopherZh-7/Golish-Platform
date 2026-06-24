# intel → EAS handoff 强化（把情报物化成攻击面种子 + 加厚交接）

> 评审 + 用户 2026-06-24 拍板的决策。状态：**P0–P3 已实现并验证（2026-06-24）；P4 + D2 自动触发 deferred**（BajieAsk-agent-4 全栈工程师，DISPATCH off）。
> 配套实现计划：`docs/superpowers/plans/2026-06-24-intel-to-eas-handoff.md`。
>
> **实现状态（2026-06-24）**：
> - ✅ **P0** L0a CIDR/ASN 段→`cidr` 种子物化 + L1a `list_in_scope_targets` 投影加宽
> - ✅ **P1** L0b CT SAN/CN→`domain` 种子物化
> - ✅ **P2** b1 按域名查 provider **能力**（`{{domain}}` 模板 + fofa domain 查询 + `recon_map_assets` `domain` 入参 + native domain 分支 + provider 门）
> - ✅ **P3** `list_attack_surface_seeds` 工具 + 优先级排序 + 可配 cap
> - ⏸ **D2 自动递归触发 deferred**：b1 能力已就绪（agent 可带 `domain` 调 `recon_map_assets`），但「EAS 主动发现新顶级域 → 自动触发 b1」的 wiring 需定位 EAS 主动落点，留作后续。
> - ⏸ **P4（EAS gate authoritative）deferred**：`stage_spec.rs::active_stages_derive_from_evidence_but_not_authoritative` 守卫测试**显式禁止** EAS/enumeration 开 `authoritative_found`（"active Empty fact source not ready"）。翻开既破坏守卫又语义不成立 → 维持 default off（现状），待主动 Empty 事实源就绪后另行启用。
> - 验证：`nextest`（golish-recon-app/app-core/agent-app/agent-kit/agent-runtime/tools/db）全绿（修了 2 个计数断言：fofa queries 2→3、tool decls 41→42）+ `clippy --all-targets -D warnings` exit 0 + `check-fe` exit 0（ts-rs `AssetIntelHydrateConfig.domain` 已重生）。**未活体验证**（需重启应用）。

## 1. 问题（评审结论）

昨天（6/23）那批 intel 优化（per-dim freshness `649fef1e`、provider direct-landing `edee7506`、source_query_log #5、technique_outcomes #4、expansion_queue #6）闭的是 **「采集 + 可证明」** 这一环（coverage / evidence / freshness / 留痕）。**不是** 「下一阶段把情报用起来」这一环。

逐维度核实下一阶段（external_attack_surface / enumeration）对 intel 的消费：

| 维度 | 落到哪 | 下一阶段怎么用 | 结论 |
|---|---|---|---|
| SUBDOMAIN | `target_assets` + `targets.scope='in'` | EAS 直接当扫描目标 | ✅ 通畅 |
| DNS | `dns_records` | EAS 复用 A 记录、不重解析 | ✅ 通畅 |
| ASN | `organizations.asns` | 只喂 coverage gate `has_asn` | ❌ 没扩面 |
| CT | `organizations.certificates` | coverage `has_ct` + 证书计数 | ⚠️ 没二次解析成目标 |
| WHOIS | `organizations.whois` | 只喂 coverage gate | ❌ 无下游（社工阶段未就绪，本设计不处理） |
| OSINT | `organizations.intel` | coverage `has_osint` + 前端展示 | ❌ 无下游（同上，不处理） |
| 扩展线索 | `expansion_queue` | 只 enqueue，Rust 无读取（run_tree.py reviewer） | （#6 范围，本设计不处理） |

**两类根因**（修的地方不同）：

- **A · 下游没读**：EAS 只读 `targets.scope='in'`，从不读 `organizations.{asns,certificates,whois,intel}` 去派生动作。
- **B · 没物化进共享表**：`CIDR` 在 `landing.rs::plan_promotable_assets` 被 `ip.parse::<IpAddr>()` 丢；`CT` 的 SAN/CN 锁在 `organizations.certificates` JSON；`ASN` 段没人读。线索躺在 org JSON 列里，下一阶段根本看不见。

**已核实的关键约束**（决定了方案边界）：

1. 下一阶段（EAS/enumeration）+ coverage gate **只认 `targets.scope='in'`**（`golish-db/src/repo/targets.rs::build_list_in_scope_values_legacy_sql`；`golish-agent-app/src/ai/db_bridge/recon.rs::in_scope_targets_impl`）。
2. intel 现在 **provider-only，无 subfinder**：`recon/subdomain` 工具类型**没有任何 stage 放行**（`tool_taxonomy.rs:46` + 三个 stage spec 的 `allowed_tool_types`；测试 `tool_taxonomy.rs:611` 显式断言 subfinder 不在 EAS）。子域来自 providers（`recon_map_assets`）。
3. `recon_map_assets` **只收 `organization_id`**（`agent_tools/mod.rs::passive_intel_parameters`），不收域名。
4. provider 查询模板**只认 `{{company_name}}`/`{{org}}`，无 `{{domain}}`**（`asset_intel/template.rs:30-31,165-166`），且 provider 按 `company_name` 跑（`service/hydrate.rs`）。

## 2. 总体设计

**核心抽象**（两个机制，不是加工具）：

1. **统一「物化成 targets」登记入口**：不管是被动 intel 发现（CIDR / CT 子域），还是 EAS 主动扫出（证书 SAN / 跳转 / vhost），**全走同一入口登记成 `targets` 行**——带去重 + scope 过滤（`value_belongs_to_organization`）+ 来源标注（`source`）+ 证据（evidence_id / source_query_log）。
2. **有界发现闭环**：发现 → 物化 → 下一步消费 → 又发现 → 再物化……靠 **递归层数 + 新鲜度窗口** 收口，防无限扩。

分三层，各自独立可灰度（I10）：

- **L0 · 种子完整性**：把漏物化的攻击面（CIDR/ASN 段、CT SAN）也落成 `targets`。→ 决定「会不会漏攻击面」。
- **L1 · 种子厚度**：加厚 intel→EAS handoff（`list_in_scope_targets` 投影加宽 + 新 `list_attack_surface_seeds` + 优先级）。→ 决定「扫得聪不聪明、会不会平铺」。
- **L2 · gate 收紧**：EAS `coverage_complete` 升 `authoritative_found`，对齐 target_intel。→ 决定「过 gate 硬不硬」。

## 3. 已拍决策（用户 2026-06-24）

| # | 决策 | 内容 |
|---|---|---|
| **D1** | ASN/CIDR 怎么用 | 物化成 `cidr` 种子（可见）+ 主动端口扫走人审（"看得见，但不乱炸"）。`target_type` 枚举**已含 `'cidr'`**（`20260408000001_initial.sql:172`），**无需改 schema**。主动扫由 EAS `human_approval.required_before:[active_scan]` 卡。 |
| **D2** | 新资产回流 | 默认 EAS 发现的新资产**直接加到对应 org 的 `targets`**；只有当新资产是「**新顶级域/新主体**」时，才对它跑一次 provider 查询（走 b1 按域名查）展开子域树；**递归 1 层**（新域查回的子域不再触发二次 b1）。 |
| **D3** | 规模上限 | **按优先级排序扫**（高置信 + 已解析到 IP + 资产类型重要的先扫）+ **可配置 per-org 上限（默认关，需要时设 N 防失控）**。 |

WHOIS/OSINT 的下游消费（社工 / 凭证 / objective_simulation）**不在本设计范围**——那些阶段未就绪；待就绪后另立设计。

## 4. 各层详细设计

### L0 · 统一物化入口（种子完整性）

**L0a · CIDR/ASN 段 → `cidr` 种子**
- 现状：`landing.rs::plan_promotable_assets` 的 `.filter(|ip| ip.parse::<IpAddr>().is_ok())` 把含 `/` 的 CIDR 丢掉，只留裸 IP。
- 改：新增纯函数把 `org.ip_ranges`（以及 `organizations.asns` 里携带的 prefix/CIDR，**Phase 0 先核实 JSON 形状**）中**解析为合法 CIDR 网段**的留下，物化成 `target_type='cidr'`、`scope='in'`、`source='asset_intel'` 的 `targets` 行（复用 `upsert_target`，加一个 `cidr` 分支）。裸 IP 行为不变。
- D1 人审：`cidr` 种子的主动端口扫由 EAS 的 `human_approval` 卡，本层只负责「登记可见」。

**L0b · CT 的 SAN/CN → `domain` 种子**
- 现状：`organizations.certificates`（JSON）里的 SAN/CN 只贡献 coverage `has_ct`，不变成 `targets`。
- 改：新增纯函数解析 `certificates` JSON 抽 SAN/CN 主机名 → 过 `value_belongs_to_organization`（只留 owned 域，丢第三方/泛域）→ `upsert_target(..., "domain")`（复用 `landing.rs` 现成 + 幂等）。

### L1 · 加厚 handoff（种子厚度）

**L1a · 加宽 `list_in_scope_targets` 投影**（最便宜，列已在行里）
- 现状：`in_scope_targets_impl`（`db_bridge/recon.rs:336`）只 `json!{target_id,value,type}`；底层 `TargetRow`（`targets.rs:95` `TARGET_ROW_COLS`）已带 25 列。
- 改：投影加宽到 `source / status / real_ip / ports / organization_id / http_status / cdn_waf`。零新 SQL（行里已有）。

**L1b · 新工具 `list_attack_surface_seeds`**（结构化 + 优先级）
- 返回每个 in-scope 资产的结构化上下文 + `priority`：`target_id / value / type / source / status / real_ip / dns_records / provider_sources / confidence / org_id / known_ports / last_intel_evidence_ids / priority`。
- 让 EAS prober 能按来源/置信度/可解析性/资产类型制定策略（高置信 root → liveness；已解析 domain → HTTP+TLS+CDN；IP → port probe；cidr → 仅 scope 内确认 + 人审；provider blocked/empty → provenance，不重复跑），而非平铺扫。

**L1c · 优先级 + per-org cap（D3）**
- 纯函数排序（高置信 + 已解析 IP + 资产类型权重）+ 可配置 per-org 上限（默认关）。

### L2 · EAS gate 收紧
- EAS `external_attack_surface/spec.json` 的 `coverage_complete` 现为 `derive_from_evidence:true`（无 `authoritative_found`）→ 自报 cell 仍兜底。
- 改：加 `authoritative_found:true`（对齐 target_intel），灰度 spec flag，**默认 off**（I10；改 gate 行为风险最高，放最后、单独灰度）。

### b1 · 按域名查 provider（支撑 D2 新顶级域）
- `template.rs`：加 `{{domain}}` 占位符（`render_http_template` + `render_asset_intel_skill_args` + `render_http_json_value` 三处）。（注：`docs/.../2026-06-23-provider-direct-landing-gate-tables.md` Task Q3 已识别此 `{{domain}}` 缺口为延后增强，本设计正式落它。）
- `resources/intel-providers/{fofa,quake,hunter,shodan,0-zone}.json`：**逐 provider** 加按域名查询模板（fofa `domain="x"`、quake 等价语法）；不支持 domain 语法的 provider 不加（记 blocked，不硬塞）。
- `agent_tools/mod.rs`：`recon_map_assets` 加可选 `domain` 入参（或新工具 `recon_map_assets_for_domain`）；`service/hydrate.rs` 走 domain 分支（把 `domain` 而非 `company_name` 塞进 render）。
- 落库复用：查回的子域照走 `promote_profile_assets_to_targets` 物化进 `targets`；证据 + source_query_log 照挂。

### D2 递归（有界）
- EAS 主动发现的新资产：**子域/主机** → 直接物化进 targets（无需 b1）；**新顶级域** → 触发对该域一次 b1 provider 查询 → 子域物化 → 进 EAS。
- 递归 **1 层**：b1 查回的子域不再触发二次 b1（防「新域→又挖新域→…」无限展开）。

## 5. 不变量

- **I2 IDOR**：物化的 `targets` 带 `organization_id` + `project_path`，按 org 隔离；b1 的 org 解析复用现有 IDOR guard。
- **I7**：物化/查询挂 evidence_id + `source_query_log` 真实行。
- **I8**：物化是「发现待扫」，与 coverage outcome 正交；`checked_empty`/`found` 仍由 DB truth 决定，物化行不冒充 found。
- **I9**：landing/物化**非事务、非 fatal**（失败只 warn，不回滚已提交 enrich；沿用现有 `land_*` 契约）。
- **I10**：每层独立灰度 flag / 分步可回滚；`target_type='cidr'` 已存在 → **零 schema 改动**；b1 的 `{{domain}}` + domain 模板为加性。
- **§2.7 人审**：`cidr` 种子主动扫走 `human_approval`。

## 6. 验证策略

- 纯函数单测：CIDR 识别 + 物化规划、CT SAN 抽取 + scope 过滤、`{{domain}}` 渲染、priority 排序、per-org cap。
- `cargo nextest -p golish-recon-app -p golish-db -p golish-agent-app -p golish-agent-kit`。
- `cargo clippy ... --all-targets --no-deps -- -D warnings`。
- 活体（用户环境，需重启应用）：跑 target_intel → 只读 DB 复查 `targets` 出现 `cidr` 行 + CT 子域行；`list_attack_surface_seeds` 返回富字段；EAS 扫到 cidr/CT 资产；新顶级域触发 b1。

## 7. 回滚

- 各层独立灰度 flag。L0 物化是**新增写**（去掉 landing 内的两段物化即恢复旧行为）。b1 的 `{{domain}}` + domain 模板是加性（provider 描述符不加 domain 模板即不生效）。L1a 投影加宽是纯展示增强。L2 gate flag 默认 off。零 schema 改动 → 无 migration 回滚负担。

## 8. 分期与依赖（详见实现计划）

| 阶段 | 内容 | 理由 |
|---|---|---|
| **P0** | L0a（CIDR 物化）+ L1a（加宽投影） | 地基 + 最便宜高收益，先做；被动/主动侧都依赖统一物化入口 |
| **P1** | L0b（CT SAN 物化） | 复用 L0 入口 |
| **P2** | b1（按域名查 provider）+ D2 递归 | 新功能，支撑新顶级域展开 |
| **P3** | L1b（`list_attack_surface_seeds` + 优先级）+ D3 cap | 让 EAS 不平铺 |
| **P4** | L2（EAS gate authoritative） | 风险最高，单独灰度，最后 |
