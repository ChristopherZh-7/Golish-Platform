# 2026-06-14 · target_intel 落库管线 + 缺失工具（CT/WHOIS）根治方案

> 起因：run `pentest-chat-1781447402541-1`（目标 pingan.com，模型 deepseek-v4-flash）
> 在 `target_intel` 阶段 per-org recon 子代理反复 `submit_stage_deliverable → needs_fix`
> 18~22 次/org，原文报错 `intel coverage incomplete: never attempted (asset ×
> SUBDOMAIN/CT/WHOIS/...)`。本文用原文日志 + 直查 Postgres + 源码定位根因并给修复方案。

## 0. 根因（证据）

coverage gate 的 `coverage_complete`（`resources/harness/stages/target_intel.json`
gate_rules + `golish-db/src/repo/coverage_truth.rs`）**只认 DB 业务表真值**，每技术落点：

| 技术 | gate 读（coverage_truth.rs） | 当前库内（直查 15432/golish） | 结论 |
|---|---|---|---|
| DNS | `dns_records` 表 (per-asset) | **0 行**（dig 跑了数百次） | 没落库 |
| SUBDOMAIN | `target_assets(asset_type='subdomain')` | **0 行** | 没落库（落到了 org.domains） |
| ASN | `organizations.asns` | 4 org 非空（pingan ✅） | OK（enrich） |
| CT | `organizations.certificates` | **0 org**（全空 `[]`） | 无采集器 |
| WHOIS | `organizations.whois` | **0 org**（全 NULL） | 无落库 |
| OSINT | `organizations.intel/contacts/...` | 11 org（pingan 713KB ✅） | OK（enrich/quake） |

为什么 DNS/SUBDOMAIN 不落库：
- **落库管线存在**：`golish-pentest/src/output_store/`（dns_records.rs → `dns_records::upsert`；
  targets/target_assets via 工具 config 的 `output.db_action="target_add"` + `patterns`）。
- 但本 run 的 recon 子代理：① 用 `recon_enrich_assets`（asset_intel provider）→ 只经
  `asset_intel/profile_patch.rs` 折叠进 **organizations 列**（domains/asns/intel），**不写**
  per-asset 表；② 用 `pentest_run` 跑**裸管道命令**（日志原文 `amass enum -d pingan.com
  -passive 2>&1 | head -5`、裸 `dig`）→ 进 evidence 账本，`| head -5` 截断又打不中
  output_store 的 `patterns` → `dns_records`/`target_assets` 收不到行。
- 净结果：数据进了 evidence 账本 + organizations 列，**没进 gate 读的 per-asset 表** →
  gate 永远判 `never attempted (× DNS/SUBDOMAIN)` → 子代理永远 needs_fix → 死循环（无熔断）。

CT/WHOIS：库里 `certificates`/`whois` 全空，因为**根本没有 CT 采集器**，whois 也没有
落 `organizations.whois` 的路径（`resources/toolsconfig` 里连 `whois.json` 都没有）。

附带证据（已确认，非本文修复目标）：org 树被跨 run 脏数据污染（默安科技/MoreSec 整支 +
重复 org ×2~3 + 14 个 NULL-org moresec target），REUSE 模式把脏树整支带进 scope。

## 1. 修复目标 / 影响面 / 验证 / 回滚

- **目标**：让 `target_intel` 六类技术都能在 DB 真值表落地，使 coverage gate 能据实判
  found / checked_empty，消除"工具跑了但 gate 看不到"的死循环。
- **影响面**：`golish-recon-app`（enrich 落库）、`golish-pentest/output_store`、
  `resources/toolsconfig/*`、`resources/harness/stages/target_intel.json`、recon prompt、
  可能 `golish-db` 新 repo 写函数。**不改 DB schema**（列/表已存在）。
- **验证**：单测（coverage_truth 已有；新增落库写函数单测）+ 端到端跑一次 pingan
  target_intel，直查 `dns_records>0 / target_assets(subdomain)>0 / organizations.certificates
  非空 / .whois 非空` + 看 gate 出 `target_intel` PASS。`just precommit` 全绿。
- **回滚点**：每部分独立 commit；prompt/stage-spec/tool-config 改动可单独 revert；落库
  写函数加在 enrich 之后，失败只告警不阻断主流程。

## 2. 四部分改动（按实现顺序）

### ③ 落库管线（根治，最高优先）
1. **SUBDOMAIN**：`recon_enrich_assets` / subfinder/amass 结果落
   `target_assets(asset_type='subdomain')`（`target_assets::upsert`，父 target = 该 org 的
   根域 target）。最稳做法：enrich 持久化后，把新增子域统一 promote 成 target_assets 行。
2. **DNS**：保证 dig（或 enrich 的 A 记录）落 `dns_records`（`dns_records::upsert`）。让
   recon 走结构化工具路径而非裸 `| head` 管道（prompt + 可选：enrich 直接写 A 记录）。
3. **CT**：新增 crt.sh 采集（`https://crt.sh/?q=%25.<domain>&output=json`）→ 落
   `organizations.certificates`（复用 profile_patch 的 merge_json_array）。
4. **WHOIS**：`whois <domain>` 输出 → 落 `organizations.whois`。

### ① 工具（resources/toolsconfig，走 ToolManager schema，参考 subfinder.json）
- `whois.json`（category recon / subcategory whois，executable 本地 `/usr/bin/whois`）。
- CT：先用 `crt-sh.json`（httpx/curl 查 crt.sh JSON，零新装）；或 `tlsx.json`
  （projectdiscovery，github `go install`）。
- ASN（可选兜底）：`asnmap.json`（projectdiscovery，github）。enrich 已覆盖 ASN，非必须。

### ② 阶段白名单（resources/harness/stages/target_intel.json）
- `allowed_tool_types` 现为 `[recon/dns, recon/subdomain, recon/osint, recon/url-history,
  recon/whois]` → **补 `recon/asn`、`recon/ct`**，否则新工具被 stage_guard 拦。

### ④ Prompt（resources/harness/stages/target_intel.methodology.md + build_recon_prompt）
- 每技术用"会落库"的结构化工具，**禁止裸 `| head` 管道**、禁止把同一批 evidence 贴到多个
  technique 格（伪造覆盖）；查不到的技术老实 `blocked+note`；一个 cell 只引该技术证据。
- （可选）加熔断：子代理同因 needs_fix 连续 N 次 → 自动收尾交回 orchestrator。

## 3. 关键设计决策（需确认）
**子域/DNS 落库放在哪？** 推荐 **A：在 enrich 持久化后统一 promote**（enrich 是本环境唯一
稳定出数的路径——quake 返回 93 候选；而裸 dig/amass 被模型用 `| head` 打残）。
- 备选 B：修 output_store 让裸工具也能落（依赖模型不加 `| head`，不可靠）。
- 备选 A+B：两路都落（最稳，工作量大）。建议先 A，B 作为 prompt 纪律补充。
