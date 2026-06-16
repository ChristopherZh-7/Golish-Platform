# 2026-06-16 · AI run 可追踪性评估（observability assessment）

> 目标（用户提出）：清空旧数据重跑后，让以后的 AI 看测试数据能 **① 快速发现问题**、
> **② 所有调用 + 所有 sub-agent + sub→sub 嵌套链都能追踪串联**、**③ 看得简单**。
>
> 本评估基于实跑 `stage-run-c3468620`（profile=red_team，only=target_intel，
> org=pingan，target=pingan.com，deepseek-v4-flash）的真实日志/transcript/DB 取证，
> 非纸面推断。

## 1. 数据源盘点（现状）

| 源 | 位置 | 内容 | 备注 |
|---|---|---|---|
| 全局 tracing | `~/.golish/backend.log` | 全进程 tracing（GUI 用） | CLI run 此文件可能为空（CLI tracing 走 stdout） |
| 单 run 时间线 | `{ws}/.golish/transcripts/<session>/run.log` | 主 agent + 子 agent + 工具命令 + harness gate + evidence，按 span 路径汇一条线 | **本次实测存在(19K)，CLI 已与 transcript 同目录** |
| 结构化事件 | `<session>/transcript.json` | 主 agent 工具调用/结果/参数（JSONL） | |
| 子 agent 事件 | `<session>/subagents/<agent_id>-<parent_req>::org::<org_id>/transcript.json` | 每个子 agent 一份 | **目录平铺**，靠 `parent_request_id` 关联 |
| 业务真值 | 嵌入式 PG（organizations/targets/target_assets/dns_records/audit_log…） | coverage gate 的 found/empty 真值 | 日志看不到，需查库 |

## 2. 实测证据（stage-run-c3468620）

- **调用链标注齐全**：run.log 每行带 span 路径 `chat_message:agent:sub_agent` +
  `agent_type=sub-agent:recon depth=1` + `job_id` + 工具命令行，主→子可串。
- **coverage landing 摘要行**（`golish_recon_app::asset_intel::agent_intel`）：
  `subdomains=N dns_records=N certificates=N whois=bool rdns=N ip_whois=N` —— 一行看本 run 各落点计数。
- **enrich provider 状态**：`provider=0.zone … state=Completed candidate_count=179 profile_field_count=828`、
  `provider=quake … state=CheckedEmpty`。
- **stage 边界可见**：`harness::stage_guard … BLOCKED by stage boundary tool=pentest_run reason=…enscan-go not permitted…`。
- **background job 闭环可见**：`[background-listener] job finished … status=done` + `background job evidence appended evidence_id=N`。

## 3. 评估（需求 × 现状 × 缺口）

| 需求 | 现状（实测） | 评分 | 缺口 |
|---|---|---|---|
| ② 全链路串联 | run.log 单时间线含主+子+工具命令+job_id；span 路径标注 depth/agent_type | 🟡 部分 | sub→sub 嵌套是**平铺 transcript 目录**，靠人肉对 `parent_request_id`，**无可视调用树** |
| ① 快速发现问题 | 有 coverage landing 摘要 + enrich state + evidence_id + stage BLOCK 原文 | 🟡 部分 | **gate「为什么 found/never-attempted」的 DB 真值依据不入日志**，必须查库（asset×technique 逐格来源） |
| ③ 看得简单 | run.log 一条线已不错 | 🟡 部分 | 仍要拼 run.log + transcript.json + DB 三处；**无「一眼全貌」的调用树视图** |

## 4. 建议（按性价比）

1. **Run 调用树总览工具**（最贴 ②③）：从 transcript + subagents/* 重建
   `main → sub(recon) → sub → 工具调用(+job 状态) → submit(gate 结论)` 的**一棵可读树**，
   一条命令打印；嵌套靠 `parent_request_id` 自动连。
2. **gate 裁决落日志**（最贴 ①）：`submit_stage_deliverable` 评估时，把 coverage 逐格裁决
   （每 asset×technique 的 found/empty/never + 来源表 dns_records/target_assets/organizations.*）
   写进 run.log/transcript，未来 AI 不查库即懂为何 BLOCK。
3. **嵌套 transcript 成树/加 chain 索引**：让 sub→sub→sub 可直接 walk（或 run.log 输出树形）。

## 5. 验证清单（本 run 收尾后逐项核）

- [ ] target_intel gate 最终 PASS？（证 3 个修复生效）
- [ ] DB：targets.organization_id 非 NULL（证 P0 org-link 兜底）
- [ ] audit_log 出现 `GOLISH-INTEL-SUBDOMAIN` found 事实（证配套 a 路径解析）
- [ ] organizations.asns/certificates/intel 非空 → ASN/CT/OSINT found
- [ ] 若仍 BLOCK：是否在连续 3 次相同后熔断（证 circuit breaker），而非烧满 40 iter
