# P0-1 端到端错误码契约（I1）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans/` 逐任务实现此计划；每个任务单独 commit。本计划只新增/修改业务代码与测试，不动 `frontend/lib/generated/`。
> Relates to: `docs/design/2026-05-29-architecture-optimization.md` §4.1 / §5 P0-1（及 P0-4 的「补 error 态 / 消裸 invoke」）；AGENTS.md 不变量 I1（错误统一带 `code`，前端按 map 翻译）、§2.3（三态 UI）。

**目标：** 后端 `GolishError` 序列化为稳定的 `{ code, message }`；前端 `ApiError` 解析出 `code`，并通过统一的 `code → 文案` map 翻译；`FindingsPanel` / `PipelinePanel` / `ProjectOverview` 三条吞错路径补齐 error 态 UI；顺带消除 `PipelinePanel` 的裸 `invoke("pipeline_list")`。

**架构：** 后端给 `GolishError` 增加一个返回 `&'static str` 的 `code()` 方法（每个变体一个稳定码），并把手写的 `Serialize` 从「裸字符串」改为「`{ code, message }` 结构体」（`message` 仍保留，叠加 `code`，对人类可读消息零损失）。前端**先**把 `client.ts` 的 `ApiError` 改造为「同时兼容旧裸字符串与新 `{code,message}` 两种形状」（向后兼容，先落地），**再**改后端序列化形状，从而保证每个 commit 应用都可用、无中间损坏态。前端新增 `lib/api/error-codes.ts` 作为后端 `code()` 的镜像清单 + `code→文案` map（ts-rs 自动生成留待 P0-2，本计划手维护并加交叉引用注释）。三个面板按 `AsyncView` 既有红色 error 样式新增一个 error 分支（surgical，不拆除既有 loading/empty 标记）。

**技术栈：** Rust（`serde::Serialize` 手写实现、`thiserror`、`serde_json`、`cargo nextest`）、TypeScript（Vitest、`@tauri-apps/api/core`）、React 19、lucide-react、`just`、biome、git。

---

## 现状（事实，带证据）

- **后端序列化丢 `code`**：`backend/crates/golish/src/error.rs:133-139` 的 `impl Serialize for GolishError` 仅 `serializer.serialize_str(&self.to_string())`，输出**裸字符串**，无 `code`。
- **`GolishError` 有 16 个变体**（`error.rs:19-70`）：`Io` / `Database` / `Json` / `Http` / `Pty` / `Tool` / `Skills` / `Pentest` / `VulnIntel` / `Pipeline` / `ScanRunner` / `SessionNotFound` / `NotFound` / `Validation` / `Config` / `Internal`。**无 `#[cfg(test)]`**（全文件 grep `mod tests` 0 命中）——序列化无单测。
- **前端 `ApiError` 不解析 `code`**：`frontend/lib/api/client.ts:9-19` 的 `ApiError` 只存 `command` / `cause` / `traceId`，消息用 `cause instanceof Error ? cause.message : String(cause)`（`client.ts:15`）。一旦后端改为对象，`String(cause)` 会变成 `"[object Object]"` —— **这是必须先改前端、后改后端的根因**。
- **前端无错误码 map**：`frontend/lib/` 下无集中的 `code→文案` 文件（grep `errorCode|ErrorCode` 命中均为无关字段）。
- **并非所有命令都返回 `GolishError`**：`error.rs:9-12` 的迁移注释说明仍有命令返回 `Result<T, String>`，因此前端解析**必须**兼容裸字符串（降级为 `UNKNOWN`）。
- **三个面板吞错、无 error 态**：
  - `FindingsPanel.tsx`：`load()` 在 `78-88`，`catch { setFindings([]) }`（`83-84`）吞错；渲染 `457-469` 只有 loading（`457-460`）/ empty（`461-469`），**无 error 分支**。
  - `PipelinePanel.tsx`：`load()` 在 `104-119`，第 `108` 行**裸** `invoke<Pipeline[]>("pipeline_list", …)`（未走 `lib/api/pipeline.ts:11` 的 `listPipelines`），`catch { /* */ }`（`115-117`）吞错；渲染 `305-310` 只有 loading，empty 在 `339-343`，**无 error 分支**。
  - `ProjectOverview.tsx`：`fetchTargets()` 在 `39-50`，`catch (e) { logger.error; setTargets([]) }`（`44-46`）吞错；渲染 `180+` 只有 loading，**无 error 分支**。
- **可复用的三态组件已存在**：`frontend/components/ui/AsyncView.tsx:28-63`（loading/error/empty），error 样式为 `text-red-400/70`（`AsyncView.tsx:50`）。本计划复用其 error 视觉，但为保「行为最小变更」，面板内**只新增 error 分支**，不拆除既有 loading/empty 标记（AsyncView 全面收敛归 P1-6）。
- **`pipeline_list` 真实返回 `Vec<Pipeline>`**：`backend/crates/golish/src/tools/pipeline/commands.rs:122-125` 签名 `Result<Vec<Pipeline>, GolishError>`。而前端 wrapper `listPipelines` 声明返回 `PipelineSummary[]`（`lib/api/pipeline.ts:11-13`）——**类型漂移**：裸 invoke 处 `Pipeline[]` 的 cast 反而是对的。故本计划用 `listPipelines` 时需先把 wrapper 返回类型纠正为 `Pipeline[]`。
- **测试基建**：前端 Vitest（`import { describe, expect, it, vi } from "vitest"`，见 `frontend/lib/pathDetection.test.ts:1`）；`just test-fe` = `pnpm test:run`（`justfile:30-37`），`just test-rust` = `cargo nextest run --status-level fail`（`justfile:56-63`）。

---

## 文件结构（创建 / 修改 + 职责）

| 文件 | 动作 | 职责 |
|---|---|---|
| `frontend/lib/api/error-codes.ts` | 新建 | 后端 `code()` 的镜像清单 `GOLISH_ERROR_CODES` + 类型 `GolishErrorCode` + `code→文案` map + `translateErrorCode()` + `UNKNOWN` 兜底 |
| `frontend/lib/api/error-codes.test.ts` | 新建 | 校验已知 code 有文案、未知 code 走 fallback |
| `frontend/lib/api/client.ts` | 修改 | 新增导出 `GolishErrorShape` + `parseGolishError()`；`ApiError` 增 `readonly code`，消息用解析后的 `message`（兼容裸字符串/对象两形状） |
| `frontend/lib/api/client.test.ts` | 新建 | 校验 `parseGolishError` 两形状解析 + `ApiError.code`/message |
| `backend/crates/golish/src/error.rs` | 修改 | 新增 `pub fn code(&self) -> &'static str`；`Serialize` 改为输出 `{ code, message }`；新增 `#[cfg(test)] mod tests` |
| `frontend/lib/api/pipeline.ts` | 修改 | `listPipelines` 返回类型从 `PipelineSummary[]` 纠正为 `Pipeline[]`（对齐后端 `Vec<Pipeline>`） |
| `frontend/components/FindingsPanel/FindingsPanel.tsx` | 修改 | 增 `error` state + catch 写入 + 渲染 error 分支 |
| `frontend/components/PipelinePanel/PipelinePanel.tsx` | 修改 | 裸 invoke → `listPipelines`；增 `error` state + catch 写入 + 渲染 error 分支 |
| `frontend/components/ProjectOverview/ProjectOverview.tsx` | 修改 | 增 `error` state + catch 写入 + 渲染 error 分支 |

> **DRY / YAGNI**：`code→文案` 文案与 `code()` 清单各一份权威源（后端 `code()` 为机器码权威，前端 `error-codes.ts` 为镜像 + i18n 文案），用注释互相交叉引用；不引入运行时反射或第三方 i18n 框架。三个面板用同一 error-分支模式，但因 props/上下文不同，逐个手改，不强抽公共组件（归 P1-6）。

---

## 任务分解（小步骤）

> **执行顺序铁律（向后兼容）**：先做任务 1-3（前端兼容两形状）→ 再做任务 4-5（后端改形状）。这样任一 commit 处，前端都能正确解析「旧裸字符串」与「新对象」，应用无中间损坏态。

### 任务 1：新增前端错误码清单与文案 map

- **文件：** `frontend/lib/api/error-codes.ts`（新建）
- **步骤：**
  1. 写入清单、类型、文案 map 与翻译函数（`GOLISH_ERROR_CODES` 必须与后端 `error.rs` 的变体逐一对应）：

```ts
/**
 * Canonical Golish error codes — MIRROR of backend `GolishError::code()`
 * (backend/crates/golish/src/error.rs). Keep both in sync until P0-2's
 * ts-rs generation can derive this list automatically.
 */
export const GOLISH_ERROR_CODES = [
  "IO",
  "DATABASE",
  "JSON",
  "HTTP",
  "PTY",
  "TOOL",
  "SKILLS",
  "PENTEST",
  "VULN_INTEL",
  "PIPELINE",
  "SCAN_RUNNER",
  "SESSION_NOT_FOUND",
  "NOT_FOUND",
  "VALIDATION",
  "CONFIG",
  "INTERNAL",
] as const;

export type GolishErrorCode = (typeof GOLISH_ERROR_CODES)[number];

/** Frontend-only fallback when the backend error is not in {code,message} shape. */
export const UNKNOWN_ERROR_CODE = "UNKNOWN";

const MESSAGES: Record<GolishErrorCode | typeof UNKNOWN_ERROR_CODE, string> = {
  IO: "A file or I/O operation failed.",
  DATABASE: "A database operation failed.",
  JSON: "Failed to read or encode data.",
  HTTP: "A network request failed.",
  PTY: "A terminal session error occurred.",
  TOOL: "A tool operation failed.",
  SKILLS: "A skill operation failed.",
  PENTEST: "A pentest operation failed.",
  VULN_INTEL: "A vulnerability-intel operation failed.",
  PIPELINE: "A pipeline operation failed.",
  SCAN_RUNNER: "A scan failed to run.",
  SESSION_NOT_FOUND: "The session was not found.",
  NOT_FOUND: "The requested item was not found.",
  VALIDATION: "The input was invalid.",
  CONFIG: "There is a configuration problem.",
  INTERNAL: "An unexpected error occurred.",
  UNKNOWN: "An unexpected error occurred.",
};

/**
 * Translate a backend error `code` into a user-facing message. Falls back to
 * the raw backend `message` when the code is unknown, then to a generic line.
 */
export function translateErrorCode(code: string, fallbackMessage?: string): string {
  if (code in MESSAGES) {
    return MESSAGES[code as GolishErrorCode];
  }
  return fallbackMessage && fallbackMessage.length > 0 ? fallbackMessage : MESSAGES.UNKNOWN;
}
```

- **验证：** `cd frontend && pnpm exec tsc --noEmit`（exit 0，无类型错误）。
- **提交：** `feat(api): add Golish error-code list + translation map`

### 任务 2：为错误码 map 写单测

- **文件：** `frontend/lib/api/error-codes.test.ts`（新建）
- **步骤：**
  1. 写入测试：

```ts
import { describe, expect, it } from "vitest";
import { GOLISH_ERROR_CODES, translateErrorCode } from "./error-codes";

describe("translateErrorCode", () => {
  it("returns a message for every canonical code", () => {
    for (const code of GOLISH_ERROR_CODES) {
      const msg = translateErrorCode(code);
      expect(msg.length).toBeGreaterThan(0);
      expect(msg).not.toBe("");
    }
  });

  it("falls back to the raw backend message for unknown codes", () => {
    expect(translateErrorCode("SOMETHING_NEW", "raw backend text")).toBe("raw backend text");
  });

  it("falls back to a generic line when unknown and no raw message", () => {
    expect(translateErrorCode("SOMETHING_NEW")).toBe("An unexpected error occurred.");
  });
});
```

- **验证：** `cd frontend && pnpm test:run -- error-codes`（预期：3 tests passed）。
- **提交：** `test(api): cover error-code translation map`

### 任务 3：`client.ts` 解析 `code`，兼容两形状（先于后端落地）

- **文件：** `frontend/lib/api/client.ts`（修改）、`frontend/lib/api/client.test.ts`（新建）
- **步骤：**
  1. 在 `client.ts` 顶部（`ApiError` 定义之前）新增类型与解析函数：

```ts
export interface GolishErrorShape {
  code: string;
  message: string;
}

/**
 * Tauri rejects command promises with whatever the Rust side serialized.
 * After P0-1, `GolishError` serializes to `{ code, message }`; legacy commands
 * that still return `Result<T, String>` reject with a bare string, and JS
 * runtime failures reject with an `Error`. Normalize all three into
 * `{ code, message }` so callers can branch on a stable `code`.
 */
export function parseGolishError(cause: unknown): GolishErrorShape {
  if (cause && typeof cause === "object" && "code" in cause && "message" in cause) {
    const c = cause as Record<string, unknown>;
    if (typeof c.code === "string" && typeof c.message === "string") {
      return { code: c.code, message: c.message };
    }
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  return { code: "UNKNOWN", message };
}
```

  2. 把 `ApiError` 改为持有 `code` 并用解析后的 `message`（替换 `client.ts:9-19` 的整块）：

```ts
export class ApiError extends Error {
  /** Stable backend error code (see lib/api/error-codes.ts), or "UNKNOWN". */
  public readonly code: string;

  constructor(
    public readonly command: string,
    public readonly cause: unknown,
    public readonly traceId: string
  ) {
    const { code, message } = parseGolishError(cause);
    super(`[API trace=${traceId}] ${command}: ${message}`);
    this.name = "ApiError";
    this.code = code;
  }
}
```

  3. 新建 `frontend/lib/api/client.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { ApiError, parseGolishError } from "./client";

describe("parseGolishError", () => {
  it("extracts code+message from a structured GolishError", () => {
    expect(parseGolishError({ code: "NOT_FOUND", message: "Not found: x" })).toEqual({
      code: "NOT_FOUND",
      message: "Not found: x",
    });
  });

  it("falls back to UNKNOWN for legacy bare-string errors", () => {
    expect(parseGolishError("boom")).toEqual({ code: "UNKNOWN", message: "boom" });
  });

  it("falls back to UNKNOWN for Error instances", () => {
    expect(parseGolishError(new Error("kaboom"))).toEqual({
      code: "UNKNOWN",
      message: "kaboom",
    });
  });
});

describe("ApiError", () => {
  it("exposes the parsed code and threads traceId + command + message", () => {
    const e = new ApiError("pipeline_list", { code: "PIPELINE", message: "boom" }, "ab12cd34");
    expect(e.code).toBe("PIPELINE");
    expect(e.message).toContain("ab12cd34");
    expect(e.message).toContain("pipeline_list");
    expect(e.message).toContain("boom");
  });

  it("does not stringify object cause as [object Object]", () => {
    const e = new ApiError("x_cmd", { code: "INTERNAL", message: "real text" }, "ffffffff");
    expect(e.message).not.toContain("[object Object]");
    expect(e.message).toContain("real text");
  });
});
```

- **验证：** `cd frontend && pnpm exec tsc --noEmit` 且 `pnpm test:run -- client`（预期：5 tests passed，含 `[object Object]` 防回归）。
- **提交：** `feat(api): parse structured {code,message} errors in ApiError`

### 任务 4：后端 `GolishError::code()` 方法 + 单测

- **文件：** `backend/crates/golish/src/error.rs`（修改）
- **步骤：**
  1. 在已有的 `impl GolishError { … }` 块（`error.rs:72-77`）内，`from_anyhow` 之后新增 `code()`：

```rust
    /// Stable, machine-readable error code for the IPC boundary.
    /// MIRROR of `frontend/lib/api/error-codes.ts`; keep both in sync.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "IO",
            Self::Database(_) => "DATABASE",
            Self::Json(_) => "JSON",
            Self::Http(_) => "HTTP",
            Self::Pty(_) => "PTY",
            Self::Tool(_) => "TOOL",
            Self::Skills(_) => "SKILLS",
            Self::Pentest(_) => "PENTEST",
            Self::VulnIntel(_) => "VULN_INTEL",
            Self::Pipeline(_) => "PIPELINE",
            Self::ScanRunner(_) => "SCAN_RUNNER",
            Self::SessionNotFound(_) => "SESSION_NOT_FOUND",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Validation(_) => "VALIDATION",
            Self::Config(_) => "CONFIG",
            Self::Internal(_) => "INTERNAL",
        }
    }
```

  2. 在文件末尾（`error.rs:147` `pub type Result` 之后）新增测试模块（此任务仅测 `code()`，序列化测试在任务 5 加）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_stable_per_variant() {
        assert_eq!(GolishError::NotFound("x".into()).code(), "NOT_FOUND");
        assert_eq!(GolishError::Validation("x".into()).code(), "VALIDATION");
        assert_eq!(GolishError::Config("x".into()).code(), "CONFIG");
        assert_eq!(GolishError::Internal("x".into()).code(), "INTERNAL");
        assert_eq!(
            GolishError::SessionNotFound("s".into()).code(),
            "SESSION_NOT_FOUND"
        );
    }
}
```

- **验证：** `cd backend && cargo nextest run -p golish error::tests`（预期：`code_is_stable_per_variant` 通过）。若 `code()` 漏写某变体，`match` 非穷尽会**编译失败**——这是设计内的护栏。
- **提交：** `feat(error): add stable code() per GolishError variant`

### 任务 5：后端 `Serialize` 改为 `{ code, message }` + 序列化单测

- **文件：** `backend/crates/golish/src/error.rs`（修改）
- **步骤：**
  1. 替换 `Serialize` 实现（`error.rs:133-140`）：

```rust
impl Serialize for GolishError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // I1: emit a stable { code, message } envelope. `message` is retained
        // (still human-readable) so older string consumers degrade gracefully;
        // `code` lets the frontend branch via lib/api/error-codes.ts.
        let mut state = serializer.serialize_struct("GolishError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}
```

  2. 在任务 4 新建的 `mod tests` 内追加序列化测试：

```rust
    #[test]
    fn serializes_with_code_and_message() {
        let err = GolishError::NotFound("widget 42".to_string());
        let v = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "NOT_FOUND");
        assert_eq!(v["message"], "Not found: widget 42");
    }

    #[test]
    fn validation_serializes_with_validation_code() {
        let err = GolishError::Validation("bad input".to_string());
        let v = serde_json::to_value(&err).expect("serialize");
        assert_eq!(v["code"], "VALIDATION");
        assert_eq!(v["message"], "Validation error: bad input");
    }
```

  3. 确认 `serde_json` 在 `golish` crate 的 `[dev-dependencies]` 或 `[dependencies]` 可用（`error.rs` 已用 `serde`；`serde_json` 是 workspace 常见依赖）。若测试编译报 `serde_json` 不可用，则在 `backend/crates/golish/Cargo.toml` 的 `[dev-dependencies]` 加 `serde_json = { workspace = true }`。
- **验证：** `cd backend && cargo nextest run -p golish error::tests`（预期：`serializes_with_code_and_message` + `validation_serializes_with_validation_code` 通过）。
- **提交：** `feat(error): serialize GolishError as {code,message} (I1)`

### 任务 6：纠正 `listPipelines` 返回类型漂移

- **文件：** `frontend/lib/api/pipeline.ts`（修改）
- **步骤：**
  1. 先 grep 确认改类型不会波及其它调用方：`rg "listPipelines\(" frontend`（预期当前仅本计划任务 7 会用；若有其它消费 `PipelineSummary` 的调用方，在本任务一并适配）。
  2. 后端 `pipeline_list` 返回 `Vec<Pipeline>`（`commands.rs:122-125`），故纠正 wrapper（`pipeline.ts:11-13`）：

```ts
import type { Pipeline, PipelineSummary } from "../pentest/pipeline-types";
import { invoke } from "./client";

export async function listPipelines(projectPath: string | null): Promise<Pipeline[]> {
  return invoke<Pipeline[]>("pipeline_list", { projectPath });
}
```

  3. 若 `PipelineSummary` 在本文件改后变为未使用 import，删除它以过 biome（保持 import 干净）。
- **验证：** `cd frontend && pnpm exec tsc --noEmit`（exit 0）且 `pnpm exec biome check lib/api/pipeline.ts`（无未用 import 告警）。
- **提交：** `fix(api): correct listPipelines return type to Pipeline[]`

### 任务 7：`PipelinePanel` 消裸 invoke + 补 error 态

- **文件：** `frontend/components/PipelinePanel/PipelinePanel.tsx`（修改）
- **步骤：**
  1. 在从 `@/lib/api/pipeline` 的既有 import（当前含 `savePipeline` / `savePipelineTemplate` / `listPipelineTemplates`）中**加上** `listPipelines`。
  2. 从 `@/lib/api/client` import `ApiError`，从 `@/lib/api/error-codes` import `translateErrorCode`；从 `lucide-react` 既有 import 行加上 `AlertTriangle`。
  3. 在 `loading` state 声明旁（`PipelinePanel.tsx:77`）新增：

```tsx
  const [error, setError] = useState<string | null>(null);
```

  4. 改写 `load`（`PipelinePanel.tsx:104-119`）——裸 invoke → `listPipelines`，并把 `catch` 写入 error：

```tsx
  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [pl, tl, ai] = await Promise.all([
        listPipelines(getProjectPath()),
        scanTools(),
        listAiTools().catch(() => [] as AiToolMeta[]),
      ]);
      setPipelines(Array.isArray(pl) ? pl : []);
      setTools((tl?.tools || []).filter((t) => t.launchMode === "cli" && t.installed));
      setAiTools(Array.isArray(ai) ? ai : []);
    } catch (e) {
      setError(translateErrorCode(e instanceof ApiError ? e.code : "UNKNOWN", e instanceof Error ? e.message : undefined));
      setPipelines([]);
    } finally {
      setLoading(false);
    }
  }, []);
```

  5. 在渲染处的 loading 早返回（`PipelinePanel.tsx:305-310`）之前，新增 error 早返回：

```tsx
  if (error)
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-2 text-red-400/70">
        <AlertTriangle className="w-5 h-5" />
        <p className="text-[11px]">{error}</p>
      </div>
    );
```

- **验证：** `cd frontend && pnpm exec tsc --noEmit`；`rg "invoke\(\"pipeline_list\"" frontend/components` 应**0 命中**；`pnpm test:run -- PipelinePanel`（若无该测试文件则跑 `pnpm test:run` 确认无连带破坏）。
- **提交：** `fix(pipeline): route list through api layer + add error state (P0-4)`

### 任务 8：`FindingsPanel` 补 error 态

- **文件：** `frontend/components/FindingsPanel/FindingsPanel.tsx`（修改）
- **步骤：**
  1. import：`ApiError`（`@/lib/api/client`）、`translateErrorCode`（`@/lib/api/error-codes`）、`AlertTriangle`（既有 lucide-react 行已含 `Loader2` / `Bug`，追加）。
  2. 在 `loading` state（`FindingsPanel.tsx:65`）旁新增：

```tsx
  const [error, setError] = useState<string | null>(null);
```

  3. 改写 `load`（`FindingsPanel.tsx:78-88`）：

```tsx
  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const store = await findingsApi.list(getProjectPath());
      setFindings(store?.findings || []);
    } catch (e) {
      setError(translateErrorCode(e instanceof ApiError ? e.code : "UNKNOWN", e instanceof Error ? e.message : undefined));
      setFindings([]);
    } finally {
      setLoading(false);
    }
  }, []);
```

  4. 在渲染的 findings 列表三态（`FindingsPanel.tsx:457`）最前面加 error 分支（保持既有 loading/empty 不动）：

```tsx
        {error ? (
          <div className="flex flex-col items-center justify-center h-32 gap-2 text-red-400/70">
            <AlertTriangle className="w-8 h-8" />
            <p className="text-[11px]">{error}</p>
          </div>
        ) : loading ? (
          // …既有 loading 块不变…
```

  > 即把现有 `{loading ? (…) : filtered.length === 0 ? (…) : (…)}` 包成 `{error ? (errorBlock) : loading ? (…) : …}`，仅新增一层三元，不改既有两支。

- **验证：** `cd frontend && pnpm exec tsc --noEmit`；`pnpm test:run`（确认无连带破坏）。
- **提交：** `fix(findings): add error state to findings list (P0-4)`

### 任务 9：`ProjectOverview` 补 error 态

- **文件：** `frontend/components/ProjectOverview/ProjectOverview.tsx`（修改）
- **步骤：**
  1. import：`ApiError`、`translateErrorCode`、`AlertTriangle`（lucide-react）。
  2. 在 `loading` state（`ProjectOverview.tsx:35`）旁新增：

```tsx
  const [error, setError] = useState<string | null>(null);
```

  3. 改写 `fetchTargets`（`ProjectOverview.tsx:39-50`）——保留既有 `logger.error`，叠加 error state：

```tsx
  const fetchTargets = useCallback(async () => {
    try {
      const pp = getProjectPath();
      const data = await targetsApi.listTargets(pp);
      setTargets(data.targets);
      setError(null);
    } catch (e) {
      logger.error("[ProjectOverview] fetchTargets failed:", e);
      setError(translateErrorCode(e instanceof ApiError ? e.code : "UNKNOWN", e instanceof Error ? e.message : undefined));
      setTargets([]);
    } finally {
      setLoading(false);
    }
  }, []);
```

  4. 在渲染的 loading 分支（`ProjectOverview.tsx:180`）前加 error 分支（同 FindingsPanel 模式，最小新增）：

```tsx
            {error ? (
              <div className="flex flex-col items-center justify-center gap-2 text-red-400/70 py-8">
                <AlertTriangle className="w-6 h-6" />
                <p className="text-[11px]">{error}</p>
              </div>
            ) : loading ? (
              // …既有 loading 块不变…
```

- **验证：** `cd frontend && pnpm exec tsc --noEmit`；`pnpm test:run`。
- **提交：** `fix(overview): add error state to targets fetch (P0-4)`

### 任务 10：全套门禁 + 手测

- **文件：** 无（仅验证）
- **步骤：**
  1. 后端：`cd backend && cargo nextest run --status-level fail`（或 `just test-rust`）。
  2. 后端 lint：`just lint-rust`（clippy 零 warning）。
  3. 前端：`just check-fe`（biome + tsc）+ `just test-fe`（Vitest 全绿）。
  4. 全门禁：`just precommit`（= `just check` + `just test`）全绿。
  5. **手测（I1 端到端）**：`just dev` 启动；故意制造一个 `NotFound`（例如对不存在的 finding/pipeline 触发删除/读取），在浏览器 devtools 确认 IPC 拒绝体为 `{ code: "NOT_FOUND", message: "Not found: …" }`，且 FindingsPanel/PipelinePanel/ProjectOverview 在数据加载失败时显示红色 error 文案（非白屏）。把命令、退出码、关键输出片段记入 `agent-progress.md`「已记录证据」。
- **验证：** 上述命令退出码均为 0；手测截图/日志记入 progress。
- **提交：** 无新增（前序任务已分别 commit）；若 precommit 触发格式化改动，单独 `chore: precommit formatting`。

---

## 影响面

- **后端**：仅 `golish/src/error.rs`（新增 `code()` + 改 `Serialize` + 测试）。**所有返回 `GolishError` 的命令**的错误 JSON 形状从「字符串」变为「`{code,message}`」——这是契约层变更，但因前端先兼容（任务 3），无运行期断裂。
- **前端**：新增 `error-codes.ts` + 2 个测试文件；`client.ts` `ApiError` 增字段；`pipeline.ts` 纠类型；3 个面板各加 error 分支。
- **不影响**：Tauri 命令签名/命名、DB schema、ts-rs 生成目录、业务语义（normalize / pipeline 协议等）。

## 验证

| 命令 | 预期 |
|---|---|
| `cd backend && cargo nextest run -p golish error::tests` | `code()` + 序列化测试通过 |
| `just lint-rust` | clippy 零 warning |
| `cd frontend && pnpm test:run -- error-codes client` | 错误码 map + ApiError 解析测试通过 |
| `rg "invoke\(\"pipeline_list\"" frontend/components` | 0 命中（裸 invoke 已消除） |
| `just check-fe` | biome + tsc 通过 |
| `just test-fe` | Vitest 全绿、无连带破坏 |
| `just precommit` | 合并前全绿门禁（AGENTS.md §2.6） |
| 手测制造 `NotFound` | devtools 见 `{code,message}`；三面板显示红色 error 态而非白屏 |

**实施约定（工程效率，来源：用户要求 2026-05-29；设计文档 §7.1）：** 后端改动（任务 4-5）采用「**批量改完 → 统一编译 → 批量修错**」——把 `code()`、`Serialize`、测试一次性全部写完，再**只**统一跑一次 `cargo check`（或 `just check-rust`），集中批量修编译错误；不要每改一处就编译。最终合并仍以 `just precommit` 全绿为准。

## 回滚

- **纯增量叠加**：`code` 是在 `message` 之上叠加的字段；前端 `parseGolishError` 同时兼容旧裸字符串，故后端序列化即使回退到字符串，前端仍正确（降级 `UNKNOWN`）。
- **逐任务单 commit**，可独立 revert：回滚顺序为「面板 error 分支 → pipeline 类型 → 后端 Serialize → 后端 code() → client.ts → error-codes.ts」。
- 因前端先兼容、后端后改形状，**任一中间 commit 应用均可用**，无需「整批回滚」。

## 风险

| 风险 | 缓解 |
|---|---|
| 后端改对象形状但前端未先兼容 → `[object Object]` 白屏 | **严格按任务 1-3 先于 4-5 的顺序**；任务 3 测试含 `[object Object]` 防回归断言 |
| `code()` 漏某变体 | `match` 非穷尽会编译失败（Rust 护栏）；任务 4 测试覆盖代表变体 |
| 前后端码表漂移 | 两侧注释互相交叉引用；`GOLISH_ERROR_CODES` 与 `code()` 变体一一对应；P0-2 ts-rs 落地后可自动生成消除手维护 |
| `listPipelines` 改类型波及其它调用方 | 任务 6 先 `rg "listPipelines\("` 排查，一并适配 |
| 与正在活跃的 asset_intel / TargetPanel 改动冲突 | 本计划只碰 `error.rs` + `lib/api/{client,error-codes,pipeline}` + 3 个面板，均不在 asset_intel 活跃面；小步提交、频繁 rebase |
| 面板 error 文案用后端原文可能泄漏内部细节 | `translateErrorCode` 优先用 `code→文案`，仅未知码才回退后端 `message`；敏感变体（如 `INTERNAL`）已映射为通用文案 |

---

## 自检

**1. 规格覆盖度（对照设计文档 §4.1 / §5 P0-1 + P0-4）：**
- 「`GolishError` 序列化为 `{ code, message }`」→ 任务 4（`code()`）+ 任务 5（`Serialize`）。
- 「前端按 `code` map 翻译」→ 任务 1（map）+ 任务 3（`ApiError.code` 解析）。
- 「所有读 `error.message` 的面板」→ `ApiError` 仍提供可读 `message`（任务 3），向后兼容。
- 「error 序列化单测 / ApiError 映射单测」→ 任务 5 / 任务 2+3。
- 「手动制造 `NotFound` 看前端按 code 渲染」→ 任务 10 手测。
- P0-4「消 `PipelinePanel.tsx:108` 裸 invoke → `listPipelines`」→ 任务 6+7。
- P0-4「为 Findings/Pipeline/ProjectOverview 补 error 态」→ 任务 7/8/9。
- 回滚「叠加字段、旧字符串匹配可并存」→ 「回滚」节 + 任务 3 兼容逻辑。

**2. 占位符扫描：** 无「TODO / 待定 / 类似任务 N / 添加适当错误处理」等空泛措辞；每个代码步骤均有完整代码块。

**3. 类型一致性：** 后端 `code()` 返回值与前端 `GOLISH_ERROR_CODES` 字面量逐一对应（`IO/DATABASE/JSON/HTTP/PTY/TOOL/SKILLS/PENTEST/VULN_INTEL/PIPELINE/SCAN_RUNNER/SESSION_NOT_FOUND/NOT_FOUND/VALIDATION/CONFIG/INTERNAL`）；`parseGolishError` / `translateErrorCode` / `ApiError.code` 命名在任务 1/3/7/8/9 间一致；`listPipelines` 返回 `Pipeline[]` 在任务 6 定义、任务 7 消费，类型一致。
