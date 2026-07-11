# Enumeration Web Origin 分母与非终态收口

> 状态：Accepted（2026-07-10，用户在项目复核后确认“开始”）。
>
> 本文只收敛 `enumeration`：Web Origin 分母、`error` / `partial` 非终态、
> gate/worklist/pass-token 一致性。它不改变其它 stage 的历史 `error` 合同，
> 不进入 `vuln_triage`，不改数据库 schema / migration。

## 1. 结论

Enumeration 的确定性完成单位改为：

```text
normalized Web Origin × GOLISH-ENUM-{JS,DIR,PARAM,JSAPI}
```

其中 normalized Web Origin 固定为显式端口的
`scheme://host:port`，例如 `https://a.example:443`。路径、query、fragment
不属于分母身份；`http://a:8080` 与 `https://a:8080` 是两个不同单元。

Enumeration 的完成状态固定为：

| outcome | 是否完成 cell | 语义 |
|---|---:|---|
| `found` | 是 | 本次完整闭包发现内容 |
| `empty` | 是 | 本次完整闭包确实无内容 |
| `blocked` / `not_applicable` | 是 | 显式、可审计的策略终态 |
| `error` | 否 | 本次尝试失败，可修复/重试 |
| `partial` | 否 | 只完成部分队列/闭包，原始发现可保留但不能宣称完成 |

`stage_worklist_status.ready_to_submit`、per-org gate、submit preview 和
pass-token 必须消费同一状态机。只要当前 origin×technique 还有 `error` 或
`partial`，gate 必须 BLOCK，不能写 `org_stage_completions`，也不能发 pass token。

## 2. 当前缺陷

### 2.1 gate 与 worklist 对 error 的合同相反

`rule_engine::coverage_complete` 当前把 `EvidenceOutcome::Error` 当终态，
而 worklist 要求 `error_cells == 0` 才可提交。因此真实 run 会出现
`ready_to_submit=false` 后十几秒 gate PASS 的自相矛盾结果。

### 2.2 target 不是 Web 内容枚举的正确分母

当前 stage snapshot 一行一个 target，并从 `targets.ports[].url` 只挑一个
“最佳” URL。同一 target 的 80/443/8443 被丢成一格；gate 的通用
`canon_asset` 还会去掉 scheme，进一步把不同 origin 合并。

### 2.3 partial 被伪装成 empty/found

`route_probe_paths` 在请求预算/时间预算耗尽且无命中时仍写 `empty`；
`browser_collect_js_api` / `js_extract_apis` 在 closure 未完成时仍可能根据已落的
少量行写 `found`。业务行一旦被 DB truth 投影，又会绕过 technique outcome
直接闭格。

## 3. 身份合同

新增一个 I/O-free 的共享 helper，输入任意 HTTP(S) URL，输出：

```rust
pub struct WebOriginKey {
    pub key: String,      // https://a.example:443
    pub root_url: String, // https://a.example:443/
    pub scheme: String,
    pub host: String,
    pub port: u16,
}
```

规则：只接受 HTTP(S)、拒绝 credentials、host 必须存在、默认端口显式化、
host 小写、IPv6 保留合法方括号。所有 bridge evidence 与
`technique_outcomes.asset` 必须写 `key`；执行请求仍使用完整 effective URL。

Enumeration roots 从当前 target metadata 的**全部** confirmed HTTP(S) origins
展开，而不是 `max_by_key` 只选一个。显式 URL target 可作为兼容 root；无法
确认 scheme+port 的 host 不允许猜测默认 origin。

JS capture 也必须服从同一身份：新路径为
`.golish/captures/{host}/{port}/{scheme}/js/`，禁止 `http://h:p` 与
`https://h:p` 共用 manifest/文件。`js_extract_apis` 只读取当前 exact origin 的
scheme namespace；无法证明 origin 的 legacy capture 不能直接用于 completion。

当前实现优先复用现有 repo/target metadata，不修改 `golish-db` crate；若后续
必须改 `golish-db`，按 AGENTS.md §2.7 单独请求确认。

## 4. 完成真相合同

### 4.1 technique_outcomes 是 Enumeration 完成真相

`directory_entries`、`api_endpoints`、`js_analysis_results` 是发现数据，不是
“完整跑完”证明。Enumeration 四轴不再让这些 host-level business rows直接闭格；
只有本次工具以 exact origin 写入的 `technique_outcomes` 才能标记完整完成。

这条规则同时解决 partial raw rows 假 PASS：partial run 仍保留已发现 URL/JS/API，
但它写 `outcome='partial'`，coverage 保持未完成；后续完整复跑通过现有
`(run_id, asset, technique)` upsert 覆盖成 `found` 或 `empty`。

### 4.2 partial marker

`technique_outcomes.outcome` 是 TEXT，因此直接使用 `partial`，无需 migration。
marker 不伪造 completion evidence；响应保留 `completion_state='partial'`、
`queue_completed=false` / `closure_complete=false` 和原因。coverage UI/worklist
显示 `partial`，默认 repair preference 把它与 `pending` / `error` 一起返回。

### 4.3 三个 bridge 的判定

- `route_probe_paths`：只有 `queue_completed=true` 才允许 `found` / `empty`；
  timeout、request limit、candidate-generation limit 任一导致队列未完成时写
  `partial`，即使已经发现若干路径也不闭格。
- `browser_collect_js_api`：`closure_complete=false`、`closure_partial`、
  `timeout_partial` 时 JS/JSAPI/PARAM 均写 `partial`；已抓原始行照常保存。
- `js_extract_apis`：输入 JS 未完整读取/分析或返回 partial 时 JSAPI/PARAM 均写
  `partial`；只有完整 pass 才写 found/empty。

hard process failure 写 `error`。`blocked` 只能来自明确策略/授权终态，不由普通
transport failure 自动升级。

## 5. gate 与 pass-token

`coverage_complete` 增加向后兼容开关 `error_is_terminal`，默认 `true`；仅
Enumeration spec 设 `false`。同时，Enumeration 的 join key 对四个 ENUM technique
使用 exact Web Origin，其他 stage 继续用既有 `canon_asset`。

`AssetClass::classify` 会有意把 `http(s)://IP:port` 按 host 语义归为 `Ip`，即使
snapshot 的 `target_type` 已改成 `url`。因此四轴适用性不能再要求 exact-origin 字符串
命中 raw-IP `web_capable_assets`：`technique_applies_web_aware` 必须把任何可 canonicalize
的 exact HTTP(S) origin 视作 intrinsically web-capable；只有裸 IP/CIDR 才依赖上游
web-capable proof。旧 DNS/53-only host `not_applicable` context 不得关闭 exact-origin
cell，read model、submit preview 与 final org gate 都要保留四个 pending gap。

`partial` 不映射成 `EvidenceOutcome::Empty`，也不能成为 terminal error completion。
当前 gate 内部把它复用为 `EvidenceOutcome::Error` sentinel，但 Enumeration 的
`error_is_terminal=false`，所以该 sentinel 只表示当前 run 未完成，并会优先否决交付物
自报终态。因为 Enumeration 不再消费 business-table found 投影，partial marker 不会被
已落的少量发现绕过。

Enumeration 也不消费 `source_query_log` 或 audit/business compatibility facts 来
关闭四轴；三条 gate 装配路径都先清除这些 ENUM facts，再仅投影 `stage_started_at`
之后、当前 session 的 `technique_outcomes`。active-session read model 禁止 latest-run
fallback，避免同一 chat session 的上一次 Enumeration 结果被新 run 复用。

Enumeration 的 `coverage_complete` 使用 `authoritative_found=true`、
`require_note_for_other=true`。当前 error/partial marker 的优先级高于交付物自报的
found/checked_empty/blocked/not_applicable；只有完整复跑覆盖 marker 才能解除缺口。
found/empty outcome 必须引用真实 ledger evidence id：evidence append 失败时 producer
不写 terminal outcome，gate/read-model 也拒绝 `evidence_ids=[]` / sentinel id 0。
`freshness_window=true` 但拿不到 stage start cutoff 时按缺行处理，不回退到同 session
历史 outcome。

pass-token 路径无需新增旁路：per-org gate BLOCK 后不会写
`org_stage_completions`，聚合 closeout 自然无法发 token。测试必须锁住这条链。

## 6. crawler 授权边界

`enum_crawl_same_origin_urls` 必须约束**实际请求**，不能只过滤输出。Katana
批处理命令增加 anchored exact-origin union scope，覆盖 scheme+host+port；既有
输出 same-origin filter 保留为纵深防御。scheme flip、wrong port、sibling host 和
`example.com.evil` 均不得进入调度范围。

同一边界适用于另外两个主动 collector：browser navigation、recipe fetch、recursive
script fetch 和 `route_probe_paths` redirect 每一跳都只能留在 exact origin；跨
scheme/host/port 必须 abort/stop 并标为 error 或 partial，不能先请求后只过滤结果。
browser 的导航全失败是 error；部分页面失败、page budget 未闭、JS size skip 等是
partial，不能以 `status=ok/closure_complete=true` 写三轴 empty。

## 7. 兼容与回滚

- 无 schema/migration/generated TS 变更。
- 历史 host-key outcome 不删除、不回填；新 Web-Origin axis 下不允许它关闭多
  origin cell。旧 run 需要 fresh Enumeration rerun，这是有意的 fail-closed 行为。
- 其它 stage 的 error-terminal 合同保持不变。
- 回滚时可关闭 Enumeration origin axis / `error_is_terminal=false`，旧行仍在；
  不需要数据回滚。

## 8. 验收

1. 同一 target 的 HTTP 80、HTTPS 443、HTTPS 8443 生成三组四轴 cell。
2. 一个 origin 的完整 found/empty 不关闭 sibling origin。
3. `error` / `partial` 时 worklist 与 gate 都未完成，且不能产生 pass token。
4. partial run 的原始发现保留，完整复跑能覆盖 marker 并闭格。
5. Katana 不请求授权 origin 集之外的 scheme/port/host。
6. headless fixture 与 fresh Test1 运行中，preflight ready 与 gate verdict 一致。

## 9. 浏览器采集活性补充（2026-07-11）

`browser_collect_js_api` 的 budget 必须是可续跑 slice，不能成为隐式丢数据上限。
旧 helper 把 DOM link 数限制为 `max(max_pages * 3, 10)`；因此一个有 301 个安全
同源链接的入口在 `max_pages=1` 时只保存 10 个 pending，另外 291 个只计
`page_budget_truncated` 后永久丢失。递归 JS queue 也没有写进 manifest，
`max_recursive_scripts=1` 的链每次都从第一个 chunk 重新开始。两者都会让
Enumeration 长期 partial，却永远不能推进到 closure。

修订后的 manifest v2 保存：

- 所有安全 exact-origin `pending_pages`，`max_pages` 只决定本次消费多少页；
- `pending_recursive_scripts`，让 bounded recursive slice 从上次 cursor 继续；
- 当前 provenance 已验证的 scripts/API 与 page/recursive resume count；
- transport/body failure 的稳定 `kind + canonical URL` signature 与累计次数。

恢复仍严格绑定当前 run、session、trusted operation、stage attempt 与 exact origin。
每次加载都重新验证 pending URL、危险 route、manifest freshness 和旧脚本完整性；
scope/ownership/provenance 漂移不能恢复。navigation/page-inspection、script/API/
manifest/recursive body、recursive fetch、pending wait 这类可重试失败，单 signature
最多允许两次。同一条件第二次仍失败时返回 `recovery_exhausted=true`、
`automatic_retry_allowed=false`、非终态 `error` 和明确恢复说明；后续同 provenance
不得再次发网络请求，Rust 内建 AI recipe 也不得继续循环。

这个 breaker 只解决活性和重复请求，不创造完成真相：exhausted 仍不能关闭
JS/JSAPI/PARAM cell，也不能自动转换为 `blocked`/`checked_empty`。要恢复需先修正
transport/timeout 条件，再由 trusted orchestration 建立新的 producer operation 或
stage attempt。若产品未来要把长期不可达 origin 变成可审计策略终态，必须另加
target-bound、带原因和 evidence 的 operator/preflight blocked 决策，不能由 Node
collector 自行升级。

新增回归必须覆盖：301 links + `max_pages=1` 全量 checkpoint；三段 recursive chain +
`max_recursive_scripts=1` 跨 invocation 前进；同一 navigation signature 第二次失败后
exhaust，第三次同 provenance 零网络请求；exhausted result 不触发 Rust AI recipe。
