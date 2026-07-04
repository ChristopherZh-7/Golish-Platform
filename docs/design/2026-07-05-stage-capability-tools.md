# Stage Capability Tools：把阶段能力包装成可控工具

> Status: proposed
> Date: 2026-07-05
> Owner: Golish harness / agent runtime

## 1. 背景

Golish 现在的 harness 已经有三层关键保护：

1. `StageSpec.allowed_tool_types` + `tool_taxonomy.rs`：按阶段限制可运行的扫描工具。
2. `stage_worklist_status` / `stage_worklist_next` / `check_stage_asset_coverage`：从 DB/gate truth 生成当前缺口。
3. evidence ledger / `technique_outcomes` / business tables：工具结果必须落库，gate 只认确定性事实。

但 AI 当前看到的主要仍是底层工具名：

- `StageAssetCoverageCell.suggested_tools`
- `CoverageGapAction.suggested_tools`
- `stage_worklist_next.items[].suggested_tools`
- `stage_refiner` 的 `RepairAction.tool`
- `stage_run_call` objective 里的 concrete tool list

这会让模型把“我要完成什么能力”和“我要怎么拼命令”混在一起。典型问题是：SERVICE-FINGERPRINT 缺口本应是“对已确认 open ports 做服务指纹”，但模型容易自己拼 `nmap -sV -iL`、把 URL 喂给 nmap、或者把 WhatWeb 当通用服务指纹工具。

本设计的核心判断：

> AI 应该选择阶段能力和处理顺序；后端应该决定底层工具、参数模板、输入过滤、落库和 evidence。

也就是说，不是把 `nmap` 包成一个万能工具，而是把 `eas.fingerprint_services` 这类阶段能力包成工具。`nmap` 只是这个能力在当前环境下的一种实现。

## 2. 目标

- 给每个 stage 建立一份 machine-readable capability registry。
- 让 coverage/worklist/refiner/stage objective 输出 `suggested_capabilities`，而不只输出 `suggested_tools`。
- 保留 `suggested_tools` 作为兼容字段，直到 UI 和 worker 都迁完。
- 让 AI 的默认动作从“自由拼命令”变成“调用能力或按能力 recipe 调工具”。
- 高风险能力最终由 backend wrapper tool 固定参数、固定输入范围、固定落库路径。
- gate 继续只认 DB truth / evidence，不认模型自报。

## 3. 非目标

- 第一阶段不改 DB schema。
- 第一阶段不替换现有 `pentest_run` / bridge tools。
- 不把全部工具一次性改成 wrapper。
- 不取消 `allowed_tool_types` 白名单。capability 是计划语义，tool taxonomy 仍是执行安全边界。
- 不让模型通过 capability 绕过 human approval、scope、profile authorization。

## 4. 设计原则

### 4.1 能力是业务动作，不是工具名

错误抽象：

```text
nmap 能做什么
```

正确抽象：

```text
eas.fingerprint_services
  closes: GOLISH-EAS-SERVICE-FINGERPRINT
  allowed stage: external_attack_surface
  input: confirmed IP/CIDR host assets with open ports
  implementation: nmap -Pn -sV on confirmed host:port groups; WhatWeb only for confirmed HTTP(S) endpoints
  writes: fingerprints, network_endpoints, technique_outcomes, audit evidence
```

AI 看到的是 `eas.fingerprint_services`。后端内部才知道当前用 `nmap`、是否允许 `whatweb`、怎么拼参数、哪些资产要跳过、什么结果写入什么表。

### 4.2 capability 是 stage-local

同一个底层工具在不同 stage 的含义不同。`httpx` 在 EAS 是 liveness / HTTP fingerprint；在 Enumeration 不应作为主动 HTTP 探测工具重新开放。capability 必须绑定 stage 和 technique。

### 4.3 wrapper 优先保护输入边界

能力 wrapper 的第一职责不是“省 prompt”，而是：

- 限制资产只能来自当前 `stage_worklist_next` / DB in-scope worklist。
- 过滤不适用资产，例如 domain/url 不能进入 PORT/SERVICE batch。
- 复用已确认 open port，禁止 speculative broad sweep。
- 统一 timeout/background/job attribution。
- 固定 evidence / `technique_outcomes` 写入。

### 4.4 metadata 先行，runner 渐进

先落 capability registry 和 `suggested_capabilities`，让所有 prompt/refiner/worklist 口径统一；再给最容易出错、最耗时、最高风险的能力补 wrapper runner。

## 5. 现有代码落点

当前改造涉及的主要 seam：

| 现有位置 | 当前职责 | 改造方向 |
|---|---|---|
| `golish-agent-kit/src/harness/tool_taxonomy.rs` | 工具分类和 stage 白名单 | 保持执行 guard，不承载 capability 语义 |
| `golish-agent-app/src/ai/commands/stage_coverage.rs` | UI/agent coverage snapshot，生成 `suggested_tools` | 增加 `suggested_capabilities`，`suggested_tools` 从 capability 派生 |
| `golish-agent-kit/src/tool_executors/security.rs` | `stage_worklist_*` / coverage compact JSON | worklist item 输出 capability suggestions |
| `golish-agent-kit/src/harness/gate/rule_engine.rs` | coverage gap recovery action | `CoverageGapAction` 兼容增加 capability suggestions |
| `golish-agent-kit/src/task_orchestrator/stage_refiner.rs` | needs_fix / gate BLOCK 的修复指令 | repair action 优先说 capability，再给 tool hint |
| `golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs` | specialist objective | worker objective 改为 capability-first |
| `golish-pentest-app/src/pentest_bridge/*` | direct bridge tools和内容枚举落账 | 作为部分 capability 的现有实现 |
| `frontend/components/Engagement/StageAssetCoveragePanel.tsx` | coverage UI | tooltip / 状态摘要优先显示能力，不暴露太多底层工具 |

新增纯模块建议放在：

```text
backend/crates/golish-agent-kit/src/harness/stage_capability.rs
```

它是 IO-free、DB-free、可单测的 capability registry。`agent-kit` 已拥有 `StageKind` / stage spec / tool taxonomy，所以它是最合适的第一落点。不要把 golish-db 或 app bridge 依赖塞回 `agent-kit`。

## 6. 核心数据结构

第一阶段只需要纯 DTO / 常量映射。

```rust
pub struct StageCapabilitySpec {
    pub id: &'static str,
    pub label: &'static str,
    pub stage: StageKind,
    pub techniques: &'static [&'static str],
    pub tool_names: &'static [&'static str],
    pub allowed_tool_types: &'static [&'static str],
    pub risk: CapabilityRisk,
    pub batchable: bool,
    pub max_batch: usize,
    pub writes: &'static [&'static str],
    pub runner: CapabilityRunnerKind,
}

pub enum CapabilityRisk {
    Passive,
    Active,
    Exploit,
    PostExploit,
}

pub enum CapabilityRunnerKind {
    MetadataOnly,
    ExistingDirectTool,
    PentestRunRecipe,
    BackendWrapper,
}
```

对外展示给 agent/UI 的字段用更小的 serializable suggestion：

```rust
pub struct StageCapabilitySuggestion {
    pub id: String,
    pub label: String,
    pub tools: Vec<String>,
    pub risk: String,
    pub batchable: bool,
    pub max_batch: usize,
    pub reason: String,
}
```

向后兼容 DTO：

```rust
pub struct StageAssetCoverageCell {
    pub suggested_tools: Vec<String>,
    pub suggested_capabilities: Vec<StageCapabilitySuggestion>,
}

pub struct CoverageGapAction {
    pub suggested_tools: Vec<String>,
    pub suggested_capabilities: Vec<StageCapabilitySuggestion>,
}
```

`suggested_tools` 不再手写 match，而是由 `suggested_capabilities[].tools` 去重派生。

## 7. Stage capability registry

### 7.1 Scoping

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `scope.resolve_company` | none / subsidiary-gate precondition | `recon_lookup_company` | 纠正公司名，建立 root org |
| `scope.discover_subsidiaries` | `GOLISH-INTEL-SUBSIDIARY` when enabled | `recon_discover_subsidiaries` | 只查 OSINT/工商源，不碰目标主机 |
| `scope.confirm_scope` | none | `ask_human`, `manage_organizations` | 人审范围确认 |

Scoping 不做 scan wrapper。它是范围定义阶段，能力只服务 org tree 和 scope review。

### 7.2 Target Intel

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `intel.collect_passive_assets` | DNS / ASN / CT / SUBDOMAIN / OSINT | `recon_map_assets` | provider survey first，不能 fallback 到 scan CLI |
| `intel.collect_whois` | WHOIS | `recon_lookup_whois` | RDAP/WHOIS，一次 per org |
| `intel.record_terminal_gap` | all intel techniques | submit coverage terminal cell | provider missing/empty/error 时写 checked_empty/blocked/not_applicable |

`target_intel.allowed_tool_types=[]` 仍保持。能力可以调用 provider/registry 工具，但不开放 `pentest_run` 扫描工具。

### 7.3 External Attack Surface

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `eas.probe_http_liveness` | `GOLISH-EAS-LIVENESS` | `httpx` recipe / future wrapper | domain/url/vhost liveness，批量优先 |
| `eas.discover_ports` | `GOLISH-EAS-PORT` | `naabu`, `masscan`, `nmap` recipe / future wrapper | 只对 IP/CIDR host；domain/url 不进 PORT |
| `eas.fingerprint_services` | `GOLISH-EAS-SERVICE-FINGERPRINT` | `nmap -Pn -sV` recipe / future wrapper | 只对 confirmed open ports；WhatWeb 只用于 confirmed HTTP(S) |
| `eas.capture_web_screenshot` | supporting evidence | `gowitness` | optional，不能关闭 core coverage |
| `eas.record_terminal_gap` | EAS techniques | submit terminal coverage | DNS-only/no-open-port/unresolved 等确定性终态 |

第一批真正 wrapper 建议从 EAS 开始，因为 EAS 最容易因命令拼接不当导致：

- broad nmap sweep
- URL/domain 被当成 IP 扫
- SERVICE gap 被 WhatWeb 误导
- 批量 output-store 未落库时就 submit

### 7.4 Enumeration

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `enum.collect_browser_surface` | JS / JSAPI / PARAM | `browser_collect_js_api` | 动态浏览器采集 JS + runtime API + observed params |
| `enum.extract_js_apis` | JSAPI / PARAM | `js_extract_apis` | 静态 JS endpoint/param extraction |
| `enum.probe_routes` | DIR | `route_probe_paths` | 使用 DB seeds + built-in/local wordlist，不用外部 dir fuzzer |
| `enum.crawl_same_origin_urls` | supplemental API/DIR context | `katana` through `pentest_run` | supplement only，不替代 browser/route_probe |
| `enum.record_terminal_gap` | ENUM techniques | submit terminal coverage | no-js/no-api/no-param/blocked/not_applicable |

Enumeration 已经有 direct tools，第一阶段不急着再包 runner；先把 worklist capability 和 batch policy 统一，尤其是 `max_batch` 不应一律 50。`route_probe_paths` 这类长耗能力建议默认 `max_batch=10..20`，跑完 re-query。

### 7.5 Vuln Triage

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `vuln.scan_nday` | `GOLISH-NDAY` | `nuclei` / `searchsploit` bounded recipe | formulaic sweep only |
| `vuln.scan_config_exposure` | `WSTG-CONF-05` | `nuclei` mapped tags | 只认 handler upsert 的 technique_outcomes |
| `vuln.scan_default_creds` | `WSTG-ATHN-02` | nuclei/wpscan/nikto recipes | 高风险动作需受 approval/profile 限制 |
| `vuln.scan_tls` | `WSTG-CRYP-03` | TLS templates / nmap ssl scripts when allowed | 先做 objective classes |
| `vuln.record_terminal_gap` | WSTG techniques | terminal coverage | 无工具/不适用/阻断 |

这里必须先补齐 deterministic handler 写入 `technique_outcomes`，再扩大 `authoritative_found`。不要让模型自报 found 变成能力结果。

### 7.6 Attack Candidate

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `attack.synthesize_candidates` | `CANDIDATES` | reasoning + DB/evidence read only | no scan tools |
| `attack.rank_candidates` | `CANDIDATES` | pure scoring / RAG prior | 只产 hypothesis，不验证 |

`attack_candidate.allowed_tool_types=[]`。能力是 reasoning-only，不能调用 scan/exploit。

### 7.7 Verification

| Capability | Techniques | Implementation | 说明 |
|---|---|---|---|
| `verify.run_approved_poc` | candidate disposition | future wrapper / approved exploit tool | 必须绑定 candidate id + human approval |
| `verify.refute_candidate` | candidate disposition | safe probe / evidence read | 记录 refuted |
| `verify.mark_blocked` | candidate disposition | terminal note | WAF/auth/rate-limit/scope 阻断 |

Verification 不应先开放通用 exploit wrapper。第一版能力只做 candidate-bound contract，runner 需要另行设计。

## 8. 执行流

### 8.1 Metadata-only 阶段

```text
stage_asset_coverage_snapshot
  -> cell pending/error
  -> capabilities_for_technique(stage, technique)
  -> suggested_capabilities + suggested_tools

stage_worklist_next
  -> items[] include suggested_capabilities

stage_run specialist objective
  -> "choose capability from worklist"
  -> "tool names are implementation details"

stage_refiner
  -> coverage_gap_actions with capabilities
  -> RepairDirective actions mention capability first
```

AI 仍可能调用原始工具，但它收到的计划单位变成 capability。

### 8.2 Wrapper-tool 阶段

后续新增一个通用 executor tool：

```json
{
  "tool": "run_stage_capability",
  "args": {
    "capability_id": "eas.fingerprint_services",
    "work_item_ids": [
      "target_uuid:GOLISH-EAS-SERVICE-FINGERPRINT"
    ],
    "limit": 20
  }
}
```

后端执行：

1. 读取当前 active operation 的 stage/org/session。
2. 校验 capability 属于当前 stage。
3. 读取 `stage_worklist_next` / snapshot，校验 work item 存在且仍 pending/error。
4. 按 capability 的 asset/type precondition 过滤。
5. 生成固定 recipe 或调用 direct bridge。
6. await output-store / bridge evidence landing。
7. re-read coverage，返回哪些 cell terminal、哪些 blocked/not_applicable。

第一批 wrapper 只建议实现 EAS 三个能力：

- `eas.probe_http_liveness`
- `eas.discover_ports`
- `eas.fingerprint_services`

Enumeration direct tools 已经比较像 capability runner，可以晚一点再接入通用 `run_stage_capability`。

## 9. 安全约束

- `run_stage_capability` 不是 scan whitelist 的替代品；内部调用 `pentest_run` / direct tool 时仍必须经过 `stage_allows` 等价检查。
- wrapper 必须只接受当前 stage/org 的 work item，不接受任意 target string。
- wrapper 不允许模型传原始 shell args；只允许传 work item ids、limit、可选 safe mode。
- high/critical risk capability 必须检查 `HarnessAuthz` / profile approval。
- exploit/post-exploit capability 必须绑定 candidate id 或 approved scope item。
- capability runner 不能在 transaction 内执行外部命令。
- 所有 found/empty/error 结果必须由 handler 或 output-store 写 evidence / `technique_outcomes`；模型提交只能补 DB 无法派生的 terminal note。

## 10. UI 表达

Stage coverage UI 不应该把用户引导到 `nmap` / `httpx` 这种底层词，而应该显示：

- `Probe liveness`
- `Discover ports`
- `Fingerprint services`
- `Collect browser surface`
- `Extract JS APIs`
- `Probe routes`

hover/debug 才显示底层 tools：

```text
Fingerprint services
Tools: nmap
Closes: SERVICE
Only runs on confirmed open ports.
```

这能让用户理解“平台正在完成哪个阶段能力”，而不是只看到一串工具名。

## 11. 兼容策略

| 旧字段 | 新字段 | 兼容策略 |
|---|---|---|
| `suggested_tools` | `suggested_capabilities[].tools` | 保留旧字段，派生生成 |
| `CoverageGapAction.suggested_tools` | `suggested_capabilities` | 保留旧字段，sub-agent 旧 parser 不破 |
| `RepairAction.tool` | `capability_id` + `tool` | 第一阶段同时填 |
| objective concrete tools | capability list + tool details | objective 仍列 concrete tool，但从 capability 派生 |

不要一次性删旧字段，否则前端 generated types、sub-agent repair mode、旧 transcript replay 都会被打断。

## 12. 验证策略

纯 capability registry：

```bash
cd backend && cargo nextest run -p golish-agent-kit stage_capability tool_taxonomy --status-level fail
```

coverage/worklist/refiner：

```bash
cd backend && cargo nextest run -p golish-agent-app stage_coverage --status-level fail
cd backend && cargo nextest run -p golish-agent-kit stage_refiner coverage_gap worklist --status-level fail
```

runtime objective / sub-agent prompt：

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run_call --status-level fail
cd backend && cargo nextest run -p golish-sub-agents defaults --status-level fail
```

UI：

```bash
pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx
pnpm exec tsc --noEmit --pretty false
```

wrapper runner live smoke：

```bash
python3 scripts/run_tree.py --workspace /Users/christopherzheng/golish-platform/Test1 --db --full
```

期望：

- worklist 返回 `suggested_capabilities`
- AI 选择 capability，而不是自由拼 broad command
- wrapper 执行后对应 business table / `technique_outcomes` / evidence rows 落库
- `stage_worklist_status.ready_to_submit` 从 false 收敛到 true
- submit 不要求模型手写 evidence ids

## 13. 迁移顺序

1. 写 capability registry 和纯单测。
2. 给 `StageAssetCoverageCell` / worklist / gap action 增加 `suggested_capabilities`。
3. 用 capability registry 替换散落的 `suggested_tools` match。
4. 更新 stage_run objective、stage methodologies、sub-agent repair instruction。
5. UI 优先展示 capability label。
6. 实现 EAS wrapper runner。
7. 评估 Enumeration 是否接入 `run_stage_capability`。
8. 再进入 Vuln/Verification wrapper 设计。

## 14. Open questions

- 通用工具名用 `run_stage_capability` 还是按阶段拆成 `eas_fingerprint_services` / `enum_probe_routes`？建议先通用工具，内部 allowlist 限 capability id，减少 function declaration 数量。
- `CapabilitySpec` 是否需要从 JSON 读？第一版建议 Rust 常量，避免 runtime 配置漂移；后续可导出 JSON 给 UI。
- `max_batch` 是否按 capability 固定，还是根据 live host / timeout 动态调节？第一版固定，后续由 runtime supervisor 调整。
- EAS `httpx` server header 是否足够算 SERVICE-FINGERPRINT？现有设计仍建议 SERVICE 以 fingerprints / nmap -sV / WhatWeb HTTP tech 为准，能力层不改变 gate 标准。
- Verification wrapper 的 approval/candidate binding 需要单独设计，不能跟 EAS wrapper 同批实现。
