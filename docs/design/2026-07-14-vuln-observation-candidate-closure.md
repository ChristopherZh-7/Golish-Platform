# Vuln Observation → Candidate → Verification 闭环

> Status: accepted for implementation
> Date: 2026-07-14
> Scope: `vuln_triage` / `attack_candidate` / `verification` / scoped ContextPack

> **当前执行范围（用户 2026-07-14 恢复全闭环）**：三条 Vuln capability、冻结
> Candidate manifest、精确 Verification replay、evidence operation 身份修复与旧
> `auth_probe` 清理均属于本轮。代码实现必须通过本地确定性门禁，并以指定公司的真实
> CLI stage run 另行证明运行时闭环；两类证据不得互相替代。

## 1. 一句话契约

`vuln_triage` 只调用受控 AI capability 产生可追溯 observation；`attack_candidate`
用“当前任务结构化事实 + 具体 observation + 带来源的记忆提示”逐条决定
Candidate；`verification` 只按服务端冻结的精确计划复现，只有 proof-backed
verified Attempt 才能写 Finding。

```text
Enumeration facts
  │
  ├─ vuln_nuclei_general
  ├─ vuln_nuclei_fingerprint_targeted
  └─ vuln_probe_anonymous_access
          ↓
   evidence + technique_outcomes + typed observations
          ↓ final-sealed handoff
   frozen Candidate manifest
          ↓
   AI candidate / evidenced no_candidate
          ↓ approval
   exact Verification replay
          ↓
   verified / refuted / blocked → Finding only on verified
```

## 2. 现状与问题

### 2.1 Nuclei

- 现有 `vuln_run_formulaic_sweep` 只根据 technique 拼 `-tags`，不会根据当前
  target fingerprint 选择精确 template id。
- 它调用非 guarded 的后台 runner：落库前会过滤越 scope 结果，但不能
  阻止越 scope 请求本身。
- JSONL 解析失败会被静默跳过，可能把未知错误折叠成 `empty`，违反 I8。
- GUI 扫描入口已有 fingerprint→PoC 和 targeted Nuclei，但会直接写 Finding，
  不能作为 `vuln_triage` 产物路径。

### 2.2 匿名访问

- Enumeration 已把 JS/API 中的 URL、method、param name、auth hint 落到
  `api_endpoints` / `js_analysis_results`。
- 旧 `auth_probe` 会猜测路径 id、允许 OPTIONS、未限制 redirect exact origin、
  无界读 body，并且直接写 Finding。
- v1 不实现 A/B 账号 IDOR。它只测试服务端已持久化、已具体化的
  GET/HEAD endpoint，且不猜参数值。
- 新匿名 wrapper 稳定后删除旧 crate、bridge、Chat/sub-agent 注册、策略和仅为旧链路
  服务的 Vault helper，避免两套匿名/越权实现并存。

### 2.3 Candidate

- Candidate V2 的 manifest/final Gate/approval/Attempt/Finding terminalizer 主干已存在。
- `attack_candidate_seeds.observation JSONB` 已可容纳 typed observation，无需新增列；但
  Candidate canonical manifest 现在纳入 `observation + observation_hash` 后，既有 DB shadow
  rebuild function 也必须按同一投影重算 hash，因此需要一个只替换函数的 additive migration。
- 现在的断层是：seeder 把所有 found/empty/blocked/not_applicable 都转成粗粒度
  work item；model-facing manifest 却没有 observation，所以 AI 只看到“某资产×某技术”。
- verifier 依旧按通用 tag 扫根目标，没有重放冻结的 template/url/request。

### 2.4 记忆 / RAG / 知识图谱

底座已存在：持久 Worker checkpoint、canonical/runtime/handoff/episode、long-term
assertion/document、temporal graph、vector layer 和 ContextPack 授权/渲染。但是
stage-run 已绑定 Worker 的 unit identity 没有透传给 ContextPack subject，Candidate
worker 实际可能拿不到 pack。

权威层级固定为：

1. 冻结 scope / canonical DB facts / final-sealed handoff / evidence 是 authority。
2. 当前 Worker checkpoint 和 runtime state 是当前任务短期记忆。
3. assertion/document/RAG/KG/vector 是 `PRIOR_HINT must_revalidate`。
4. 记忆不得改变 scope、tool allowlist、approval、Gate 或 Candidate 计划。
5. 只有历史记忆、没有当前 frozen evidence 的 Candidate 不能被接受。

## 3. 新的 AI capability 表面

### 3.1 共享 adapter registry

```text
VulnAdapterRegistry
  ├─ nuclei.general
  ├─ nuclei.fingerprint_targeted
  └─ anonymous_access
```

每个 adapter 实现同一契约：

- server-owned input schema；
- exact workspace/org/target/origin authorization；
- bounded plan；
- typed parse result；
- evidence-first landing；
- terminal outcome 只能引用刚落地的 evidence；
- `structured_storage_disabled=true` 和 `generic_evidence_disabled=true`，禁止通用
  output-store 越级写 Finding 或重复 evidence。

新增 adapter 只需注册 id/schema/planner/parser/landing，不修改 Candidate/Verification
主干。

### 3.2 `vuln_nuclei_general`

- AI 只传 `target_id + target_url + techniques + bounded timeout`。
- 后端使用固定安全 profile 和 technique→tag mapping；不允许 raw args、proxy、
  template path、output path、Interactsh/update、DoS/fuzz/bruteforce 等扩展。
- 非 N/A 调用只使用已安装的本地 template tree：按环境变量、Nuclei config、
  home fallback 的固定优先级解析并 canonicalize；缺失、空目录、坏配置或无 YAML
  都在零 Nuclei process 的状态下 fail closed。proof 与 active argv 都显式绑定
  shell-quoted `-t <canonical-dir> -duc -dut`，不允许首次运行隐式下载/更新或
  unsigned template。
- 必须 foreground，且在 spawn 紧前重验 TargetWriteGuard。
- 同一个 local-path witness 传给 proof/active；runner 完成异步准备和 DB target
  guard 后同步复验目录，再在本 task 无额外 await 地调用 launch closure。该 witness
  不是 OS 文件锁，不能消除其他线程/进程并发修改造成的底层 TOCTOU。
- JSONL malformed/truncated/non-zero/timeout 一律 `partial/error`，不得记 empty。

### 3.3 `vuln_nuclei_fingerprint_targeted`

- AI 不传 template id。
- 后端从 current-owner fingerprints 和本地 PoC KB 选出、去重、校验安全的
  Nuclei template ids；没有匹配时明确 `no_templates`，绝不 fallback 到 general。
- “精确”指本次用服务端冻结的 exact template-id set 执行，不表示仅靠
  指纹已证明版本受影响；真实命中仍由 Nuclei 响应证据确认。
- hit 必须属于 requested template set 且 matched URL 属于 exact origin。

### 3.4 `vuln_probe_anonymous_access`

- AI 只选择 target 和可选 endpoint ids；URL/method 必须来自 current-owner
  `api_endpoints`。
- 只允许具体 GET/HEAD；拒绝 OPTIONS、mutating method、模板化 URL、危险路由、
  越 exact-origin redirect。
- 使用 fresh no-cookie client，不重放 endpoint 里的 Authorization/Cookie/API-key headers。
- body 有界读取，evidence 只保留 status/final-url/content-type/length/hash/title/
  bounded JSON-key shape，不保留敏感原文。
- verdict 是 `access_controlled | public_expected | suspicious_anonymous_access |
  inconclusive | skipped_unsafe`。v1 不生成 confirmed bypass。
- technique 使用 `WSTG-ATHN-04`（Authentication Bypass），不再把它写成 IDOR
  `WSTG-ATHZ-04`。A/B IDOR 留作后续 adapter。

## 4. Observation 与落库契约

### 4.1 类型

```text
nuclei_match_v1
  source_mode, target_id, exact matched_url, template_id, matcher_name,
  severity, technique, fingerprint_refs, observed_at

anonymous_access_v1
  endpoint_id, endpoint_row_sha256, request_plan_sha256, method, path,
  safe query_bindings, no_auth=true, network_attempted, status_code,
  bounded response/redirect fingerprint, verdict, authority_current_after

surface_analysis_v1
  target_id, target identity, formulaic coverage summary,
  upstream_query_required=true, evidence ids
```

### 4.2 写入顺序

1. 将有界、脱敏的 typed observation JSON 写入 target-bound evidence。
2. 只有 evidence append 成功后，才在 guarded transaction 写 terminal
   `technique_outcomes`。
3. 解析/截断/请求/授权错误保持 `partial/error`，不发布 clean empty。
4. `vuln_triage` 绝不直接写 Candidate/Finding。
5. Candidate seed 只能在 final-sealed handoff 之后由 server materializer 生成。

## 5. Candidate 物化和 AI 输入

- negative coverage 保留为 context，不再每格生成 Candidate work item。
- 每个具体 positive/suspicious observation 生成一个有界 lead work item。
- 每个 target 另生成一个 `surface_analysis_v1` work item，AI 通过已给定的
  `target_live_id` 调 `query_target_data` 综合 JS/path/param/directory/fingerprint 事实。
- 一个 work item 最多接受一个 Candidate，不让单个“整组织摘要”无界产生多个
  Candidate。
- model-facing manifest 必须带 observation + observation_hash；Rust manifest hash 与 DB
  whole-record shadow rebuild hash 必须使用同一 canonical projection 并覆盖它们。
- stage-run 在 provider dispatch 前把 exact frozen manifest 以有界 data block 交给
  analyst；过大则 fail closed，不静默截断。
- `surface_analysis_v1` 要求 AI 显式选择 registry 已支持的 technique；普通 lead
  不得改冻结 technique。

## 6. Verification 精确重放

- `nuclei_match_v1` 派生 `verify.nuclei_template_replay`：canonical args 固定
  exact matched URL + template id + foreground + budget；执行前证明 exact template id
  仍在同一个本地模板 witness 中，active argv 不允许 tags/family fallback。
- `anonymous_access_v1` 派生 `verify.anonymous_request_replay`：canonical args 固定
  endpoint id + endpoint row hash + request plan hash + GET/HEAD + path + query bindings +
  no-auth；重载当前 endpoint 后必须逐字段全等，漂移在发请求前阻断。
- 其他 surface-analysis Candidate 仍由 immutable classifier registry 选择受控 generic recipe。
- model 仍只传 `action_ordinal`。每次执行前从 DB 重读 plan/approval/lease/
  workspace/target guard；不从 model 参数取 target/template/url/header。
- 两种 exact replay 都把真实 harness operation id 写进 evidence ledger，并让 action journal、
  `command.evidence`、Attempt result 的 `proof|refutation|blocker` 引用三方全等；没有 evidence
  的失败 action 可以终结为 blocker 以避免 `started` 悬挂，但不能提交 Attempt。

## 7. 安全与非目标

- 本实现不新增表或列。经用户明确授权，新增 additive migration
  `20260714000001_candidate_observation_shadow_hash.sql`，只 `CREATE OR REPLACE` Candidate
  shadow rebuild function，使 DB 端 canonical manifest 与 Rust 端同样覆盖
  `observation + observation_hash`；历史 migration 保持不可变。
- 本地自动测试只用 pure parser/planner、fake runner 和 loopback HTTP fixture；不请求外部目标。
- 不实现 A/B 账号 IDOR、低权到高权 privilege escalation、mutating endpoint 测试。
- 不允许记忆/RAG/KG 直接证明漏洞；它们只用于提高 Candidate 排序和假设质量。
- 不恢复全局 legacy memory fallback，不跨 operation/org/project 检索。

## 8. 完成定义

下列链条必须有自动化证据：

```text
authorized enumerated input
→ guarded capability execution
→ typed redacted evidence
→ terminal/partial technique outcome
→ final-sealed handoff
→ frozen observation manifest visible to analyst
→ candidate/no_candidate final Gate acceptance
→ exact immutable verification plan
→ ordinal-only replay
→ verified/refuted/blocked terminal result
```

`just precommit` 和不访问外部目标的集成测试全绿后，只能宣称代码闭环通过；本功能还
要求对用户指定公司完成 Scoping→Vuln→Candidate→Verification 的真实 CLI acceptance，
并同时核对 `run.log`、`transcript.json`、`run_tree.py --full --db` 与数据库事实后才可
把 feature 标为 `passing`。
