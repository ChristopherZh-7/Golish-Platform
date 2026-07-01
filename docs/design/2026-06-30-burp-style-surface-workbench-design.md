# Burp 式 Target Surface 工作台重设计（Inspector / Endpoints / History / Repeater）

> Date: 2026-06-30
> Status: approved（用户已拍板 §13，2026-06-30；下一步 writing-plans）
> Author: BajieAsk-agent-2（全栈工程师）
> Related:
> - `frontend/components/TargetPanel/TargetSurfaceWorkbench.tsx`、`frontend/components/TargetPanel/surface/*`（现状 surface/sitemap UI）
> - `scripts/browser_collect_js_api.mjs`（capture 写入：`saveApiResponseCapture` / `saveScriptCapture`）
> - `backend/crates/golish-pentest-app/src/pentest_bridge/{browser_collect_js_api.rs,js_extract_apis.rs,route_probe_paths.rs}`
> - `backend/crates/golish-db/src/repo/api_endpoints.rs`（`api_endpoints` 真值）
> - `docs/design/2026-06-30-jsapi-ai-tools-design.md`（同期 JS/API 工具内建 AI）
> Invariants touched: I2（IDOR/所有权）、I3（后端独立校验）、I5（ts-rs 跨 IPC 类型）、I7（证据可追溯）、§2.5（pentest evidence）、§2.7（高风险主动操作先确认）
>
> 注：设计文件位置遵循 AGENTS.md §2.4（`docs/design/`），与同期 `2026-06-30-jsapi-ai-tools-design.md` 一致；brainstorming skill 默认的 `docs/superpowers/specs/` 在本仓库让位于项目约定。

---

## 1. Problem / 背景

现状 Surface 工作台把 endpoint 与 JS 资产混在同一棵 Burp 式路径树里（`SitemapTab`）。今天已修复一个阻断性 bug（`js_analysis_results.url` 存的是裸 origin，导致同 host 上百个 JS 在树里塌成一个节点且被 dedupe 丢弃；前端改用 `origin + filename` 重建真实 URL 后已正确分层）。但从真实数据看，现有 UI 对渗透测试仍不够用：

1. **信号被噪声淹没**：`_next/static/chunks` 上百个哈希 bundle 把树塞满，真正的攻击面（endpoint/param）被埋。
2. **`SitemapNode` 只渲染 `node.items[0]`**：同路径多 method（GET/POST）被吃掉。
3. **无全局搜索/过滤、无展开收合、深树无虚拟化**。
4. **看不到请求/响应实体**：capture 已落盘但 UI 不展示，无法像 Burp 那样审 request/response。
5. **不能重放请求**：无 Repeater，验证一个 endpoint 必须跳出工具。

**现有数据能力（已查证）：**

- API capture（`captures/{host}/{port}/api/{parent}/{method}_{hash}_{name}.json`，payload v1）**只存 response**：`status / headers / content_type / body_len / body_sha256 / body_text_sample(64KB) / body_base64`；request 仅 `method / url / resource_type`，**无 request headers / body / cookies**。
- **无 HTTP 重放（Repeater）能力**（后端 `replay` 全是 transcript 重播）。
- `api_endpoints` 有 `params`（query+body 合并的扁平名数组）、`headers`(response)、`statusCode`、`responseType`、`capturePath`、`source`；但**无 per-request 历史表**（按 `(target,url,method)` 去重）。

## 2. Decision（TL;DR）

把 Surface 工作台重构为 **Burp 式三栏工作台**，按 4 个**可独立交付**的子系统组成，共享一个 Inspector 详情面板，**增量落地（1→2→3→4），但本设计一次覆盖全部**：

- **#1 Inspector（Req/Resp 检视）**：选中 endpoint/capture → 读 capture JSON 展示完整 request/response（含语法高亮、headers、body、render/hex/raw）。补 `browser_collect_js_api.mjs` 落 request headers + body（capture payload v2），新增**只读** IPC `pentest_read_capture`。
- **#2 Endpoints 表**：把攻击面从树里抽出来，做成**可排序/过滤**的扁平表（method/path/params/auth/status/source/→源 JS），点行进 Inspector。数据来自既有 `api_endpoints`，**无新存储**。
- **#3 History（请求历史）**：per-target 请求时间线，**派生自 capture 文件 + audit_log**（不新增表），新增只读 IPC `pentest_list_captures`；Repeater 产生的请求以 `source=repeater` 追加 capture。
- **#4 Repeater（capture replay）**：新增**受 scope 严格约束**的后端命令 `pentest_send_request`，重送可编辑请求、落新 capture、写 audit。**主动外发，按 §2.7 必须有授权模型 + 用户确认**。

守住不变量：所有跨 IPC 类型走 ts-rs（I5）；读 capture 做路径穿越防护、Repeater 做 scope/所有权校验（I2/I3）；落库/落盘证据可追溯（I7）；Repeater 不自动利用、单请求用户发起（§2.5/§2.7）。

## 3. 架构总览

```text
TargetSurfaceWorkbench（壳，保留 tab 导航）
├─ 顶部：全局 search（path/method/param/status）+ kind 过滤
├─ 左栏（可切换两种视图）
│   ├─ Sitemap 树（资产/路径，已修复分层；JS 默认折叠哈希 chunk）
│   └─ Endpoints 表（#2，可排序/过滤；攻击面主视图）
├─ 中栏：Inspector（#1，共享详情）
│   ├─ Request 子页（method/url/headers/body）   ← capture v2 / Repeater 编辑态
│   └─ Response 子页（status/headers/body：render｜raw｜hex）
└─ 右栏/底栏
    ├─ Params（#2 衍生，当前 endpoint 的参数表）
    ├─ Repeater（#4，把当前 request 载入可编辑器 → 发送 → 结果回 Inspector）
    └─ History（#3，当前 target 的请求时间线，点条目回灌 Inspector）
```

设计原则（brainstorming「隔离与清晰」）：Inspector 是纯展示单元（输入 = 一个 `CapturePayload`），Endpoints/History 是数据列表单元（输入 = 已有 DB/capture），Repeater 是唯一的「写/主动」单元，边界清晰、可独立测试。

## 4. capture payload v2（向后兼容）

`saveApiResponseCapture` 增补 request 端字段（v1 阅读器对缺失字段降级显示「未捕获」）：

```jsonc
{
  "version": 2,
  "captured_at": "…",
  "request": {
    "method": "POST",
    "url": "https://h/api/x",
    "resource_type": "fetch",
    "headers": { "content-type": "application/json", "...": "..." }, // 新增
    "body": "…(postData，文本；二进制存 base64 字段)",                  // 新增
    "body_base64": null                                              // 新增（二进制 body）
  },
  "response": { /* 同 v1：status/headers/content_type/body_*  */ }
}
```

- Playwright 侧用 `request.headers()` / `request.postData()` 取值；超大 body 截断并记 `truncated`。
- **隐私**：capture 仅本地落盘（与现状一致），不进 audit detail、不外传；Authorization/Cookie 仅本地可见（渗透测试需要）。

## 5. 子系统设计

### 5.1 #1 Inspector（Req/Resp 检视）

- **IPC（新增，只读）**：`pentest_read_capture(capture_path: String) -> CapturePayload`
  - 后端：把 `capture_path` 解析为绝对路径后**必须 `starts_with(workspace/.golish/captures)`**（canonicalize 后比对，拒绝 `..` 穿越，I3）。读 JSON 反序列化为 `CapturePayload`（ts-rs）。
  - 命名遵循 `<domain>_<verb>_<object>`（I4），走 `frontend/lib/api/pentest.ts` 包装（M2）。
- **前端 `Inspector` 组件**：输入 `capturePath`，加载后渲染 Request/Response 两子页；body 支持 `render | raw | hex`，JSON 美化、headers 表格、复制按钮、大小/状态标注。v1 capture 的 request headers/body 显示「未捕获（需重采集）」。
- **接入点**：Sitemap 选中 endpoint、Endpoints 表选中行、History 选中条目，都把 `capturePath` 喂给 Inspector。

### 5.2 #2 Endpoints 表

- **数据**：`apiEndpointsList(targetId)`（已存在），前端转可排序/过滤表：列 = method · path · params(数) · auth · status · source(crawler/js_analysis) · 源 JS(→buildSitemapJsSources) · capture(→Inspector)。
- **交互**：列头排序、文本过滤、method/source/有无 capture 过滤；点行 → Inspector + Params 面板。
- **Params 面板**：展示该 endpoint 的 `params`（现为扁平名数组）。**v1 不区分 query/body**（现状存储已合并）；区分留作可选增强（见 §13）。
- **无新存储 / 无新 IPC**。

### 5.3 #3 History（请求历史）

- **数据来源（决策：派生，不新增表，YAGNI/I10 友好）**：
  - 新增只读 IPC `pentest_list_captures(target_id) -> Vec<CaptureRef>`：扫 `captures/{host}/{port}/api/**.json`（host/port 来自该 target 及其同 IP 域），返回 `{capture_path, method, url, status, content_type, captured_at, source}`（只读 metadata，不读 body）。
  - 与 `audit_log`（工具运行时间线）合并出「请求历史」视图，按时间排序。
- **Repeater 产物**：`pentest_send_request` 落新 capture 时写 `"source":"repeater"`，History 自然纳入并标记。
- **若将来需要严格时序/去重无关历史** → 再加轻量 `http_request_log` 表（§13 开放项），本期不做。

### 5.4 #4 Repeater（capture replay）—— 高风险，需授权模型

- **后端命令（新增）**：`pentest_send_request(target_id, request: ReplayRequest) -> ReplayResult`
  - `ReplayRequest { method, url, headers: Map, body: Option<String> }`（可编辑）。
  - **授权闸（I2/I3/§2.7，全部满足才发包）**：
    1. `target_id` 解析到真实 target 且 `scope == "in"`（在范围内）；否则拒绝。
    2. `request.url` 的 host 必须 ∈ 该 org 在范围内的 host 集合（同 IP 域 / 已确认 web root）；**拒绝越域**。
    3. scheme 仅 http/https；rate-limit + size cap + timeout；禁止自动跟随到越域重定向。
    4. **前端发送前二次确认**（§2.7 主动外发）。
  - **执行**：用 reqwest 发送，**落新 capture（payload v2，`source=repeater`）**，写 `PentestAudit`（记 method/url/status，**不记 Authorization/Cookie 值**），可选写 `technique_outcomes`。
  - 命名 `pentest_send_request`（`<domain>_<verb>_<object>`，I4）；走 `lib/api/pentest.ts`（M2）。
- **前端 `RepeaterPanel`**：从 Inspector「Send to Repeater」载入当前 request → 可编辑 method/url/headers/body → 发送 → 结果回 Inspector + 进 History。
- **明确不做**：不做扫描器、不做自动 fuzz、不做批量、不做 intruder（§2.5：枚举阶段不在此主动利用）。

## 6. 数据模型 / IPC 契约（ts-rs）

新增 `#[derive(ts_rs::TS)]` 类型（I5，前端从 `frontend/lib/generated/` import）：

- `CapturePayload { version, captured_at, request: CaptureRequest, response: CaptureResponse }`
- `CaptureRequest { method, url, resource_type, headers: BTreeMap<String,String>, body: Option<String>, body_base64: Option<String> }`
- `CaptureResponse { status, headers, content_type, body_len, body_sha256, body_text_sample, body_base64, truncated }`
- `CaptureRef { capture_path, method, url, status, content_type, captured_at, source }`
- `ReplayRequest { method, url, headers, body }` / `ReplayResult { status, capture_ref: CaptureRef, elapsed_ms }`

IPC（三条，均走 facade + `lib/api/pentest.ts`）：`pentest_read_capture`、`pentest_list_captures`、`pentest_send_request`。

## 7. 前端组件结构

```
surface/
  tabs/SitemapTab.tsx        （改：默认折叠哈希 chunk；渲染 node 所有 items 而非 items[0]；展开收合全部）
  tabs/EndpointsTab.tsx      （新：#2 可排序/过滤表）
  tabs/HistoryTab.tsx        （新：#3 时间线）
  Inspector/                 （新：#1 Req/Resp 检视，render|raw|hex）
  Repeater/                  （新：#4 可编辑请求 + 发送）
  surfaceModel.ts            （扩：endpoints 表选择器、capture 解析适配）
```

全局 search 提到 `TargetSurfaceWorkbench`，按 kind/method/path/status 过滤下发各 tab。所有 IPC 仅经 `lib/api/pentest.ts`（禁裸 invoke，M2）。

## 8. 不变量对齐

- **I2**：`pentest_send_request` 校验 target 所有权 + scope + host 越域；`pentest_read_capture` 仅限 target 的 capture（路径前缀 + 后续可加 target 归属校验）。
- **I3**：路径穿越防护、scope 校验在后端做，不信前端。
- **I5**：所有 capture/replay 类型 ts-rs 同步，不手维护两份。
- **I7/§2.5**：Repeater 响应落 capture + audit，可追溯；Inspector/History 只读真实落盘证据。
- **§2.7**：Repeater 主动外发，前端二次确认 + 后端授权闸。
- **I10**：History 默认派生不动 schema；如加表走向后兼容迁移。

## 9. Files（预计改动）

| File | Change | 子项目 |
|---|---|---|
| `scripts/browser_collect_js_api.mjs` | `saveApiResponseCapture` 落 request headers/body（v2） | #1 |
| `golish-pentest-app/.../capture_read.rs`（新） | `pentest_read_capture` 实现 + 路径守卫 | #1 |
| `golish-pentest-app/.../capture_list.rs`（新） | `pentest_list_captures` | #3 |
| `golish-pentest-app/.../request_send.rs`（新） | `pentest_send_request` + 授权闸 + 落 capture/audit | #4 |
| `golish/src/commands_facade/pentest.rs` + registry | 注册三条命令（不直改 registry glob，I4） | #1/#3/#4 |
| `*/ports` + ts-rs types | `CapturePayload/CaptureRef/ReplayRequest/ReplayResult` | all |
| `frontend/lib/api/pentest.ts` | 三个 wrapper | all |
| `frontend/components/TargetPanel/surface/*` | Inspector/Endpoints/History/Repeater + Sitemap 改进 | all |
| `docs/modules/...` 卡 + `feature_list.json` + `agent-progress.md` | 同步（I6/§2.4） | all |

## 10. Verification（DoD）

```bash
cd backend && cargo nextest run -p golish-pentest-app --status-level fail
cd backend && cargo clippy -p golish-pentest-app --all-targets -- -D warnings
node --check scripts/browser_collect_js_api.mjs
pnpm test:run -- frontend/components/TargetPanel   # FE 模型/组件测试
just check                                          # 全套（含 ts-rs 同步）
```

关键测试：read_capture 路径穿越被拒 / v1 capture 降级；endpoints 表排序过滤；list_captures 归属过滤；**Repeater scope 闸（在范围放行、越域拒绝）+ audit 不含 secret + 落 capture**；ts-rs 类型同步。

## 11. 错误处理 / 回滚

- read_capture：文件缺失/损坏/越界 → 结构化错误码（I1），Inspector 显示降级态。
- list_captures：目录缺失 → 空列表（非报错）。
- Repeater：scope 拒绝/超时/越域重定向 → 明确错误码 + audit「blocked」；网络错误不写 capture。
- 回滚：四子项目各自独立 PR，可单独 revert；capture v2 向后兼容（v1 仍可读）。

## 12. 增量实现顺序（4 个 PR）

1. **#1 Inspector**（含 capture v2 + read_capture）— 骨干，纯读。
2. **#2 Endpoints 表 + Params**（无新后端）。
3. **#3 History**（list_captures，派生）。
4. **#4 Repeater**（先写独立授权小节获批 → 再实现）。

每个 PR 自带 TDD（先红后绿）+ `just precommit` 绿 + 模块卡/feature_list/progress 更新。

## 13. 决策记录（用户已拍板 2026-06-30：全部采用建议默认）

1. **Repeater 授权**：✅ 仅 `target.scope == "in"` 且 `request.url` host ∈ org 在范围 host 集合；scheme 仅 http/https；rate/size/timeout 上限；拒绝越域重定向。**确认节奏**：每个 (target host) 每会话**首次发送确认**一次，确认后该 host 持续显示「active sending」横幅，同 host 后续发送不再逐次弹窗（平衡 §2.7 安全与 Burp 可用性；可在设置里改为「每次确认」）。
2. **Param 区分 query/body**：✅ 延后（YAGNI）。本期 `api_endpoints.params` 维持扁平名数组；`{name, location}` 升级留作后续增强。
3. **History 存储**：✅ 派生（capture 文件 + audit_log，不动 schema）。严格时序的 `http_request_log` 表延后。
4. **左栏默认视图**：✅ 默认 Endpoints 表（攻击面优先），Sitemap 树作次视图（可切换）。

> 下一步：以本设计为准用 writing-plans 出实现计划 `docs/superpowers/plans/2026-06-30-burp-style-surface-workbench.md`（按 §12 四个 PR 逐个 TDD 落地）。本设计独立新增，不覆盖旧文档（I6）。
