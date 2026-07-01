# Burp 式 Surface 工作台 · 实现计划（#1 Inspector）

> **面向 AI 代理的工作者：** 必需子技能：使用 executing-plans 逐任务实现此计划。每个任务单独 commit；TDD（先红后绿）；DRY / YAGNI。
> **设计来源：** `docs/design/2026-06-30-burp-style-surface-workbench-design.md`（用户已拍板 §13）。
> **范围说明（writing-plans 范围检查）：** 设计含 4 个子系统；本计划只覆盖 **#1 Inspector（Req/Resp 检视）**，它是骨干、可独立交付。#2 Endpoints 表 / #3 History / #4 Repeater 在 #1 落地后各出独立计划（顺序见设计 §12）。

**目标：** 让用户在 Surface 工作台点选一个 endpoint/capture，就能看到该请求的完整 request/response（headers + body，支持 render/raw/hex）。
**架构：** Playwright 收集器把 request headers/body 一并落进 capture（payload v2，向后兼容 v1）；新增只读 Tauri 命令 `pentest_read_capture`（路径穿越防护）读 capture JSON；前端 `Inspector` 组件渲染。
**技术栈：** Node/Playwright（`scripts/browser_collect_js_api.mjs`）、Rust/Tauri（`golish-pentest-app` + `golish` registry）、React/TS（`frontend/components/TargetPanel/surface`）、Vitest。

**关键既读证据（实现者无需重查）：**
- capture v1 payload 由 `saveApiResponseCapture`（`scripts/browser_collect_js_api.mjs:579`）写：`request{method,url,resource_type}` + `response{status,headers,content_type,body_len,body_sha256,body_text_sample,body_base64,...extra}`。
- response handler（`scripts/browser_collect_js_api.mjs:1368`）里 `const request = response.request()` 可取 `request.headers()`（同步）与 `request.postData()`（同步，无 body 时返回 null）。`entry` 在 `:1355` 建立。
- 现有只读 IPC 模式（`golish-pentest-app/src/security_analysis.rs`）：`#[tauri::command] pub async fn xxx(state: tauri::State<'_, DbState>, ...) -> Result<serde_json::Value, GolishError>`；在 `golish/src/commands_registry.rs:198` 的 `generate_handler!` 列表登记（经 facade `pub use`，不直改 glob，I4）。
- 前端 wrapper 模式（`frontend/lib/api/security-analysis.ts`）：`invoke("cmd", { camelKey })` + `normalizeXxx` 适配（本 domain 用 `serde_json::Value` 返回 + TS normalize，**非 ts-rs**；本计划遵循此既有模式）。
- 现有 `ApiEndpoint.capturePath`（`frontend/lib/api/security-analysis.ts:52`）= capture 文件相对 workspace 的路径（如 `.golish/captures/host/443/api/x/get_<hash>_x.json`）。

---

## 文件结构（创建/修改）

| 文件 | 职责 | 阶段 |
|---|---|---|
| `scripts/browser_collect_js_api.mjs` | capture payload v2：补 request headers/body | A |
| `backend/crates/golish-pentest-app/src/security_analysis.rs` | 新增 `pentest_read_capture` 命令 + 路径守卫纯函数 | B |
| `backend/crates/golish/src/commands_facade/pentest.rs` | `pub use` 暴露新命令（如需） | B |
| `backend/crates/golish/src/commands_registry.rs` | 在 `generate_handler!` 加 `pentest_read_capture` | B |
| `frontend/lib/api/security-analysis.ts` | `CapturePayload` 接口 + `normalizeCapturePayload` + `readCapture()` | C |
| `frontend/lib/api/security-analysis.test.ts` | normalize 单测 | C |
| `frontend/components/TargetPanel/surface/Inspector/Inspector.tsx` | Req/Resp 检视组件 | D |
| `frontend/components/TargetPanel/surface/Inspector/inspectorModel.ts` | body 渲染模式判定（render/raw/hex）等纯函数 | D |
| `frontend/components/TargetPanel/surface/Inspector/inspectorModel.test.ts` | 纯函数单测 | D |
| `frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx` | endpoint detail 接入 Inspector | E |

---

## 阶段 A：capture payload v2（请求端落盘）

### 任务 A.1：response handler 给 entry 补 request headers/body

**文件：** `scripts/browser_collect_js_api.mjs`

**步骤：** 在 response handler 命中 xhr/fetch 分支、设置 `entry.status` 之后（`:1376` 附近），补两行：

```js
      const entry = apiRequestsByKey.get(`${request.method()} ${url}`);
      entry.status = response.status();
      entry.headers = headers;
      entry.content_type = headers["content-type"] ?? "";
      // capture v2: persist the request side so the Inspector can show it.
      entry.request_headers = request.headers();
      entry.request_body = request.postData() ?? null;
```

**验证：** `node --check scripts/browser_collect_js_api.mjs`（exit 0）。

**提交：** `feat(collector): capture request headers/body for inspector (payload v2)`

### 任务 A.2：saveApiResponseCapture 写出 v2 request 字段

**文件：** `scripts/browser_collect_js_api.mjs`

**步骤：** 改 `saveApiResponseCapture`（`:608` 的 `payload`）：

```js
  const requestBody = typeof entry.request_body === "string" ? entry.request_body : null;
  const payload = {
    version: 2,
    captured_at: new Date().toISOString(),
    request: {
      method: entry.method,
      url: entry.url,
      resource_type: entry.resource_type,
      headers: entry.request_headers ?? {},
      body: requestBody && requestBody.length <= 64_000 ? requestBody : null,
      body_truncated: Boolean(requestBody && requestBody.length > 64_000),
    },
    response: {
      status: entry.status,
      headers,
      content_type: contentType,
      body_len: bodyBuffer?.length ?? 0,
      body_sha256: bodyBuffer ? sha256Hex(bodyBuffer) : null,
      body_text_sample: textualBodySample(contentType, bodyBuffer),
      body_base64: bodyBuffer ? bodyBuffer.toString("base64") : null,
      ...extra,
    },
  };
```

**验证：** `node --check scripts/browser_collect_js_api.mjs`（exit 0）。

**提交：** `feat(collector): write request headers/body in capture payload v2`

---

## 阶段 B：`pentest_read_capture` 只读命令（路径守卫）

### 任务 B.1：路径守卫纯函数 + 测试（先红）

**文件：** `backend/crates/golish-pentest-app/src/security_analysis.rs`

**步骤（测试先行）：** 在文件末尾 `#[cfg(test)]` 区加：

```rust
#[cfg(test)]
mod capture_read_tests {
    use super::resolve_capture_path;
    use std::fs;

    #[test]
    fn resolves_inside_captures_and_rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let cap_dir = ws.join(".golish/captures/h/443/api");
        fs::create_dir_all(&cap_dir).unwrap();
        let file = cap_dir.join("get_abc_x.json");
        fs::write(&file, "{\"version\":2}").unwrap();

        // ok: relative path under .golish/captures
        let ok = resolve_capture_path(ws.to_str().unwrap(), ".golish/captures/h/443/api/get_abc_x.json").unwrap();
        assert!(ok.ends_with("get_abc_x.json"));

        // reject: path traversal escaping captures
        assert!(resolve_capture_path(ws.to_str().unwrap(), ".golish/captures/../../etc/passwd").is_err());
        // reject: outside captures
        assert!(resolve_capture_path(ws.to_str().unwrap(), ".golish/other/x.json").is_err());
    }
}
```

实现纯函数（同文件，非 `#[tauri::command]`）：

```rust
/// Resolve a workspace-relative capture path to an absolute path, guaranteeing
/// it stays inside `<workspace>/.golish/captures` (rejects `..` traversal). The
/// file must exist (canonicalize). Read-only; never used for writes.
fn resolve_capture_path(project_path: &str, capture_path: &str) -> Result<std::path::PathBuf, GolishError> {
    let base = std::path::Path::new(project_path).join(".golish").join("captures");
    let full = std::path::Path::new(project_path).join(capture_path);
    let base_canon = base
        .canonicalize()
        .map_err(|e| GolishError::msg(format!("captures dir unavailable: {e}")))?;
    let full_canon = full
        .canonicalize()
        .map_err(|e| GolishError::msg(format!("capture not found: {e}")))?;
    if !full_canon.starts_with(&base_canon) {
        return Err(GolishError::msg("capture path outside captures directory"));
    }
    Ok(full_canon)
}
```

> 注：若 `GolishError` 无 `msg` 构造，改用本文件已用的错误构造方式（grep `GolishError::` 在本文件的现有用法，照抄一种返回 String 的构造）。

**验证：** `cd backend && cargo nextest run -p golish-pentest-app capture_read --status-level fail`（先红：函数未实现时编译失败 → 实现后绿）。

**提交：** `feat(pentest): capture path guard (reject traversal/outside captures)`

### 任务 B.2：`pentest_read_capture` 命令

**文件：** `backend/crates/golish-pentest-app/src/security_analysis.rs`

**步骤：** 加命令（紧邻 `js_analysis_list`）：

```rust
#[tauri::command]
pub async fn pentest_read_capture(
    _state: tauri::State<'_, DbState>,
    project_path: String,
    capture_path: String,
) -> Result<serde_json::Value, GolishError> {
    let full = resolve_capture_path(&project_path, &capture_path)?;
    let text = tokio::fs::read_to_string(&full)
        .await
        .map_err(|e| GolishError::msg(format!("read capture failed: {e}")))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(GolishError::from)?;
    Ok(value)
}
```

**验证：** `cd backend && cargo check -p golish-pentest-app`（exit 0）。

**提交：** `feat(pentest): pentest_read_capture read-only IPC`

### 任务 B.3：注册命令

**文件：** `backend/crates/golish/src/commands_facade/pentest.rs`、`backend/crates/golish/src/commands_registry.rs`

**步骤：**
1. 在 `commands_facade/pentest.rs` 确认/添加 `pub use golish_pentest_app::security_analysis::pentest_read_capture;`（照该文件现有 `pub use` 风格；若该 domain 命令在别处 re-export，照同一处加）。
2. 在 `commands_registry.rs` 的 `tauri::generate_handler![ ... ]` 列表里、`js_analysis_list,` 同段加入 `pentest_read_capture,`。

**验证：** `cd backend && cargo check -p golish`（exit 0）。

**提交：** `feat(app): register pentest_read_capture command`

---

## 阶段 C：前端 wrapper + 类型 + normalize

### 任务 C.1：CapturePayload 类型 + normalize + readCapture（含测试，先红）

**文件：** `frontend/lib/api/security-analysis.ts`、`frontend/lib/api/security-analysis.test.ts`

**步骤（测试先行）：** 在 `security-analysis.test.ts` 加：

```ts
import { normalizeCapturePayload } from "./security-analysis";

describe("normalizeCapturePayload", () => {
  it("normalizes a v2 capture", () => {
    const c = normalizeCapturePayload({
      version: 2,
      request: { method: "post", url: "https://h/api/x", headers: { "content-type": "application/json" }, body: "{\"a\":1}" },
      response: { status: 200, headers: { "content-type": "application/json" }, body_text_sample: "{}", body_len: 2 },
    });
    expect(c.request.method).toBe("POST");
    expect(c.request.headers["content-type"]).toBe("application/json");
    expect(c.response.status).toBe(200);
  });

  it("degrades a v1 capture (no request headers/body)", () => {
    const c = normalizeCapturePayload({
      version: 1,
      request: { method: "GET", url: "https://h/api/y" },
      response: { status: 204, headers: {}, body_text_sample: "" },
    });
    expect(c.request.headers).toEqual({});
    expect(c.request.body).toBeNull();
    expect(c.response.status).toBe(204);
  });
});
```

实现（`security-analysis.ts`）：

```ts
export interface CaptureRequest {
  method: string;
  url: string;
  resourceType: string;
  headers: Record<string, string>;
  body: string | null;
}
export interface CaptureResponse {
  status: number | null;
  headers: Record<string, string>;
  contentType: string;
  bodyLen: number | null;
  bodyTextSample: string;
  bodyBase64: string | null;
}
export interface CapturePayload {
  version: number;
  capturedAt: string;
  request: CaptureRequest;
  response: CaptureResponse;
}

function strMap(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) out[k] = String(v);
  return out;
}

export function normalizeCapturePayload(value: unknown): CapturePayload {
  const row = asRecord(value);
  const req = asRecord(row.request);
  const res = asRecord(row.response);
  return {
    version: nullableNumberField(row, "version") ?? 1,
    capturedAt: stringField(row, "capturedAt", "captured_at"),
    request: {
      method: (stringField(req, "method") || "GET").toUpperCase(),
      url: stringField(req, "url"),
      resourceType: stringField(req, "resourceType", "resource_type"),
      headers: strMap(get(req, "headers")),
      body: typeof req.body === "string" ? req.body : null,
    },
    response: {
      status: nullableNumberField(res, "status"),
      headers: strMap(get(res, "headers")),
      contentType: stringField(res, "contentType", "content_type"),
      bodyLen: nullableNumberField(res, "bodyLen", "body_len"),
      bodyTextSample: stringField(res, "bodyTextSample", "body_text_sample"),
      bodyBase64: nullableStringField(res, "bodyBase64", "body_base64"),
    },
  };
}

export async function readCapture(projectPath: string, capturePath: string): Promise<CapturePayload> {
  return normalizeCapturePayload(
    await invoke("pentest_read_capture", { projectPath, capturePath })
  );
}
```

**验证：** `pnpm test:run -- frontend/lib/api/security-analysis.test.ts`（先红→实现后绿）。

**提交：** `feat(fe-api): readCapture wrapper + CapturePayload normalize`

---

## 阶段 D：Inspector 组件

### 任务 D.1：body 渲染模式纯函数 + 测试

**文件：** `frontend/components/TargetPanel/surface/Inspector/inspectorModel.ts` + `inspectorModel.test.ts`

**步骤（测试先行）：**

```ts
import { bodyRenderMode, prettyBody } from "./inspectorModel";

describe("inspectorModel", () => {
  it("picks json mode for json content-type and pretty-prints", () => {
    expect(bodyRenderMode("application/json")).toBe("json");
    expect(prettyBody("json", '{"a":1}')).toContain("\n");
  });
  it("falls back to text for html/plain", () => {
    expect(bodyRenderMode("text/html")).toBe("text");
    expect(prettyBody("text", "<html>")).toBe("<html>");
  });
});
```

实现：

```ts
export type BodyMode = "json" | "text";

export function bodyRenderMode(contentType: string): BodyMode {
  return /json/i.test(contentType) ? "json" : "text";
}

export function prettyBody(mode: BodyMode, body: string): string {
  if (mode !== "json") return body;
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}
```

**验证：** `pnpm test:run -- frontend/components/TargetPanel/surface/Inspector/inspectorModel.test.ts`（绿）。

**提交：** `feat(inspector): body render-mode helpers`

### 任务 D.2：Inspector 组件

**文件：** `frontend/components/TargetPanel/surface/Inspector/Inspector.tsx`

**步骤：** 组件签名 `{ projectPath, capturePath }`；`useEffect` 调 `readCapture`，三态（loading/error/empty）；两子页 Request/Response，headers 用表格，body 用 `prettyBody(bodyRenderMode(contentType), ...)`；v1 capture 的 request headers/body 空 → 显示「未捕获（重新采集以填充）」。复用现有 `cn`、小字号样式（参照 `SitemapTab` 的 `EndpointDetail`）。完整代码：

```tsx
import { useEffect, useState } from "react";
import { type CapturePayload, readCapture } from "@/lib/api/security-analysis";
import { cn } from "@/lib/utils";
import { bodyRenderMode, prettyBody } from "./inspectorModel";

function HeaderTable({ headers }: { headers: Record<string, string> }) {
  const entries = Object.entries(headers);
  if (entries.length === 0)
    return <p className="text-[10px] text-muted-foreground">未捕获（重新采集以填充）</p>;
  return (
    <div className="mt-1 max-h-44 space-y-1 overflow-auto">
      {entries.map(([k, v]) => (
        <div key={k} className="grid grid-cols-[120px_minmax(0,1fr)] gap-2 text-[10px]">
          <span className="truncate font-mono text-muted-foreground">{k}</span>
          <span className="min-w-0 break-all font-mono text-foreground/80">{v}</span>
        </div>
      ))}
    </div>
  );
}

export function Inspector({
  projectPath,
  capturePath,
}: {
  projectPath: string;
  capturePath: string | null;
}) {
  const [data, setData] = useState<CapturePayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"request" | "response">("response");

  useEffect(() => {
    let cancelled = false;
    setData(null);
    setError(null);
    if (!capturePath) return;
    readCapture(projectPath, capturePath)
      .then((c) => !cancelled && setData(c))
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [projectPath, capturePath]);

  if (!capturePath)
    return (
      <div className="rounded border border-border/20 bg-muted/5 p-3 text-[11px] text-muted-foreground">
        No response capture stored for this entry.
      </div>
    );
  if (error)
    return (
      <div className="rounded border border-red-500/25 bg-red-500/5 p-3 text-[10px] text-red-300">
        {error}
      </div>
    );
  if (!data)
    return (
      <div className="rounded border border-border/20 bg-muted/5 p-3 text-[11px] text-muted-foreground">
        Loading capture…
      </div>
    );

  const isReq = tab === "request";
  const headers = isReq ? data.request.headers : data.response.headers;
  const contentType = isReq
    ? (data.request.headers["content-type"] ?? "")
    : data.response.contentType;
  const body = isReq ? (data.request.body ?? "") : data.response.bodyTextSample;

  return (
    <div className="rounded border border-border/20 bg-muted/5">
      <div className="flex items-center gap-1 border-b border-border/15 px-2 py-1.5">
        {(["request", "response"] as const).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTab(t)}
            className={cn(
              "rounded px-2 py-0.5 text-[10px]",
              tab === t ? "bg-muted/30 text-foreground" : "text-muted-foreground hover:bg-muted/20"
            )}
          >
            {t === "request" ? "Request" : "Response"}
          </button>
        ))}
        {!isReq && data.response.status != null && (
          <span className="ml-auto font-mono text-[10px] text-green-300">
            {data.response.status}
          </span>
        )}
      </div>
      <div className="space-y-2 p-2.5">
        <div className="font-mono text-[10px] text-foreground/80 break-all">
          {data.request.method} {data.request.url}
        </div>
        <div className="rounded bg-background/25 px-2 py-1.5">
          <p className="text-[9px] uppercase text-muted-foreground">Headers</p>
          <HeaderTable headers={headers} />
        </div>
        <div className="rounded bg-background/25 px-2 py-1.5">
          <p className="text-[9px] uppercase text-muted-foreground">Body</p>
          {body ? (
            <pre className="mt-1 max-h-72 overflow-auto whitespace-pre-wrap break-all font-mono text-[10px] text-foreground/80">
              {prettyBody(bodyRenderMode(contentType), body)}
            </pre>
          ) : (
            <p className="mt-1 text-[10px] text-muted-foreground">
              {isReq ? "未捕获（重新采集以填充）" : "No body sample stored."}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
```

**验证：** `pnpm test:run -- frontend/components/TargetPanel`（现有用例不破）+ `just check-fe`（biome+tsc 绿）。

**提交：** `feat(inspector): request/response viewer component`

---

## 阶段 E：接入 SitemapTab endpoint detail

### 任务 E.1：endpoint 详情下方挂 Inspector

**文件：** `frontend/components/TargetPanel/surface/tabs/SitemapTab.tsx`

**步骤：** `SitemapTab` 已有 `items/jsResults`，需要 `projectPath`：从 `TargetSurfaceWorkbench` 透传（它已能拿到当前 target/project；若无现成 projectPath，复用 oplog 取 projectPath 的同一来源）。在 `EndpointDetail` 的 `<JsSourceList />` 之后插入：

```tsx
        <Inspector projectPath={projectPath} capturePath={item.capturePath} />
```

并在 `DetailPanel`/`SitemapTab` 增加 `projectPath` 透传参数；`TargetSurfaceWorkbench` 渲染 `<SitemapTab ... projectPath={projectPath} />`。import：`import { Inspector } from "../Inspector/Inspector";`

**验证：** `pnpm test:run -- frontend/components/TargetPanel` + `just check-fe`（绿）。

**提交：** `feat(sitemap): show request/response Inspector for selected endpoint`

---

## 阶段 F：门禁

```bash
node --check scripts/browser_collect_js_api.mjs
cd backend && cargo nextest run -p golish-pentest-app --status-level fail
cd backend && cargo clippy -p golish-pentest-app -p golish --all-targets -- -D warnings
pnpm test:run -- frontend
just check-fe
```
全绿后更新 `docs/modules/...`（pentest_bridge 卡 + frontend components 卡）、`feature_list.json`（新增 `surface-inspector`）、`agent-progress.md`（证据）。

---

## 自检（对照设计 §5.1 / §6 / §8 / §10）

- **规格覆盖：** 设计 §5.1 Inspector → A（capture v2）+B（read_capture）+C（wrapper）+D（组件）+E（接入）。§4 capture v2 → A。§8 路径守卫(I3) → B.1。后续 §5.2/§5.3/§5.4 不在本计划（独立计划）。
- **占位符扫描：** 无 TODO/后续实现；每步有代码或精确命令。唯一柔性点：B.1 `GolishError::msg` 若不存在则照本文件现有错误构造（已注明如何处理）。
- **类型一致：** `CapturePayload/CaptureRequest/CaptureResponse`（C.1）→ Inspector（D.2）使用一致；`readCapture(projectPath, capturePath)`（C.1）→ E.1 调用一致；`pentest_read_capture(project_path, capture_path)`（B.2）↔ 前端 `{ projectPath, capturePath }`（C.1）键名映射一致；`bodyRenderMode/prettyBody`（D.1）→ D.2 使用一致。
