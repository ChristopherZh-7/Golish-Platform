# 2026-06-23 · provider 直接落库到 gate 真值表（DNS / SUBDOMAIN 去桥）

> 日期：2026-06-23
> 状态：设计（待用户 sign-off；**无 schema 改动**，纯写路径重构 + 落点新增）
> 作者：BaJie BajieAsk-agent-1（全栈工程师）· 与用户对话产出（DISPATCH off）
> 关联：
> - `docs/superpowers/plans/2026-06-17-passive-intel-pairing-probe-landing.md`（enrich landing + refresh_per_asset_landing 现状）
> - `docs/superpowers/plans/2026-06-18-slim-enrich-provider-rdap.md`（CT/WHOIS 移出 enrich；DNS 留 gate-refresh）
> - `docs/design/2026-06-22-intel-perdim-freshness-slim-deliverable.md`（每维新鲜度；本设计与之协同）
> 不变量：I7（阶段交付有 evidence）、I8（已检查为空 ≠ 未检查）、I9（事务内不调外部 HTTP）、§2.5（gate 确定性）、§2.7（无 schema 改动，免 SQL 复看）

---

## 0. 一句话

provider 测绘**已经**拿到 domain↔IP 配对（和子域），却只落进 `targets.real_ip`；DNS/SUBDOMAIN 的 gate 真值表（`dns_records` / `target_assets`）靠一个**单独的、gate-time 实时再解析**的桥（`refresh_per_asset_landing` → `land_dns_records` 的 `lookup_host`）来补——脆弱（解析超时/失败 = `dns_records` 空，即当前 live DB 现象）。本设计让 provider landing **直接**把已知的 domain↔IP 写进 `dns_records`、子域写进 `target_assets`；旧桥靠它已有的 `NOT EXISTS` 守卫**自动降级**为「provider 没覆盖的格子」的 fallback。

## 1. 现状勘验（2026-06-23 已读真实代码）

| 维度 | provider 落点 | gate 读点 | 现状 |
|---|---|---|---|
| ASN/CT/WHOIS/OSINT（org 级） | ProfilePatch → `update_profile` | `organizations.*` | ✅ **直接落**，通（live DB: ASN/WHOIS/contacts 有数据） |
| DNS | `targets.real_ip`（`landing.rs::upsert_target`） | `dns_records` | ❌ 靠 `land_dns_records` 实时再解析（`lookup_host` 3s，易超时→空） |
| SUBDOMAIN | （不写） | `target_assets[subdomain]` | ❌ 靠 `land_subdomain_assets`（从 in-scope targets 配对），provider 配对的 host 未直接落子域 |

关键代码：
- `golish-recon-app/src/asset_intel/landing.rs::promote_profile_assets_to_targets` / `upsert_target`：手里有 `HostIpPair{host, ip}`，写 `targets`+`real_ip`，**不写** `dns_records`/`target_assets`；`upsert_target` 返回 `bool`（不暴露 `target_id`）。
- `organization_recon/persistence.rs::land_dns_records`：`WHERE NOT EXISTS(dns_records)` + `lookup_host` 实时解析 → `dns_records(source='resolver')`。
- `persistence.rs::land_subdomain_assets`：subdomain↔root 配对 → `target_assets::upsert(target_id=root, 'subdomain', value)`。
- `refresh_per_asset_landing`（在 `golish-agent-app/ai/db_bridge/recon.rs:278` gate-time 调）= `land_subdomain_assets` + `land_dns_records`。

## 2. 提案（方案 A：直接落，桥降级 fallback）

在 provider landing（`upsert_target` 成功、拿到 `target_id` 后）**直接**写：

1. **dns_records**：把 provider 的 `(host → ip)` 当 A/AAAA 记录写入
   `dns_records(target_id, 'A'|'AAAA', name=host, value=ip, source='<provider>')`。
   幂等：现有唯一键 `(target_id, record_type, name, value)` `ON CONFLICT DO NOTHING`（已有，见 `dns_records::upsert`）。
2. **target_assets**：若 `host` 是某 owned root 的子域，写
   `target_assets(root_target_id, 'subdomain', value=host, source=provider)`。复用 `land_subdomain_assets` 的 root→target_id 解析。
3. **CT（修 Quake 提取 + 确保发查询，2026-06-23 活体定位）**：CT 数据 provider 早能给——Quake 免费档 `cert: "<域名>"` 一次 **24,615** 命中（活体实测，见 §7）；`map_cert` 设计上直接落 `organizations.certificates`（无桥）。**但活体探测 Quake 返回结构发现：cert 不在顶层 flat `cert` 字段，而在 `service.cert`（+ `service.tls.handshake_log.server_certificates`）**——而 `QuakeService.cert` 只读顶层 flat `cert`、`QuakeInnerService` 未捕获 `service.cert` → `map_cert` 取到 `None` → **Quake CT 永远落不进 `organizations.certificates`**（live DB `certificates=0` 的 Quake 侧确定根因）。修：
   - `QuakeInnerService` 加 `cert: Option<String>`（反序列化 `service.cert`）；
   - `quake::mapper::{map_cert, map_site}` 读 `svc.service.as_ref().and_then(|s| s.cert.clone())`，回退顶层 flat `svc.cert`；
   - enrich 的 cert 查询用**域名**（`cert: "pingan.com"`，干净）而非公司名（`cert: "中国平安"` 命中更多但混入非本主体噪声，见 §7）。

桥（`land_dns_records` / `land_subdomain_assets`）**不删**：`land_dns_records` 的 `WHERE NOT EXISTS(dns_records)` 守卫使它自动只补 provider 没落的域名（fallback，且仍是「想要实时新鲜」时的兜底）。

## 3. 与每维新鲜度（2026-06-22）协同

provider 直接落的 `dns_records` 行 `created_at = NOW()`（本轮）→ Phase B 行级窗（`dns_records.created_at >= run_start`）判它新鲜。`real_ip` 本就用 provider 值；DNS 用**同一份** → 二者一致（消除「real_ip 信 provider、dns_records 要实时」的口径不一致）。provider 缓存 vs 实时新鲜的取舍交给 freshness 窗：上一 run 落的行被窗排除，本 run 重落即新鲜。

## 4. 取舍 / 红线

- provider DNS 是「provider 索引声明」，非实时解析 → `source='<provider>'` 记清来源（可观测 / 可信度区分；fallback `land_dns_records` 仍写 `source='resolver'`）。
- I9：landing 在事务外（现有 `promote_profile_assets_to_targets` 已是非事务、非 fatal）；新增的 `dns_records`/`target_assets` 直接写同样**非 fatal、失败只 warn**，绝不回滚已提交的 enrich。
- **不改 schema、不改 gate 判定纯函数**（`coverage_truth` / `rule_engine` 一行不动）。

## 5. 影响面

- **写**：`landing.rs`（`upsert_target` 返回 `Uuid` + 新增 dns_records/target_assets 直接写 + root 解析）。
- **读**：无（gate 读点 `dns_records`/`target_assets` 不变）。
- **测**：`landing.rs` 既有 pure 规划单测保留 + 新增「直接落 A 记录 / 子域」的调用断言。
- **风险**：provider 给的 ip 可能不是合法 IP（脏数据）→ 落 dns_records 前 `parse::<IpAddr>()` 过滤（与 `plan_promotable_assets` 一致）。

## 6. 备选（未采纳）

- **B 直接落 + 可选实时增强**：直接落 provider 值兜底 + 有余力再实时解析覆盖。比 A 多一层，YAGNI——freshness 窗已处理新鲜度，fallback 已在，暂不需要。
- **删桥**：不删——`land_dns_records` 仍是「域名没被 provider 配对到」时的唯一来源 + 实时新鲜兜底。

## 7. 活体证据 + 根因（2026-06-23 Quake 实测 · 用户提供 token，token 不入库/不外泄）

实跑 Quake v3 API（`POST /api/v3/search/quake_service`，quota 45495/月）：

| 查询 | total 命中 | 备注 |
|---|---|---|
| `cert: "pingan.com"` | **24,615** | 域名查，干净（pa18-pweb.pingan.com / m.pingan.com + 大量带证书 IP） |
| `cert: "中国平安"` | **71,055** | 公司名查，命中更多但更噪（混入 hwclouds / ph.com.cn 等非本主体） |
| `domain: "pingan.com"` | **75,451** | 海量子域（pacdn-download.stock / edu.stock / life / chexian ...） |

**结论 1 · 数据源完全不缺**：Quake 光 pingan 就 2.4 万证书 / 7.5 万子域记录。live DB 的 CT/DNS/子域=0 **不是没数据**，是采集→落库管线没接住——印证本设计（直接落库）的必要性。

**结论 2 · Quake CT 不落库的确定根因（结构探测）**：返回记录**无顶层 flat `cert`**（`top-level keys` 不含 `cert`）；cert 实际在 `service.cert`（present）+ `service.tls.handshake_log.server_certificates`。Golish `QuakeService.cert` 读顶层 flat、`QuakeInnerService` 未取 `service.cert` → `map_cert` 取 `None` → Quake CT 永远不落 `organizations.certificates`。**非查询词问题、非数据源问题、非 FOFA-only**——是 Quake 反序列化漏了 `service.cert`（修法见 §2.3）。

**对「数据量够不够」的回答**：数据天花板极高（Quake 单 org 7.5 万子域、2.4 万证书），平台实际覆盖被**管线/落库 + 上述提取 bug**卡住，不是被数据源卡住。补齐落库 + 修 Quake cert 提取 = 覆盖大涨。
