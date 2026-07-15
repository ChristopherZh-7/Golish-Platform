# JS/API 候选上下文解析 v1

> Status: implemented and repository-verified; fresh live-site rerun not performed
> Date: 2026-07-14
> Scope: `golish-js-analyzer` / `js_extract_apis`

## 1. 一句话契约

保留现有 browser collector、deterministic broad capture 和 AI opt-in 边界，在
`raw endpoint call-site` 与 `api_endpoints` 之间增加一个确定性的上下文解析层：
只有能绑定到同文件命名 HTTP client 及唯一 base path 的调用才拼接 prefix；歧义继续
保留为 raw evidence，不选择第一个候选冒充事实。

```text
raw call-site candidate
  -> source span + receiver evidence
  -> same-file client/base binding
  -> exact-origin guarded URL resolution
  -> resolved URL dedupe
  -> api_endpoints
```

## 2. 当前根因

当前实现有两个相互放大的问题：

1. `detect_api_base_path` 只认少量配置形状，并把跨所有文件找到的第一个 path 套到
   每一个 root-relative endpoint 上；多 axios client 时会错拼，client-local prefix
   也无法表达。
2. `js_ai_extract::merge_dedup` 在 contextual resolution 之前按
   `(uppercase method, raw path)` 去重。两个 client 都调用 `GET /users` 时，第二条在
   base 解析前已经丢失，后面即使识别更多 `baseURL` 写法也无法恢复。

analyzer 的 `HTTP_CLIENT_VERB` 已捕获 receiver identifier，但构造 `Endpoint` 时丢弃；
AST 层只验证 regex 命中位于真实 call expression 中，没有输出 call span/callee。

## 3. v1 目标与非目标

### 目标

- analyzer 新增并行的 call-site candidate API，输出原 `Endpoint` 加 source span、
  callee、receiver；旧 `extract_from_source/files` 签名和 `Endpoint` JSON 保持兼容。
- bridge 解析同文件 `axios.create({baseURL: ...})`、一步 literal 常量、
  `client.defaults.baseURL = ...` 与简单 alias 链。
- 同一 raw leaf 可因不同 client base 产生不同 resolved URL；最终只按
  `(method, resolved URL)` 去重。
- `js_analysis_results.raw_analysis.contextual_resolution_v1` 保存 candidate、binding、
  disposition 和理由；只有唯一解析结果进入 `api_endpoints`。
- exact-origin、安全 scope、AI 默认关闭和现有 evidence/outcome 规则保持不变。

### 非目标

- 不改 DB schema/migration，不改 generated IPC 类型。
- 不读取或修改 `route_probe_paths`；resolved endpoint 落表后由它的现有 seed 查询自然消费。
- 不做跨 chunk import/export、完整 JS scope/type inference、递归 wrapper 解释或 Service Worker。
- 不用 AI 猜 prefix；AI 仍只能处理后续歧义 candidate group，且本切片不实现该阶段。
- 不把唯一 axios instance base 当作 fetch 或未知 wrapper 的全局 base。

## 4. Analyzer 增量契约

新增类型：

```rust
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub line: usize,
    pub column: usize,
}

pub struct CallSiteContext {
    pub callee: String,
    pub receiver: Option<String>,
    pub span: SourceSpan,
}

pub struct EndpointCandidate {
    pub endpoint: Endpoint,
    pub call: CallSiteContext,
}

pub struct CandidateExtractReport {
    pub candidates: Vec<EndpointCandidate>,
    pub skipped: Vec<SkippedFile>,
    pub unique: usize,
}
```

新增 `extract_candidates_from_source/files`。旧 API 从 candidate 投影回 `Endpoint`，
避免 `golish-auth-probe` 和历史 `js_analysis_results` 同步迁移。span 使用原 regex match
的 byte range，因此 minified 单行内两个相同 leaf 仍可区分；AST parse 失败时保留 raw
candidate，但 bridge 不得凭缺失的 binding evidence 强拼 prefix。

## 5. Bridge 上下文索引

`ApiBaseContextIndex` 按 source file 建立三个事实集合：

1. literal symbol：`const BASE = "/admin-api"`；
2. client binding：`const admin = axios.create({baseURL: BASE})` 或
   `admin.defaults.baseURL = "/admin-api"`；
3. alias edge：`const api = admin`，以 bounded fixed-point 传播已知 binding。

同一 `(source_file, identifier)` 若出现多个独立 base fact、动态 base、可变/重赋值 symbol
或冲突 alias，状态为 `ambiguous`，不得挑一个值冒充事实；晚于 call-site 的 fact 也不得
倒灌。跨文件同名 identifier 不关联。`axios.create({...})` 内按 JavaScript 对象顺序处理：
重复显式 `baseURL` 取最后一项；后置 spread 令结果 dynamic；spread 后再出现显式
`baseURL` 则恢复为静态值。legacy 全局 `VITE_GLOB_API_URL/apiURL` 只作为没有明确
client binding 的 wrapper 兼容 fallback；多个全局候选同样保持 ambiguous。

解析优先级：

```text
same-file exact named-client binding
  > receiver-less fetch/Request/jQuery uses origin-root
  > unbound wrapper uses unique legacy global base
  > origin-root path

known named-client conflict/dynamic
  -> unresolved (no fallback)

opaque/member-chain receiver without an exact binding
  -> unresolved raw evidence (never bind its leaf identifier)
```

命名 Axios client 使用 Axios combine 语义：`baseURL=/v2` 与 `url=/v2/users` 会解析为
`/v2/v2/users`。legacy global prefix 为保持历史兼容仍执行 segment-aware 防双拼。无前导
`/` 的 custom-client 相对路径只在存在唯一命名 client binding 时提升；否则不进入 endpoint
projection。fetch/Request/jQuery 不继承无关的 application/axios prefix。

## 6. 持久化与证据

raw analysis 增加 versioned 对象：

```json
{
  "contextual_resolution_v1": {
    "client_bindings": [],
    "candidates": [
      {
        "candidate_id": "app.js:412:438:4f61b629b85d4c81",
        "method": "GET",
        "raw_path": "/users",
        "receiver": "admin",
        "base_path": "/admin-api",
        "resolved_path": "/admin-api/users",
        "disposition": "resolved"
      }
    ]
  }
}
```

candidate ID 包含 source span 与稳定内容 fingerprint；binding/candidate/global-path 数组都带
`total/omitted` 计数，base/evidence 字符串有明确上限，避免 minified 恶意输入放大 raw JSON。
该对象是推理证据，不替代 `api_endpoints` truth。最终 projection 仍经过现有 URL parser、
HTTP(S) scheme 和 exact-origin validator。foreign absolute base 必须 `scope_excluded`；
动态/冲突 binding 必须 `unresolved`，并使本次 JSAPI closure 保持 partial，而不是假 empty。
dry-run 不落数据库，但只要存在 resolved projection，其诊断 outcome 仍是 `found`，不能因
`persisted_endpoint_rows=0` 误报 empty。

## 7. 验证与回滚

必须覆盖：两个 axios client 同 leaf 不折叠；一步常量和 alias；同名跨文件不串线；
client base 不泄漏到 fetch；冲突/dynamic base 不猜；minified 同行用 span 区分；旧 API/JSON
兼容；prefix 判断按 segment boundary，`/api` 不得把 `/apiary/x` 当作已加前缀；还需覆盖
source-order、relative custom-client、member/optional chain、对象 duplicate/spread、模板字符串
伪配置、evidence bounds、supplemental resolved 后去重和 dry-run outcome。

本切片尚未对 Test1/真实站点做 fresh live rerun，因此只证明代码与聚焦测试闭环，不声称
真实站点采集到解析再落库的现场闭环已重新证明。

回滚只需让 `js_extract_apis` 重新消费旧 endpoint report；新增 analyzer API 和 raw JSON 字段
是 additive，不需要数据回滚。
