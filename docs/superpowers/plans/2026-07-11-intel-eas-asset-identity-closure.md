# Intel / EAS 资产身份与关系闭环实现计划

> 面向 AI：严格按任务顺序执行。每个行为修复先写会失败的测试，再做最小实现；只运行任务对应的 scoped 验证，用户已明确要求不要运行 `init`。

**目标：** 修复 Scoping→Target Intel→External Attack Surface 中域名/IP 多对多关系、组织 ownership、provider 终态、EAS alias/exact-origin 与 wave 漏尾问题，并用默安科技 `moresec.cn` 的非交互 CLI 得到可审计闭环。

**架构：** `targets` 保存组织授权下的可执行身份，`dns_records` 保存不扩权的 Domain→IP 观察边，`network_endpoints` 与 `web_origins` 分别保存网络端点与 exact Web Origin；`real_ip` 仅作 primary cache。EAS 保留 target 四轴兼容矩阵，并增加 required-origin minus completed-origin 的确定性 barrier。

**技术栈：** Rust 2021、sqlx/Postgres、cargo nextest、JSON toolsconfig、现有 `stage_smoke.py`/`run_tree.py`。

---

## Task 0：锁定 Scoping trusted intake 与精确 review

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/*`
- 修改：`backend/crates/golish/src/stage_run/mod.rs`
- 修改：`frontend/components/AIChatPanel/ScopeReviewTable.tsx`
- 修改：`resources/harness/stages/scoping/{spec.json,methodology.md}`

**步骤：**

1. 红测：trusted UI/CLI seed 在 Scoping 前已落 exact target；Scoping/Target Intel
   没有 `manage_targets`，模型无法变更 scope。
2. backend 从当前 org 只读 `manual/imported/stage-run-seed/seed/cli` 来源的
   trusted snapshot，对 `scope_review` 精确比较 canonical value + type + scope；
   外层 ask_human result JSON 与内层 response 均必须合法，缺失/编辑 fail closed。
3. 前端未编辑确认原样保留 type/scope；编辑只返回 proposal，必须先由
   trusted intake 写回 DB 才能通过下一次 review。
4. CLI `--target` 在进 Scoping 前写 trusted seed，headless auto-review 只回显该
   snapshot，不从 objective/LLM context 造 target。
5. `DbTracker::start_tool_call/finish_tool_call` 改为可 await 的有序生命周期，避免
   `scope_review` 完成先于 start row 而被 gate 误判缺失。

## Task 1：锁定资产规范化与 Intel landing

**文件：**

- 修改：`backend/crates/golish-recon-app/src/asset_intel/landing.rs`
- 修改：`backend/crates/golish-recon-app/src/organization_recon/persistence.rs`
- 修改：`backend/crates/golish-db/src/repo/targets.rs`
- 测试：上述模块内现有 test modules

**步骤：**

1. 新增红测：同 hostname 多 IP 不丢；apex/www 不合并；certificate hostname 保留 www；primary IP 稳定；provider pair/host-only 的完整 URL 只提升 concrete hostname，不能写成 domain 型 URL。
2. 新增红测：passive DNS 只写关系、不自动生成未授权 IP target，且 `set_real_ip` 不标记 alive。
3. 新增红测：target/service lookup 以 org + type + exact value 隔离，legacy null-org 可认领，sibling org 不可复用。
4. 新增红测：`organizations.domains/app_domains/ip_ranges` 只是 metadata，不能创建
   新 root/IP/CIDR target；WHOIS 仅从 trusted target snapshot 取根。
5. 新增红测：wildcard 本身不执行，Target Intel 只有 SUBDOMAIN 格，
   `found` 必须有真实 strict-child domain target；apex 不在 `*.` 授权内。
6. 运行 `cd backend && cargo nextest run -p golish-recon-app landing persistence --status-level fail`，记录 RED。
7. 实现 pair 级去重、URL→concrete-host 规范化、身份与归属判断分离、org-scoped upsert/lookup、确定性 primary IP；本轮 candidates/observations 与累计人工 review queue 分离，禁止历史 targets 重配刷新 freshness。
8. 运行同一命令至 GREEN。

## Task 2：修复 Intel terminal 与 freshness

**文件：**

- 修改：`backend/crates/golish-recon-app/src/asset_intel/runtime/native.rs`
- 修改：`backend/crates/golish-recon-app/src/asset_intel/mod.rs`
- 修改：`backend/crates/golish-recon-app/src/organization_recon/persistence.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
- 修改：`backend/crates/golish-db/src/repo/coverage_truth.rs`
- 修改：`resources/harness/stages/target_intel/spec.json`

**步骤：**

1. 红测覆盖 native all-error、reachable-empty、失败仍落 source row、source error 不闭 checked-empty、WHOIS 旧值不伪造 empty。
2. 实现 attempted/succeeded/error 分类和 typed WHOIS outcome；失败尝试仍写 audit/source。
3. 删除 generic provider found 自动补齐其他 technique 的兼容投影。
4. freshness 对 `target_assets` 使用 `GREATEST(discovered_at, updated_at)`；DNS 冲突刷新当前观察时间。
5. spec 设置 other/blocked 必须有 note，error 非 terminal。
6. DNS per-asset refresh 对全部 domain target 做稳定 128-concurrency 分块；补 401 条
   `128 + 128 + 128 + 17` 无遗漏回归，禁止 SQL target LIMIT。
7. Hickory 显式分别查 A/AAAA 并识别 `NoRecordsFound`；OS resolver 只做正向
   fallback。native mixed records+error 保持 Failed；duplicate guard 同审 generic、
   exact technique suffix 与 DNS error/partial/running，保证非终态可重试。
8. 运行：
   `cd backend && cargo nextest run -p golish-recon-app -p golish-agent-kit -p golish-db -E 'test(native) | test(whois) | test(source_) | test(freshness)' --status-level fail`。

## Task 3：取消 EAS 整资产 alias 折叠

**文件：**

- 修改：`backend/crates/golish-app-core/src/domain/targets.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/org_gate.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`

**步骤：**

1. 红测：`IP + apex + www + sibling vhost` 全保留；domain/url cell 为 LIVENESS+WEB，PORT/SERVICE N/A。
2. 红测：CIDR 行只有 LIVENESS/PORT，guarded in-range child IP 进 supplemental
   wave 后才有 SERVICE/WEB；wildcard 行 EAS/Enumeration 全 N/A。
3. 删除 seed ranking、org gate、submit preview、coverage read-model 对 alias 整行剔除。
4. 继续由 technique resolver 决定各 asset 的 required/N/A 技术。
5. 运行：
   `cd backend && cargo nextest run -p golish-app-core -p golish-agent-kit -p golish-agent-app -E 'test(attack_surface_seed) | test(alias) | test(stage_coverage) | test(org_gate)' --status-level fail`。

## Task 4：让 HTTP landing 保存 exact origin

**文件：**

- 修改：`resources/toolsconfig/httpx.json`
- 修改：`resources/toolsconfig/whatweb.json`
- 修改：`backend/crates/golish-pentest/src/output_store/helpers.rs`
- 修改：`backend/crates/golish-pentest/src/output_store/targets.rs`
- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/eas_capabilities.rs`

**步骤：**

1. 红测：httpx 显式/默认 port 与 scheme；WhatWeb reason phrase 和无插件输出；两 vhost 共 endpoint 仍有两个 origins/observations。
2. 配置 httpx 映射 `port/scheme/content_type`，helper 从 URL 可靠推导缺失字段。
3. foreground guarded landing 同步 upsert `web_origins`、`network_endpoints`、`web_origin_observations`。
4. WEB-FINGERPRINT outcome 使用 canonical exact origin；authorization 允许同 target 的不同 origin。
5. 运行 JSON 校验和：
   `cd backend && cargo nextest run -p golish-pentest -p golish-pentest-app -E 'test(httpx) | test(whatweb) | test(web_origin) | test(authorize_launch)' --status-level fail`。

## Task 5：增加 EAS exact-origin barrier

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/org_gate.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/*`
- 修改：`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`

**步骤：**

1. 先写纯函数红测：两个 scheme/port、两个 vhost、缺 evidence、查询失败、partial coverage。
2. Repo provider 读取 current-org/current-cutoff 的 target-bound required origins；错误 fail closed。
3. org close 和 submit preview 共用 `required - completed` barrier。
4. coverage details 暴露 `required_origins/completed_origins/missing_origins`。
5. 运行：
   `cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -E 'test(origin) | test(external_attack_surface) | test(harness_submit_tool) | test(stage_coverage)' --status-level fail`。

## Task 6：修复 200-asset wave 漏尾

**文件：**

- 修改：`backend/crates/golish-db/src/repo/stage_asset_waves.rs`

**步骤：**

1. 红测：401 个阶段前资产按 `200 + 200 + 1` 分页；运行中新资产进入下一波；无重复。
2. 下一波只排除已分配到任一当前 stage wave 的 target，移除 parent-started-at 时间下界。
3. 将最后一次候选读取与 completion watermark 放进同一短事务，并用 wave/target 表锁等待在途 writer；有候选则建下一波，无候选才原子发布 org completion。
4. completion 写入当前 operation UUID；stage_run resume/token 与 orchestrator final
   closeout 必须读取 `stage_run_id` 并精确匹配当前 operation，不能只看 fresh timestamp。
5. 运行 `cd backend && cargo nextest run -p golish-db stage_asset_wave --status-level fail`。

## Task 6.5：关闭 terminal preview、最终 gate 与 worker 生命周期

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/tool_executors/security.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/*`
- 修改：`backend/crates/golish-db/src/repo/technique_outcomes.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor/*`
- 修改：`backend/crates/golish-tools/src/definitions/security_tools.rs`

**步骤：**

1. Target Intel/EAS preflight 对权威 snapshot 预演 checked_empty/blocked/N/A；拒绝 found、重复格、非 snapshot 格和无 evidence 的 checked_empty。
2. `submit_stage_deliverable=accepted` 后 specialist/main loop 都立即持久化 checkpoint 并停止；同一 tool-call batch 余下调用也必须跳过并返回配对结果。
3. 最终 per-org Gate PASS 后只物化 blocked/N/A，条件 upsert 不覆盖 found/empty/已有终态；snapshot/read/write 失败必须 fail closed，producer 已抢占 terminal truth 时才允许零更新。
4. DB truth read 路径保持纯读，DNS refresh 只留在成功 `recon_map_assets` 写路径。
5. 补 accepted/needs_fix 分流、preview、条件 upsert、PASS/BLOCK 物化测试。

## Task 7：模块文档与 scoped 静态门禁

**文件：**

- 更新受影响的 `docs/modules/backend/*.md`
- 更新：`docs/modules/INDEX.md`
- 更新：`agent-progress.md`
- 更新：`feature_list.json`

**步骤：**

1. 同步资产/授权/landing/gate/wave 新合同和测试入口。
2. 运行 `python3 -m json.tool feature_list.json resources/harness/stages/scoping/spec.json resources/harness/stages/target_intel/spec.json resources/harness/stages/external_attack_surface/spec.json resources/toolsconfig/httpx.json resources/toolsconfig/whatweb.json`。
3. 运行 selected `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与相关 crate nextest。
4. 不运行 `./init.sh`；用户已明确跳过。

## Task 8：非交互 CLI 实跑与 DB 审计

**步骤：**

1. 用本地 fixture 跑 EAS exact-origin smoke，确认 runner、landing、gate。
2. 执行：

   ```bash
   python3 scripts/stage_smoke.py \
     --profile red_team \
     --from scoping \
     --to external_attack_surface \
     --workspace /tmp/golish-moresec-full \
     --org "默安科技" \
     --target moresec.cn \
     --json --run-tree \
     --objective "验证默安科技已授权 moresec.cn 的 scoping、target_intel、external_attack_surface 完整闭环；不要纳入子公司，不要扩展到 DNS 仅关联但未明确授权的共享或 CDN IP。"
   ```

3. 用 `scripts/run_tree.py --workspace /tmp/golish-moresec-full --full --db` 复核；必要时按同一 session 精确续跑，不重建 scope。
4. 上述三阶段 PASS 后，核对 target-bound `web_origin_observations`、exact-origin
   WEB outcomes 与 stage handoff 状态；本工作项只验证 Enumeration 输入合同，不重跑
   用户已经修复的 Enumeration 阶段。
5. 记录每阶段 pass/block、首要 blocker、DB targets/DNS/origins/outcomes/evidence 统计；只有新鲜证据齐全才把 feature 改为 `passing`。
