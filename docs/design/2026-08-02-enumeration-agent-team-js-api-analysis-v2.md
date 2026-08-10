# Enumeration Agent Team 与 JS/API/参数证据解析 v2

**日期：** 2026-08-02
**状态：** 提案，等待排期与 schema 授权
**范围：** Pentest / Red Team 的 Enumeration 阶段；不改变 Recon、Application Understanding、Vulnerability 或 Verification 的阶段职责

## 1. 结论

Enumeration 不再把 JS/API 发现视为一个“调用一次、等待汇总结果”的黑盒工具，而是在现有一次 `stage_run(enumeration)` 内运行一个受控 Agent Team：

```text
Primary Task Agent
└─ stage_run(enumeration)                         前端仍只发起一次
   └─ Company Controller                         每家公司一个
      ├─ Content / DIR Worker                    exact-origin 分片
      ├─ Browser Runtime Worker                  只读加载与真实流量观察
      ├─ JS / API Analysis Worker                AST、模块与 client 上下文
      ├─ Parameter Analysis Worker               occurrence 级参数归并
      ├─ Resolution Analyst                      仅处理 unresolved cluster
      └─ Coverage Reviewer                       只读复核，不能替 Gate 放行
```

关键约束是：

> 每个 exact origin、页面、JS 和 endpoint candidate 都有独立状态与证据，但不为每个 URL 无限制启动一个模型 Agent。

服务端从权威 Enumeration worklist 生成有类型的、互不重叠的 shard；Worker 在固定并发窗口内滚动领取。爬虫、Playwright、AST 与确定性 data-flow 是主路径；AI 只分析无法确定的动态拼接、跨文件 wrapper、混淆代码或证据冲突，而且每个 AI 结论必须引用源码、配置或 runtime capture。

数据层保留 `api_endpoints` 作为 canonical identity，保留现有 Enumeration manifest 作为 Gate 汇总；新增 operation-scoped occurrence、occurrence 参数与 occurrence-evidence 关系。一条 canonical endpoint 可以有多条不可互相覆盖的发现记录，未解析或 scope 外候选也能落库而不会被错误提升为可执行接口。

## 2. 与既有设计的关系

本设计扩展而不覆盖以下现行设计：

- `2026-06-25-enumeration-js-api-collection.md`：浏览器与 JS/API 采集基础。
- `2026-07-14-js-api-contextual-resolution.md`：同文件 axios client/baseURL 解析 v1。
- `2026-07-17-enumeration-surface-manifest-vuln-applicability.md`：exact-origin manifest 与 endpoint/parameter 汇总。
- `2026-07-27-enumeration-worker-output-convergence.md`：Worker 输出回流和 Controller/Gate 权威边界。
- `2026-08-01-pentest-intake-and-enumeration-origin-convergence.md`：可信 URL intake 与 exact-origin 收敛。
- `2026-08-02-unified-visible-stage-workspace-mock.md`：统一可见 Agent Workspace 的产品方向。共享树已在本设计审计期间完成基于现有 `StageTeamRunView` / `StageTeamWorkspaceView` 的生产统一详情；v2 直接为该 shell 提供 typed artifact read model，不再创建平行的 `StageWorkspace` 组件树。

`2026-07-14` 的 v1 仍是兼容路径：它解决同一文件中的简单 axios client，但明确没有覆盖跨文件 import/export、document base、source map、wrapper data-flow 或多来源持久化。v2 在其上增加 source/module/runtime context，不将旧历史 operation 重新解释为 v2。

## 3. 当前状态与问题

### 3.1 编排仍是自由文本全功能 Enumerator

当前已有 Company Controller、动态 worker、durable Stage Team 与滚动并发外壳，但：

- Scheduler 只 seed `leader:primary`，没有 server-owned Enumeration shard。
- Controller 与 Enumerator 最终映射到同一个 frozen specialist。
- Enumerator 同时拥有 preflight、crawler、browser、extractor、route probe、worklist 和 submit 能力。
- 子任务主要由模型用自由文本 objective 分配，不能确定性证明 exact-origin/axis 无重叠。
- 权威 worklist 已经是 `exact origin × DIR/JS/JSAPI/PARAM`，但 worker assignment 没有使用这份结构化事实。

因此“后台有并发”不等于真正受控的多 Agent 分析。

### 3.2 浏览器采集了丰富事实，投影时大量丢失

Node/Playwright helper 已能安全阻止跨 origin 和写请求，并收集 request body/header、response、脚本、chunk 与 manifest。但当前投影存在以下损失：

- runtime request 以 `method + URL` 去重，同一接口在不同页面、不同 body shape 或不同 initiator 的调用会合并。
- manifest 已有 script URL、canonical URL、发现方式和重复关系，Rust loader 只保留本地 `script_paths`。
- Rust API endpoint 投影稳定提取 query 名；JSON body、form、path 与 request header 字段没有形成 occurrence 级事实。
- capture v2 会写原始 request header/body 和 response sample，不适合作为普通 endpoint provenance 的长期形态。
- 静态 placeholder 升级时会丢失部分 `source_url`/执行上下文。

### 3.3 静态分析仍是 regex-first + AST range filter

`golish-js-analyzer` 当前用正则生成命中，再用 ast-grep call range 做真假过滤。Candidate 保存 method/path/source/callee/receiver/span，但没有：

- 调用参数对象与参数位置；
- source URL、document URL 或模块执行上下文；
- import/export 与跨文件 symbol graph；
- request wrapper 的数据流；
- `new URL`、relative base、HTML `<base>`、framework/bundler base；
- source map 对压缩 bundle 到源码位置的映射。

现有 free-form `param_hints` 按 `(path, method)` 归并，存在把相邻 callsite 的参数贴到错误接口上的风险。

### 3.4 “主路径”不是公司级字段

以下三段代码遵循不同 URL 语义：

```js
axios.create({ baseURL: "/a" }).get("/b/xxx") // /a/b/xxx
fetch("/b/xxx")                               // origin-root /b/xxx
fetch("b/xxx")                                // 相对 document/base URL
```

一个 origin 可以同时承载 `/a`、`/admin`、`/api/v1` 等多个 application/client context。给公司或 origin 只存一个“主路径”会制造错误确定性。

### 3.5 Canonical row 不能表达多来源

当前至少有三层发生折叠：

- `api_endpoints` 对 `(target_id, url, method)` 唯一，只能表达 canonical identity。
- `enumeration_endpoint_observations` 对 operation/origin/endpoint 唯一，更新时覆盖单个 `source`。
- `enumeration_endpoint_parameters` 对 observation/location/name 唯一，同样覆盖单个 `source`。

浏览器、两个 JS callsite、HTML form 和 AI anchored inference 最终可能只剩一条来源。`js_analysis_results.raw_analysis.contextual_resolution_v1` 虽保存 bounded occurrence JSON，但它按 filename 可变覆盖、容量受限，也不是可查询的权威关系。

### 3.6 UI 只看到汇总，不知道“在哪里发现”

Target Surface 主要读取 target-global legacy rows并进行 path/method 启发式匹配；Stage Team read model 只有 output count、evidence ID 和 hash。结果是：

- Endpoint 详情只能显示 method/path/params/source 汇总。
- evidence 数量部分来自 heuristic，并非 occurrence-evidence 关系。
- Worker badge 无法跳转到它实际发现的页面、JS、capture 或 endpoint。
- Enumeration 运行中没有专用、持续更新的 origin/script/candidate/parameter 进度。

## 4. 目标与非目标

### 4.1 目标

1. 顶层仍只启动一次 Enumeration，后台改为 server-owned、有限并发、可观测的 Agent Team。
2. 每个 exact origin 的 `DIR / JS / JSAPI / PARAM` 有确定 producer、依赖和终态。
3. 每个 endpoint occurrence 保留发现位置、原始表达式、解析链、参数事实和 evidence anchor。
4. 正确区分 runtime observed、static confirmed、AI inferred、ambiguous、unresolved 与 scope excluded。
5. 未解析候选可持久化、可交接，但不能进入 canonical executable endpoint。
6. 浏览器采集只读、字段值脱敏，普通 provenance 不保存 token/cookie/password 原值。
7. Gate 根据 DB 中的确定性闭包判断覆盖，而不是相信 Agent 自报完成。
8. 前端可以从 Worker → occurrence → endpoint/parameter/evidence 双向追踪。

### 4.2 非目标

- 不新增独立“JS Analysis”顶层阶段；它仍属于 Enumeration。
- 不主动重放 POST/PUT/PATCH/DELETE，不提交表单，不做漏洞验证。
- 不让每个 URL 独占模型 Agent。
- 不让 AI 输出直接成为可执行 endpoint 或 Gate truth。
- 不在 v2 首批实现完整通用 JavaScript 解释器、任意混淆还原或全语言 source-map debugger。
- 不回填或重写历史 operation；历史数据继续按 legacy/v1 读取。
- 不废弃 `api_endpoints`、现有 manifest 或四轴 Gate。

## 5. 安全边界

### 5.1 网络行为

- 页面和静态资源只允许当前授权 scope 内的 GET/HEAD 正常加载。
- 浏览器可观察页面自身准备发出的写请求，但必须在离开进程前 abort，记录 `sent=false`。
- 不重放 runtime capture，不猜测写接口参数，不提交 HTML form。
- 新发现跨 origin URL 先写 occurrence：只有匹配同一 frozen scope 中 EAS-confirmed exact origin 才可加入 worklist；否则标记 `scope_excluded`，不访问。

### 5.2 数据最小化

capture v3 的普通 endpoint provenance 只保存：

- header 名称、敏感性标签和必要的类型；
- query/body/form/path/GraphQL variable 的字段名、位置、推断类型；
- 仅对“字段名 + 类型”的归一化 schema 计算 shape hash，并保存原 body 长度；不对包含值的原 body 做可被字典反推的 hash；
- bounded response metadata，不把完整响应复制到 occurrence。

普通 DB/manifest URL 统一保存 value-free 形态：移除 userinfo/fragment，保留 origin + normalized path，query 只保留排序后的参数名或 `{value}` display template。`canonical_request_url` 固定为 origin + normalized path，不含 query/fragment，因此不同 query value 不会拆成多个 endpoint；query names 只进入 parameter facts。page/document/script URL 同样消毒；完整 URL、原 request/response 或 source-map body 若确有保留需要，只能留在受控 capture/raw-witness artifact，并且不进入普通 artifact IPC。

`authorization`、`cookie`、CSRF token、password、secret、session 和 API key 的值必须在 Node helper 内脱敏，不能依赖 UI 隐藏。

### 5.3 权威与 IDOR

- 所有 occurrence 写入必须同时验证 operation、project、organization、source target、source origin、stage execution/unit/worker lease。
- resolved origin 若映射到同 scope 的另一个 target，必须按 UUID 稳定顺序锁 source/resolved guards，再在同一 DB transaction 内投影；事务中禁止网络或模型调用。
- 前端 artifact API 只解析 allowlisted typed fact refs，不能透传任意 `canonical_output` JSON。
- Controller 独占 final submit；Coverage Reviewer 只能读，不能写 Gate 终态或提交 deliverable。

## 6. 受控 Agent Team

### 6.1 Server-owned shard

新增 typed assignment：

```rust
struct EnumerationWorklistShard {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    organization_id: Uuid,
    target_id: Uuid,
    web_origin_id: Uuid,
    canonical_origin: String,
    producer: EnumerationProducer,
    axes: Vec<EnumerationAxis>,
    dependency_receipts: Vec<Uuid>,
    attempt: u16,
    generation: u16,
}
```

`objective` 可以作为展示文本，但不能决定 authority。Scheduler 只从 DB worklist 生成 shard；同一 generation 内，一个 terminal producer cell 只能有一个有效 owner lease。

### 6.2 Worker 角色与工具边界

| 角色/节点 | 执行形态 | 输入 | 允许的主要工具 | 产出 | 禁止 |
|---|---|---|---|---|---|
| Content / DIR | host deterministic worker | exact origin | preflight、crawler、route probe | page/route/static frontier、DIR outcome | provider、browser/extractor、final submit |
| Browser Runtime | host deterministic worker | exact origin + seed pages | browser collector | script manifest、runtime occurrence、capture v3、JS prerequisite | provider、extractor AI、route probe、final submit |
| JS/API Analysis | host deterministic worker | script manifest | deterministic analyzer/extractor | static occurrence、resolution disposition、JSAPI producer outcome | provider、free network、one-shot AI、final submit |
| Parameter Analysis | host deterministic worker | runtime/static occurrences | parameter reducer | occurrence parameter facts、PARAM aggregate | provider、网络重放、凭据值读取 |
| Resolution Analyst | LLM SubAgent | bounded unresolved cluster | source/capture/config read-only tools | anchored inference 或 unresolved receipt | 任意文件/网络、canonical promotion |
| Coverage Reviewer | host deterministic worker | company manifest snapshot | bounded read model | missing/ambiguous/exhausted review | provider、改 outcome、final submit |

工具 mask 必须由 host/runtime 强制，不能只写在 prompt 中。

### 6.3 执行波次

```text
Wave 0  Preflight exact origins
Wave 1  Content/DIR || Browser Runtime              可并行
Wave 2  JS/API Analysis                            等 Browser manifest
Wave 3  Parameter Analysis                         消费 runtime + static occurrences
Wave 4  Resolution Analyst（条件式）               只领 unresolved cluster
Wave 5  Coverage Reviewer → Controller final Gate  只读复核后由 Controller 提交
```

新页面或动态 chunk 可以增量加入 frontier，形成下一 wave；Scheduler 保持 bounded rolling window，而不是一次创建全部 worker。首版冻结 `max_company_units_active=2`、每公司 `max_workers=3`、全局 deterministic jobs `=6`、全局 browser jobs `=2`、provider calls `=4`、每公司 dynamic requests `=8`。既有 Company Controller 与按需 Resolution Analyst 消耗模型额度，其他 lane 以 `model=None/provider=None` 运行；耗尽必须落 receipt。

### 6.4 失败与恢复

- Worker lease、generation、attempt 和 shard key 都持久化；断线后从 DB 恢复，不靠聊天记忆。
- 重试只能重新领取同一 typed shard，不能改写 origin/producer scope。
- 同一 occurrence 的 stable key 使重试幂等；不同 callsite/capture occurrence 不因 canonical URL 相同而合并。
- budget exhausted、unsupported syntax、missing source map 都是显式终态，不等价于 checked-empty。

## 7. 确定性采集与分析

### 7.1 Browser capture v3

Node helper 输出 versioned manifest：

```json
{
  "schema_version": "browser_js_api_capture_v3",
  "page_url_template": "https://x.com/a/index.html",
  "document_base_url": "https://x.com/a/",
  "scripts": [{
    "source_url_template": "https://x.com/assets/app.js?v={value}",
    "content_sha256": "...",
    "discovery_kind": "script_tag",
    "discovered_from": ["https://x.com/a/index.html"]
  }],
  "requests": [{
    "capture_id": "...",
    "method": "POST",
    "url_template": "https://x.com/a/b/xxx?page={value}",
    "sent": false,
    "initiator": {"script_url": "...", "line": 438},
    "parameter_facts": [
      {"name": "page", "location": "query", "value_type": "number"},
      {"name": "role", "location": "body", "value_type": "string"}
    ]
  }]
}
```

重复请求以 server-issued logical occurrence key 保存，多次调用可以共享 canonical endpoint，但不能互相覆盖。原始 capture event ID 只是 provenance，不参与 retry idempotency；同一 page navigation 内相同 event fingerprint 用确定性 ordinal 区分。v1/v2 reader 保留，新写入只生成 v3。

`initiator` 不能从 Playwright 的 `page.on("request")` 猜测，因为该事件本身不提供可靠的调用脚本与源码位置。Chromium 路径必须为每个 page/context 建立 CDP session，监听 `Network.requestWillBeSent`，以 CDP `requestId` 保存原始网络事件，并按同一 page、method、sanitized URL、CDP monotonic timestamp 与确定性 ordinal 关联 capture occurrence；script URL、line、column 和 stack 只来自 `initiator` payload。CDP 不可用或事件无法唯一关联时，`initiator=null` 且保存 `initiator_status=unsupported_cdp|unmatched`，不得拿当前 script tag、document URL 或相邻调用猜行号。unsafe request 仍由 Playwright route 在 dispatch 前阻断；`sent=false` 是安全事实，不由 CDP 事件是否出现反推。

### 7.2 AST 与 module/data-flow

分析优先级：

1. AST-confirmed call 与 argument/config object 提取；
2. 同模块常量、client factory、alias 与 defaults；
3. import/export symbol graph 和 request wrapper 的 bounded fixed-point；
4. document/framework/bundler/runtime config；
5. source map 映射；
6. regex 仅作为候选兜底，不能绕过 AST/disposition 校验。

首版覆盖 fetch/Request、axios、XHR、jQuery、GraphQL client、WebSocket、EventSource；扩展框架 adapter 时必须输出同一 typed candidate contract。

source map 的普通持久化只允许 map sanitized URL、content hash、`sources` 路径、generated span、可验证的 original span，以及 bounded redacted source window/hash。`sourcesContent` 与完整 map/body 只能存在受保护的 capture/raw-witness artifact，禁止写入普通 DB descriptor、manifest 或 IPC。无法验证映射时保留 bundle span 与 `unsupported_mapping`，不能把猜测位置显示成源码行。

### 7.3 Parameter fact

```rust
struct ParameterFact {
    name: String,
    location: ParameterLocation,
    value_type: Option<String>,
    requirement: ParameterRequirement,
    source_span: Option<SourceSpan>,
    confidence: ObservationConfidence,
}
```

`ParameterRequirement` 是 `required / optional / unknown` 三态；观察到字段存在不能自动证明它是 required。投影到 legacy boolean 时只有 `required` 写 `true`，`optional` 与 `unknown` 都写 `false`，但 v2 occurrence detail 必须保留二者差异。

位置至少包括 `path`、`query`、`body`、`form`、`header`、`graphql_variable`、`unknown`。旧 `body_or_form` 继续可读，v2 producer 不再用它表达已能区分的事实。事实必须绑定 candidate/callsite/capture，禁止只按 `(path, method)` 全局贴 hint。

浏览器看到具体 `/users/42` 时不能凭空知道 `42` 的参数名；`path` 参数只能来自明确的 template/route pattern、静态 callsite 或 runtime framework metadata，并与对应 occurrence 关联。

## 8. URL Resolution Context

### 8.1 上下文粒度

解析上下文属于 occurrence。每条 base fact 除 value/source 外必须带 `applies_to`：`http_request / document_relative / module_asset / router_navigation`，禁止跨类别套用。来源可以是：

- document URL 与 HTML `<base href>`；
- application/router mount path（只影响明确绑定的 router navigation）；
- HTTP client `baseURL`；
- Vite/Webpack/Next/Nuxt/Angular runtime 或 bundler config（asset/public path 默认只影响 module asset）；
- module URL、`import.meta.url` 与 source map；
- 浏览器真实 observed URL。

不创建 company-level “main path”。

### 8.2 优先级

```text
runtime observed URL
  > 明确绑定到该 callsite 的 HTTP client base
  > 与本调用 `applies_to` 匹配的 document/framework/bundler context
  > 跨文件 deterministic symbol/data-flow
  > 有源码/capture/config anchor 的 AI inference
```

低优先级证据不能覆盖更高优先级；冲突必须保留多个 candidate URL 并标 `ambiguous`。`fetch("/api")` 不受 router basename、assetPrefix 或 Webpack public path 影响；只有 chunk/module URL 才消费 bundler public path。

### 8.3 正交状态与派生展示标签

数据库不把来源可信度、URL 解析、scope 和噪声判断塞进一个 `status`。每条 occurrence 分别保存：

- `observation_kind`：`runtime_request / html_form / static_ast / ai_analysis`；
- `inference_level`：`observed / deterministic / ai_inferred`；
- `resolution_status`：`resolved / ambiguous / unresolved / not_applicable`；
- `scope_decision`：`in_scope / scope_excluded`；
- `candidate_classification`：`endpoint / noise`；
- `promotion_eligibility`：由上述字段和 frozen contract 确定性派生，不能由 Agent 自报。

前端为兼容用户心智派生 `runtime_observed`、`static_confirmed`、`ai_inferred`、`ambiguous`、`unresolved`、`scope_excluded`、`noise_excluded` 标签。例如 `static_ast + deterministic + resolved + scope_excluded` 能同时保留“静态确认”和“scope 外”，不会因单列 enum 丢失维度。

只有 `in_scope + endpoint + resolved + (observed|deterministic)` 且 contract=`agent_team_v2` 可 canonical promotion。AI inference 经后续 runtime 或 deterministic evidence 证实后，新增一条更强 child occurrence；不原地伪装成 runtime observed。

## 9. 持久化模型

### 9.1 保留的汇总层

- `api_endpoints`：target-scoped canonical `(method, resolved URL)` identity，供既有功能兼容。
- `enumeration_endpoint_observations`：operation/exact-origin/canonical endpoint 的 Gate 汇总。
- `enumeration_endpoint_parameters`：canonical parameter aggregate，供后续安全查询与兼容 UI；不再承担完整 provenance。

### 9.2 权威依赖与新增 operation-scoped 层

本设计不再新造一套 denominator/evidence authority。它硬依赖 `tool-truth-coverage-contract-2026-07-29` 已 passing，并复用其 `tool_truth_execution_authorities`、sealed `coverage_denominators/items`、`capability_execution_receipts/inputs`、`tool_truth_evidence_authorities` 与 business-ref authority。该依赖未完成时，本功能保持 `not_started/blocked`。

新增唯一 migration `20260802000003_enumeration_endpoint_provenance_v2.sql`，执行前必须重新确认 timestamp 未占用并取得用户明确 schema 授权。它增加：

- `enumeration_analysis_rollout`：server-owned deployment selector，默认 `legacy_v1`，只能通过带 expected version、review report hash 与审计 receipt 的受控 promotion 函数前进；
- `operation_state.enumeration_analysis_contract`：`legacy_v1 / agent_team_v2_shadow / agent_team_v2`，在 operation INSERT 同一事务由 deployment selector 冻结，之后不可修改；stage reset/purge 不删除、不重冻；
- Tool Truth business-ref kind 扩展 `enumeration_endpoint_occurrence` 与 `enumeration_endpoint_group`，继续走既有 snapshot/hash/authority validation。

Denominator 层级固定为：

```text
root exact-origin × axis denominator
  └─ derived script/runtime batch denominator
     └─ derived endpoint candidate denominator
        └─ parameter assessment denominator
```

每层先写 `coverage_denominator_items` 并 seal member set/hash，再执行 capability receipt。Gate 比较 sealed member set 与 terminal receipt inputs/occurrences；候选被静默漏掉时一定 BLOCK。

`enumeration_js_analysis_items` 是 Tool Truth item 的 domain descriptor：

- 引用 `(denominator_item_id, execution_authority_id, terminal_receipt_input_id)`；
- 保存 sanitized manifest/capture、page/document base、chunk/source-map metadata；
- 同一内容在两个 document/client context 中是两个 denominator item，不能仅按 hash 合并；
- `analyzed_found/analyzed_empty/skipped/exhausted` 从 terminal receipt input 派生，不再维护第二套可漂移 status。
- descriptor 只允许一次 compare-and-set 绑定 terminal receipt input；第二次 terminal transition、UPDATE 或 DELETE 均由 DB 拒绝。

`enumeration_endpoint_candidate_inputs` 是 sealed candidate denominator member 的 descriptor：

- server-issued logical input key 跨 retry 稳定；source artifact/hash、callsite/event fingerprint 与 deterministic duplicate ordinal 是 member 内容；原 capture event ID 仅作 provenance；
- 保存 sanitized raw expression/method/protocol/source span 与 resolution input；
- 每个 member 必须对应 terminal `capability_execution_receipt_inputs`，随后产生至少一条 terminal occurrence；无 occurrence/receipt 的 member 对 Gate 可见。

`enumeration_endpoint_occurrences`：

- authority：candidate input、receipt input、execution authority、operation/project/organization、source target/origin、stage execution/unit/worker；
- lineage：可空同 authority `parent_occurrence_id`，用于 AI resolution 或后续 corroboration 新增 child，原 row 不改；
- source：sanitized page/document/JS URL、JS hash、source span、CDP initiator、capture event ID；
- expression：protocol、method、sanitized raw expression、receiver/client、可空 GraphQL operation/WebSocket subprotocol；
- resolution：带 `applies_to` 的 base facts、candidate URLs、typed chain、sanitized display URL、无 query 的 canonical request URL、route kind/template；
- orthogonal outcome：observation/inference/resolution/scope/classification 维度与确定性派生 promotion eligibility；
- cross-origin：显式 `source_target/origin` 与可空 `resolved_target/origin`；discovery evidence 绑定 source，canonical/group projection 绑定 frozen EAS-confirmed resolved origin；
- safety：request sent、value-free schema hash/length/redaction metadata；不含 secret value。

`enumeration_endpoint_parameter_assessments`：

- 引用 parameter denominator item + terminal receipt input + occurrence；
- outcome 为 `found / checked_empty / unresolved / not_applicable`；缺行与 `checked_empty` 严格不同；
- `required / optional / unknown` 三态保留，legacy bool 仅 required=true。
- occurrence 与 assessment 都是 immutable terminal truth；UPDATE/DELETE 必须由 DB trigger 拒绝，而不是只依赖 repo 约定。

`enumeration_endpoint_occurrence_parameters` 以 assessment 为父，保存 name/location/type/requirement/confidence/source anchor，不含 value。

`enumeration_endpoint_groups` 是 operation-scoped canonical identity：key 为 resolved origin + protocol + method + normalized route template + 可空 GraphQL operation。它区分 `resolved_exact`、`resolved_route_template` 和 arbitrary dynamic unresolved；runtime concrete URL 只是 observation sample，不决定 group identity。

- `/users/${id}` 只有 AST 能产生稳定 segment template 时归一成 `/users/{id}`；任意拼接仍 unresolved；
- exact runtime `/users/42` 只有唯一命中一个 template 时才能 link，两个冲突 template 保持 ambiguous；
- WebSocket 只进 v2 group，不投影 HTTP `api_endpoints`；GraphQL operation 在 v2 group 中独立；
- legacy `api_endpoints` 只接兼容的 HTTP(S) exact endpoint，或已有 runtime concrete sample 的 template group；模板本身永不作为可自动 replay URL。Verification 只能在独立授权后使用 observed concrete sample。

`enumeration_endpoint_occurrence_group_links` 与 `enumeration_endpoint_group_api_links` 分开保存 occurrence→v2 group 及兼容 group→`api_endpoints`/manifest projection；Shadow、scope/noise/unresolved 禁止 legacy projection。

`enumeration_endpoint_occurrence_evidence` 不引用裸 `audit_log.id`，而引用 `(tool_truth_evidence_authority_id, execution_authority_id, authority_hash)`，role 为 discovery/resolution/parameter。由既有 authority 冻结 audit role、in-scope classification、validity、producer envelope、stage/worker fence 和 hash；跨 authority 重绑被 composite FK 拒绝。

### 9.3 Guarded persistence 与延迟 canonical projection

Producer 与 Parameter lane 先写不可变事实；等一个 origin 的 discovery/resolution/parameter 输入稳定后，再由确定性 canonicalizer 投影：

```text
validate authority + frozen scope
  → require compatible Tool Truth execution authority/contract
  → seal script/runtime denominator and domain descriptors
  → execute receipt; seal derived candidate denominator/members
  → for every candidate receipt input insert immutable terminal occurrence
  → bind normalized evidence authorities/business refs
  → Parameter lane seal denominator, receipt, immutable assessment + parameters
  → if enumeration contract = agent_team_v2:
       group promotable occurrences by protocol/origin/method/route/operation
       insert immutable occurrence↔v2-group links
       project only compatible groups to api_endpoints + manifest
       reduce canonical parameters from linked terminal assessments
  → publish producer outcome/evidence reference
```

v2 对 manifest 的单值 `source` 固定写中性的 `occurrence_v2_aggregate`，不再用最后一个 producer 覆盖它；真实来源只从 occurrence 关系读取。
`legacy_v1` 拒绝所有 v2 writer。`agent_team_v2_shadow` 只能写 sealed denominator/domain descriptors、occurrence、assessment、参数、normalized authority refs 与 comparison read model，不创建 group/api links，不创建或更新 `api_endpoints`、manifest、四轴 outcome 或 Gate truth。`agent_team_v2` 还要求 operation 的 Tool Truth contract=`receipt_v1` 才能投影。

事务只做 DB 操作。网络、浏览器、JS 分析和模型推理必须在事务外完成。

### 9.4 历史兼容与 rollout

- 历史 operation 继续读取 legacy/v1 projection，不做 silent backfill。
- 新 operation 的 `operation_state.enumeration_analysis_contract` 在 INSERT 时从 server-owned rollout 冻结；reset/purge 保留该值，任何默认值变化都不影响已存在 operation。
- Shadow 期复用同一次 legacy/browser capture，把 v3 manifest tee 给 v2 deterministic analyzer；不得为同一页面再发第二轮网络请求。它可写 occurrence 并对比覆盖率，但不改变现有 Gate producer ownership 或 canonical projection。
- 只有 focused fixture 报告经评审后，才对新 operation 切 `agent_team_v2`；不重解释运行中的 operation。

## 10. Gate 与覆盖闭包

四轴保留，producer ownership 明确为：

- `DIR`：route/content producer；
- `JS`：browser capture producer；
- `JSAPI`：deterministic extractor/resolution reducer；
- `PARAM`：parameter reducer。

Gate 额外读取 deterministic candidate lifecycle closure：

```text
每个 required exact origin
  ├─ DIR 有 producer terminal outcome
  ├─ sealed script denominator 每个 member 有 terminal receipt input
  ├─ sealed candidate denominator 每个 member 有 terminal receipt + occurrence
  ├─ 每个 occurrence 的正交 outcome 可派生确定终态
  ├─ 每个 promotable group/occurrence 有 terminal PARAM assessment
  └─ unresolved/ambiguous 有 bounded attempted/exhausted receipt 与 handoff
```

派生 `unresolved` 可以是阶段终态，但必须有 normalized evidence authority、原因和尝试预算；它不算 checked-empty，也不算 confirmed endpoint。AI 只能提交 inferred/ambiguous/unresolved suggestion，不能把候选终态化为 deterministic noise；`noise` 只有 closed validator 可派生。Coverage Reviewer 只找缺口，最终 PASS/BLOCK 仍由 DB Gate 决定。

ReceiptV1 下 preflight failure 只关闭 prerequisite，不得伪造四轴 checked-empty。methodology、prompt、StageRefiner 和 capability ownership 必须同步修正现有语义漂移。

## 11. 前端与可观测性

### 11.1 Enumeration 实时进度

新 typed read model 按 operation/stage execution 返回 bounded 统计：

```text
Origins              4 / 5 terminal
Pages                 126 discovered
Scripts               73 / 80 analyzed
Runtime occurrences   18
Static occurrences    41
Confirmed endpoints   47
AI inferred            8
Ambiguous/unresolved   3
Parameters            43 / 47 assessed
```

非终态 Enumeration 每 1.5 秒同时 polling Stage Team read model 与 Enumeration artifact page；两条流共享 operation/execution identity、abort controller 与递增 sequence guard，任何一条旧 response 都不能覆盖新 operation。stage terminal 后两条流都停止；loading/error/empty 独立呈现。

### 11.2 Worker → artifact

Stage Team read model 为每个 worker 返回显式 `execution_kind=host_deterministic|llm_subagent`；provider/model 在 deterministic worker 上为 null，但 UI 不能用“字段为空”反推执行类型。Stage Team artifact API 接受 operation、stage execution、可选 unit/worker cursor，只返回 allowlisted occurrence summary 和 typed fact refs。`StageTeamWorkspaceView` 的 worker 选择改成 controlled `selectedWorkerRunId/onSelectionChange` 合同，父级用同一 selection 过滤 artifact；Controller、host worker 和未选择三种状态都必须有确定行为。点选 Worker 后展示它分析的 origin、页面、JS、接口与 evidence，而不是 inert evidence ID。

### 11.3 Endpoint 详情

Target Surface 不得用“最近一次 operation”暗选 provenance。operation-scoped 查询必须显式带 `operationId`，并在服务端把 operation、project、organization、target 与 frozen scope 绑定后才返回；首版提供两条 bounded cursor API：

- endpoint provenance：`targetId + endpointId + operationId + cursor + limit`；
- 未晋升 candidate/occurrence：`targetId + operationId + optional webOrigin/outcome filters + cursor + limit`。

两者默认 limit=50、服务端 clamp 1–100，按 `(observed_at,id)` 稳定排序。若未来需要跨 operation 的全局 Target Surface，必须新增显式授权 operation-list/read model，不能在本 API 内隐式扩大范围。

Target Surface 的 endpoint 详情与 candidate residual 视图展示：

- canonical method/URL；
- 所有 occurrence；
- page、JS URL/hash、行列、runtime initiator/capture；
- raw expression → base/context → candidate URLs → selected URL；
- disposition/confidence/reason；
- 参数位置、类型、来源和脱敏状态；
- scope-excluded 与 unresolved 单独分组，不混入可执行接口。

UI 不再用 path/method heuristic 猜 JS 来源或 evidence count。

## 12. 失败模式与确定性处理

| 失败模式 | 处理 |
|---|---|
| 同一 JS 内容由两个 URL/页面加载 | 一个 content hash，多条 source/execution context；occurrence 分开 |
| 同 method/URL 不同 body shape | 多条 runtime occurrence，canonical endpoint 汇总参数并保留来源 |
| axios client base 冲突 | `ambiguous`，保存全部候选，不取第一个 global base |
| relative fetch 无 document base | `unresolved`，不按 origin-root 猜测 |
| 跨 origin candidate | scope 内 EAS-confirmed 才进入队列，否则 `scope_excluded` |
| AI 无 evidence anchor | 拒绝写 `ai_inferred`，产出 invalid-analysis receipt |
| AI 认为候选是噪声 | 只记录 `ai_noise_suspected` suggestion；只有 closed deterministic validator 能终态化 `noise` |
| source map 缺失 | bundle span 可作为 anchor；状态可 `unresolved`/`static_confirmed`，不能虚构源码位置 |
| CDP 不可用或 initiator 无法唯一关联 | `initiator=null` + 明确 unsupported/unmatched reason，不猜 script/line |
| Worker retry | stable occurrence key 幂等；不同 callsite/capture 不合并 |
| 旧 capture v1/v2 | 兼容读；新写只用 v3；缺字段显式 unknown |
| 前端结果过多 | bounded cursor pagination、server summary、按需加载 detail |

## 13. 分阶段交付与批准点

1. **纯合同与 fixture：** status、parameter、resolution reducer、capture v3 schema；无 schema/网络变化。
2. **持久化 provenance：** additive migration、guarded repo、stage purge；执行前必须取得 schema/`golish-db` 明确授权。
3. **确定性采集/分析：** browser v3、AST argument/module/client context；仍无主动写请求。
4. **受控调度：** server-owned shard、角色/工具 mask、滚动 wave；先 shadow。
5. **Gate cutover：** 新 operation 冻结 v2；必须基于 shadow fixture 报告单独批准。
6. **Typed API 与生产 UI adapter：** ts-rs 生成类型和前端 provenance/progress；直接复用已落地的 `StageTeamWorkspaceView` evidence supplementary 区与现有 Agent 选择，不创建第二套 Workspace shell。

Schema 授权、production contract cutover 和真实目标/browser/provider 验收是三个独立批准点；任何一个批准都不自动包含另外两个。

本设计只对应一个 production feature：shadow 实现与报告只是 cutover 前置证据，不能把该 feature 标为 `passing`。只有获批切换新 operation 到 `agent_team_v2`，并对 production contract 完成 focused 行为验证后，才满足完成条件。

## 14. 验收标准

实现完成必须至少证明：

1. 同一 canonical endpoint 的浏览器调用和两个 JS callsite 形成三条 occurrence，均可追踪。
2. 同 method/URL 的两个 body shape 都保留，canonical 参数取并集但每个参数能回到来源。
3. `/b/xxx` 在 axios `/a` client、origin-root fetch 和 relative fetch 三种上下文得到正确且不同的 disposition。
4. 不可唯一解析的候选不会写入 `api_endpoints`，但能在 UI/Gate handoff 中看见。
5. unsafe method 在离开浏览器前被阻断，capture 明确 `sent=false`，且不保存敏感值。
6. typed shard 不重叠、工具 mask 由 host 强制、Controller-only final submit 不退化。
7. Gate 能区分 checked-empty、unresolved-with-receipt 和 never-analyzed。
8. Worker、occurrence、参数与 evidence 可双向追踪，错误 scope/worker 的读写均 fail closed。
9. legacy operation 与 capture v1/v2 继续可读，新 v2 operation 不被历史数据污染。
10. Chromium runtime initiator 只来自可关联的 CDP 事件；无 CDP 时明确降级且不伪造源码位置。
11. 未晋升、ambiguous、unresolved 与 scope-excluded candidate 可通过显式 operation-scoped API/UI 查询，不因没有 canonical endpoint 而消失。
12. Shadow 证据不会被当作 production 完成；`passing` 必须有获批 cutover 后的新 operation focused evidence。

## 15. 决策摘要

- 选择 Company Controller + server-owned bounded workers，不选择单工具增强或 per-URL Agent fan-out。
- JS/API/PARAM 仍属于 Enumeration，不新增顶层阶段。
- 选择 deterministic-first、AI-on-unresolved，不在叶子工具内保留 one-shot AI 主路径。
- 选择 per-occurrence URL Resolution Context，不选择 company/origin 单一主路径。
- 选择 canonical identity + immutable terminal occurrences，不让 first/last-writer 覆盖 provenance。
- 第一版严格只读，不主动重放写接口；主动验证留给有明确授权的 Verification 阶段。
