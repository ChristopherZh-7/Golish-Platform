# 通用凭据抓取器 (Credential Capture Engine) · 架构设计文档

> 日期：2026-05-21
> 状态：Draft（待用户审核）
> 关联：
> - `docs/design/2026-05-21-integrations.md`（Integrations 集成中心 · 本文档是其 P3 扩展）
> - `backend/crates/golish-integrations/src/schema.rs`（要扩 schema 加 `CaptureRecipe`）
> - `backend/crates/golish/src/tools/pentest/misc.rs`（已存在 `WebviewWindowBuilder` 用法，可参考）
> 分支：跟随 `integrations` 继续推进，不另开新分支
>
> **设计目标一句话**：把用户「点链接 → 打开浏览器 → 登录 → F12 拷 cookie → 粘回 Integrations」的 5 步缩成「点⚡按钮 → 在弹窗里登录 → 自动填值」的 2 步；且这套机制对**未来任意需要登录后拿凭据的 integration** 通用，新增一个 ≤ 改一份 JSON。

---

## 0. Why · 上下文与动机

### 0.1 上一轮 Integrations 的现状

`integrations` 让所有外部服务的 key/cookie/token 走 schema-driven 统一表单。但用户填值的来源仍然要手动复制粘贴：

| Integration | 拿 cookie/token 的当前流程 |
|---|---|
| ENScan_GO AQC (爱企查) | 1. 浏览器打开 `aiqicha.baidu.com` → 2. 登录 → 3. F12 → 4. Application → Cookies → 5. 找 `BDUSS` → 6. 拷 → 7. 回 Golish 粘进 textarea |
| ENScan_GO TYC (天眼查) | 同上，但要拷 3 个不同的字段：cookie + tycid + auth_token |
| FOFA | 1. 登录 → 2. 进 `fofa.info/personal/api` → 3. 点显示 API key → 4. 拷 → 5. 粘 |
| 0.zone | 1. 登录 → 2. 进 `0.zone/plug-in-unit` → 3. 拷 → 4. 粘 |
| GitHub Token | 1. 进 Settings → Developer settings → 2. 生成新 token → 3. 拷 → 4. 粘 |

**痛点**：

- 每个 cookie 1-7 天就失效，用户每周都要重复一次完整流程
- F12 路径在不同浏览器里 UI 都不一样，新手用户经常拷错字段（拷成 `_csrf` 而不是 `BDUSS`）
- 多字段（TYC 3 个 cookie）拷错或漏拷，工具直接报「未授权」用户却不知道哪里错
- 整套体验与「Golish 是一个 AI agentic 平台」的定位不符——用户期待 Golish 像个助手帮他拿 cookie，而不是让他自己 F12

### 0.2 用户诉求（2026-05-21 对话原话）

> 我刚刚看了一下 有没有一种办法是可以登陆自己抓的
> 这个我也想变成一个通用的你懂我意思吗 这个能是通用的吗 就是如果后续有些功能也需要这种登陆凭证的这种情况.

**目标**：通用机制，不只是 ENScan_GO；任何 integration 在 schema 里声明「我要抓 cookie X+Y / localStorage Z / page DOM W」，Golish 就给它出一个⚡按钮，点了就开窗让用户登录、自动拿值。

---

## 1. 核心决策

| 决策 | 选项 A（推荐 · 已选）| 选项 B | 选项 C |
|---|---|---|---|
| 1 · 浏览器来源 | **Tauri WebviewWindow**（项目已用 `WebviewWindowBuilder`） | 调用系统 Chrome/Edge 然后读 cookie 文件 | 用户继续 F12（现状） |
| 2 · 抓取触发点 | **schema 字段 `IntegrationGroup.capture: Option<CaptureRecipe>`** | 单独表 `capture_recipes` | 写在 Rust 代码里 |
| 3 · 抓取规则 | **4 种内置 `CaptureRule::*` enum**：Cookie / CookieJoined / LocalStorage / PageContent + UrlQuery | 通用 JS 表达式 | 仅 cookie |
| 4 · 完成信号 | **双获取**：自动（success_url_pattern 命中）+ 手动（用户点「抓取完成」按钮）| 仅自动 | 仅手动 |
| 5 · 后端 IPC | **3 个 command**：`integrations_capture_start` / `_status` / `_cancel` | 单 command + event-only | 全 event 不用 command |
| 6 · 抓到的值落地 | **直接写 vault**（不经前端）| 返给前端再走 `integrations_set` | 临时变量等用户确认 |
| 7 · 隔离性 | **每个 capture session 独立 webview**（不共享主窗 cookie）| 共享主窗 cookie | 复用系统浏览器 |
| 8 · TTL | **5 分钟**（足够登录 + 缓冲）| 30 秒 | 30 分钟 |
| 9 · 域名白名单 | **仅允许导航到 schema 声明的 `login_url` 同 origin** | 任意域名 | 主域名 + 子域名 |
| 10 · 失败回退 | **抓不到 → 弹「请手动复制」对话框 + 把 webview cookie/storage 列出来供选** | 抓不到直接报错 | 抓不到自动重试 |

---

## 2. 整体架构

```
                                    ┌─────────────────────────────────────────┐
                                    │  Settings → Integrations → AQC Card     │
                                    │                                          │
                                    │  [Cookie ▢▢▢▢▢▢▢▢▢▢]  (空)            │
                                    │                                          │
                                    │  [Save] [Test] [Clear]  [⚡ 从浏览器拖] │
                                    └─────────────┬───────────────────────────┘
                                                  │ 点击 ⚡
                                                  ▼
                              ┌────────────────────────────────────────┐
                              │  Confirm dialog                         │
                              │  即将打开 aiqicha.baidu.com 让你登录    │
                              │  登录后 Golish 会自动提取 cookie BDUSS  │
                              │  限时 5 分钟。                           │
                              │  [取消]  [开始]                          │
                              └────────────────┬───────────────────────┘
                                               │
                              integrations_capture_start ↓ IPC
                              ────────────────────────────
              ┌──────────────────────────────────────────────────────────┐
              │  Backend · CaptureEngine                                  │
              │                                                            │
              │  1. 解析 schema.capture                                    │
              │  2. WebviewWindowBuilder::new(label=capture-<uuid>)      │
              │       .url(login_url)                                     │
              │       .data_directory(独立 dir)  ←  关键：不污染主窗 cookie │
              │       .build()                                            │
              │  3. 监听 webview navigation + close events                │
              │  4. 启 background task：                                   │
              │     - 每 1s 检查 url 是否匹配 success_url_pattern         │
              │     - 命中 → 跑 CaptureRule[] 抠值                        │
              │     - 抠到 → 写 vault + emit("capture-done") + 关窗       │
              │     - 5 分钟没命中 → 强制关窗 emit("capture-timeout")     │
              │  5. 返 { session_id, expires_at } 给前端                  │
              └──────────────────────────┬───────────────────────────────┘
                                         │
                                         │ 弹窗里用户登录
                                         ▼
                              ┌──────────────────────────┐
                              │  Tauri Capture Window     │
                              │  https://aiqicha.baidu.com│
                              │  ▼ 用户在这里输入手机号/   │
                              │    密码 → 登录            │
                              │  ▼ 网页跳到 success URL    │
                              │  ✓ Engine 抓到 cookie     │
                              │  ✓ 自动关窗                │
                              └──────────────────────────┘
                                         │
                                         │  event: "integration-capture"
                                         │  payload: { session_id, status: "captured" }
                                         ▼
                              ┌──────────────────────────────────────┐
                              │  Frontend useCaptureSession hook      │
                              │  - 订阅 event                          │
                              │  - 收到 "captured" → 重新拉           │
                              │    integrations_get(tool, group)      │
                              │  - 表单里 cookie 字段从空变成「已配置」 │
                              └──────────────────────────────────────┘
```

---

## 3. Schema 扩展（golish-integrations）

### 3.1 `IntegrationGroup.capture` 字段

`backend/crates/golish-integrations/src/schema.rs` 中 `IntegrationGroup` 加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationGroup {
    // ... 现有字段 ...

    /// 可选：声明「点⚡按钮后如何从浏览器抓字段」
    /// None = 不显示⚡按钮，用户只能手填
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureRecipe>,
}
```

### 3.2 `CaptureRecipe`

```rust
/// 一份「自动从浏览器抓凭据」的食谱。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureRecipe {
    /// 用户要在这个 URL 登录。Tauri webview 打开它。
    pub login_url: String,

    /// 登录成功后浏览器会跳到的 URL（正则）。
    /// 命中即触发 rules[] 抓取。可选——不填则等用户手动点
    /// 「抓取完成」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_url_pattern: Option<String>,

    /// 抓之前要不要先导航到指定 URL。比如 FOFA 登录后跳 dashboard，
    /// 但 API key 在 /personal/api 页才显示——填 visit_url 让 engine 自动跳过去。
    /// 不填则在 success_url 所在页直接抓。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visit_url: Option<String>,

    /// 给用户看的中文说明（默认 i18n key `integrations.capture.<tool>.<group>.hint`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// 抓取窗口的 TTL。默认 300。最大 900。
    #[serde(default = "default_capture_timeout")]
    pub timeout_secs: u32,

    /// 按顺序执行的抓取规则。任一规则失败 → 整次抓取标记 partial，
    /// 但已成功的字段照样落 vault。
    pub rules: Vec<CaptureRule>,
}

fn default_capture_timeout() -> u32 { 300 }
```

### 3.3 `CaptureRule`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureRule {
    /// 从 webview cookie store 抓单个 cookie。
    Cookie {
        /// cookie 的 domain 限定（`.aiqicha.baidu.com` / `aiqicha.baidu.com`）。
        domain: String,
        /// cookie name（`BDUSS` / `STOKEN` / `tyc-user-info`）。
        name: String,
        /// 写到 schema 里哪个 field.key（必须与 fields[].key 之一匹配）。
        target_field: String,
        /// 抓不到时是否致命（true = 整次 capture 失败；false = 标 partial 继续）。
        #[serde(default = "default_true")]
        required: bool,
    },

    /// 抓多个 cookie 拼成一个值（典型：ENScan TYC 的 cookie 串需要
    /// `auth_token=xxx; tyc-user-info=yyy` 这种格式）。
    CookieJoined {
        domain: String,
        names: Vec<String>,
        /// cookie 间分隔符（默认 `"; "`）。
        #[serde(default = "default_cookie_sep")]
        sep: String,
        /// 每个 cookie 的输出格式（默认 `"{name}={value}"`，可改成 `"{value}"` 等）。
        #[serde(default = "default_cookie_fmt")]
        fmt: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// 从 webview 当前页的 localStorage 抓。
    LocalStorage {
        key: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// 从 webview 当前页的 sessionStorage 抓。
    SessionStorage {
        key: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// 从 webview 当前页的 DOM 抓元素内容（document.querySelector）。
    /// 用于 FOFA / 0.zone 等「API key 显示在某个 div 里」的场景。
    PageContent {
        /// CSS selector。
        selector: String,
        /// 取 textContent / innerText（默认 textContent）。
        /// 或取属性：`{ "attribute": "data-token" }` 取该元素 attr 值。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
        /// 抓之前要不要等元素出现（毫秒）。默认 3000。
        #[serde(default = "default_wait_ms")]
        wait_ms: u32,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },

    /// 抓当前 URL 的 query 参数（典型：OAuth callback `?code=xxx`）。
    UrlQuery {
        name: String,
        target_field: String,
        #[serde(default = "default_true")]
        required: bool,
    },
}

fn default_true() -> bool { true }
fn default_cookie_sep() -> String { "; ".to_string() }
fn default_cookie_fmt() -> String { "{name}={value}".to_string() }
fn default_wait_ms() -> u32 { 3000 }
```

### 3.4 现成例子：ENScan_GO AQC

`resources/toolsconfig/enscan-go.json` 中 aqc group 加 `capture`：

```jsonc
{
  "id": "aqc",
  "name": "爱企查 (AQC)",
  "fields": [
    { "key": "cookies.aqc", "type": "secret_textarea", "required": true }
  ],
  "test": { /* ... */ },
  "capture": {
    "login_url": "https://aiqicha.baidu.com",
    "success_url_pattern": "aiqicha\\.baidu\\.com/(home|company)",
    "timeout_secs": 300,
    "rules": [
      {
        "type": "cookie",
        "domain": ".aiqicha.baidu.com",
        "name": "BDUSS",
        "target_field": "cookies.aqc"
      }
    ]
  }
}
```

> 注：实际的 ENScan AQC cookie 抓哪个名字、tyc 的 3 字段哪些是 cookie 哪些是 localStorage——还需用户在 P1 实施阶段实测拍板。本文档先给 schema shape，留实测拍板。

---

## 4. 后端 API 契约

### 4.1 三个 IPC command

```
backend/crates/golish/src/tools/integrations/capture/commands.rs
    integrations_capture_start
    integrations_capture_status
    integrations_capture_cancel
```

#### `integrations_capture_start`

```
Req:
  { tool_id: String, group_id: String }
Res 200:
  {
    session_id: String,           // UUID v4
    login_url: String,            // echo from schema
    expected_keys: Vec<String>,   // target_field list, 供 UI 提示用户
    expires_at: i64               // unix ms
  }
Res 400 (CAPTURE_NO_RECIPE):
  schema 里没声明 capture，但前端却调了 → 前端有 bug
Res 404 (INTEGRATION_NOT_FOUND):
  tool_id/group_id 不存在
Res 409 (CAPTURE_ALREADY_RUNNING):
  同 (tool_id, group_id) 已有 inflight 的 session → UI 应先调 cancel
Res 500 (WEBVIEW_CREATE_FAILED):
  Tauri 创窗失败（罕见，比如平台不支持）
```

#### `integrations_capture_status`

轮询用。也可不用，订阅 `integration-capture` event 收推送。

```
Req:
  { session_id: String }
Res 200:
  {
    state: "waiting_login" | "captured" | "partial" | "timeout" | "cancelled" | "failed",
    captured_at: Option<i64>,
    captured_fields: Vec<String>,       // 哪些 target_field 已写入 vault
    failed_rules: Vec<{ rule_index, reason }>,
    error_message: Option<String>
  }
Res 404 (CAPTURE_SESSION_NOT_FOUND):
  session_id 不存在或已 TTL 过期清理
```

#### `integrations_capture_cancel`

```
Req:
  { session_id: String }
Res 200:
  { ok: true }
```

行为：立即关 webview + 清独立 cookie dir + state → cancelled。

### 4.2 Event：`integration-capture`

后端用 `app.emit("integration-capture", payload)` 推：

```ts
type IntegrationCaptureEvent = {
  session_id: string;
  tool_id: string;
  group_id: string;
  state: "waiting_login" | "captured" | "partial" | "timeout" | "cancelled" | "failed";
  captured_fields?: string[];
  failed_rules?: { rule_index: number; reason: string }[];
  error_message?: string;
};
```

前端 `useCaptureSession(sessionId)` hook 订阅 + 维护本地 state machine。

---

## 5. 引擎实现细节

### 5.1 CaptureEngine state machine

```
created
   │ start_capture()
   ▼
waiting_login        ◄────────────────┐
   │                                   │ 用户切走又切回，webview 仍开着
   │ navigation 到 success_url_pattern │
   ▼                                   │
visit_url? ──── yes ──► navigating ────┘
   │ no                                │
   ▼                                   │
extracting                             │
   │                                   │
   ├── all rules ok ────► captured ────► cleanup webview ──► done
   │
   ├── some rules ok ───► partial ─────► cleanup webview ──► done
   │   (写部分字段)
   │
   ├── critical rule failed ► failed ──► cleanup webview ──► done
   │
   └── 5 min 还在 waiting_login ► timeout ► cleanup webview ─► done

任意状态收到 cancel ──► cancelled ─► cleanup webview ─► done
```

### 5.2 WebviewWindow 设置

```rust
let label = format!("capture-{}", session_id);
let url = WebviewUrl::External(url::Url::parse(&recipe.login_url)?);
let win = WebviewWindowBuilder::new(&app, &label, url)
    .title(format!("Golish · 凭据抓取: {}", recipe.login_url))
    .inner_size(900.0, 700.0)
    .center()
    .focused(true)
    .visible(true)
    .data_directory(per_session_data_dir())  // ← 关键
    .build()?;
```

`per_session_data_dir()` 在 `~/Library/Application Support/com.golish.platform/capture-sessions/<session_id>/` 创建临时目录；session 结束删干净。

> Tauri 2 在 macOS/Windows/Linux 都支持 webview 数据目录隔离。Linux 走 webkit2gtk 的 ephemeral data manager（如果不行就用每次新 path）。

### 5.3 监听 navigation

```rust
let weak_app = app.clone();
let sid = session_id.clone();
win.on_navigation(move |new_url| {
    let app = weak_app.clone();
    let sid = sid.clone();
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<CaptureEngine>();
        engine.on_navigation(&sid, &new_url).await;
    });
    true  // allow navigation
});
```

`on_navigation` 内部：

```rust
async fn on_navigation(&self, sid: &str, url: &Url) {
    let session = self.sessions.read().get(sid).cloned();
    if let Some(s) = session {
        if let Some(pat) = &s.recipe.success_url_pattern {
            if Regex::new(pat).is_match(url.as_str()) {
                self.try_extract(sid).await;
            }
        }
    }
}
```

### 5.4 抓取实现（按 CaptureRule 分发）

#### Cookie / CookieJoined

```rust
// Tauri 2 webview.cookies_for_url(url) -> Vec<Cookie>
let cookies = win.cookies_for_url(domain).await?;
let cookie = cookies.iter().find(|c| c.name() == name);
```

#### LocalStorage / SessionStorage / PageContent / UrlQuery

只能通过 webview.eval_js(...) 注入 JS 拿：

```rust
let js = format!(r#"
  (function() {{
    return JSON.stringify({{
      ls: localStorage.getItem("{key}"),
      ss: sessionStorage.getItem("{key}"),
      sel: document.querySelector("{selector}")?.textContent ?? null,
      attr: document.querySelector("{selector}")?.getAttribute("{attr}") ?? null,
      url: location.href
    }});
  }})()
"#);
let result_json: String = win.eval_then_capture(js).await?;
```

> Tauri 2 默认 `eval()` 不返回值，需要走 IPC bridge。本设计在 capture webview 启动时注入一个轻量 bridge script（`__GOLISH_CAPTURE_BRIDGE__`），用 `window.__TAURI_INTERNALS__.postMessage(...)` 把 eval 结果回传。bridge script 不暴露任何 Tauri API（最小权限）。

### 5.5 直接写 vault（不经前端）

抓到 cleartext 立刻：

```rust
let backend = state.pick_backend(&schema)?;
backend.write(&tool_id, &group_id, fields_map).await?;
```

这样 cleartext 在前端从未出现过 —— 最小化暴露面。

---

## 6. 前端 UX

### 6.1 IntegrationGroupForm 增量

`frontend/components/Settings/IntegrationsSettings/IntegrationGroup.tsx`：

```tsx
{group.capture && (
  <Button
    variant="outline"
    onClick={() => openCaptureDialog(group)}
    title={t("integrations.capture.button.tooltip")}
  >
    <Wand2 className="h-4 w-4 mr-1" />
    {t("integrations.capture.button.label")}
  </Button>
)}
```

### 6.2 Confirm Dialog

```tsx
<AlertDialog>
  <AlertDialogTitle>{t("integrations.capture.dialog.title")}</AlertDialogTitle>
  <AlertDialogDescription>
    {t("integrations.capture.dialog.desc", {
      url: group.capture.login_url,
      fields: group.capture.rules.map(r => r.target_field).join(", "),
      ttl: group.capture.timeout_secs,
    })}
  </AlertDialogDescription>
  <Button onClick={onStart}>{t("integrations.capture.dialog.start")}</Button>
</AlertDialog>
```

### 6.3 进行中的状态

整张卡片右上角浮一条 toast：

```
⏳ 凭据抓取中 · 请在弹出窗口登录 · 剩 04:32  [取消]
```

### 6.4 三态全走

| state | UI |
|---|---|
| `idle` | ⚡ 按钮可点 |
| `confirming` | 二次确认 dialog |
| `waiting_login` | 卡片角浮 toast「请在弹窗登录」+ 倒计时 |
| `captured` | toast 变绿「✓ 已抓取 N 个字段」, 自动重新 `integrations_get` 刷新表单 |
| `partial` | toast 变黄「⚠ 部分字段抓到，剩 X 个需要手填」+ 列出哪些字段缺 |
| `timeout` | toast 变红「⏱ 超时，请重试或手动复制」+ 提供「打开调试视图」按钮 |
| `failed` | toast 变红 + 展示 failed_rules，引导手动 |
| `cancelled` | toast 灰色「已取消」, 1 秒后消失 |

### 6.5 hook：useCaptureSession

```tsx
function useCaptureSession() {
  const [state, setState] = useState<CaptureState>("idle");
  const [sessionId, setSessionId] = useState<string | null>(null);

  const start = async (toolId, groupId) => {
    setState("confirming");
    if (!await userConfirms()) { setState("idle"); return; }
    setState("waiting_login");
    const { session_id } = await api.integrations.captureStart(toolId, groupId);
    setSessionId(session_id);
  };

  useEffect(() => {
    if (!sessionId) return;
    const un = listen("integration-capture", (evt) => {
      if (evt.payload.session_id !== sessionId) return;
      setState(evt.payload.state);
      if (evt.payload.state === "captured") queryClient.invalidateQueries(["integration", toolId, groupId]);
    });
    return () => un.then(f => f());
  }, [sessionId]);

  const cancel = async () => {
    if (sessionId) await api.integrations.captureCancel(sessionId);
    setState("idle");
  };

  return { state, start, cancel };
}
```

---

## 7. 错误码

| code | 触发场景 | UI 文案 |
|---|---|---|
| `CAPTURE_NO_RECIPE` | schema 没声明 capture | "此 integration 不支持自动抓取" |
| `INTEGRATION_NOT_FOUND` | tool/group 不存在 | "Integration 配置已变更，请刷新" |
| `CAPTURE_ALREADY_RUNNING` | 同一 group 有 inflight session | "正在抓取中，请先取消" |
| `CAPTURE_SESSION_NOT_FOUND` | session TTL 后查询 | "抓取会话已过期" |
| `WEBVIEW_CREATE_FAILED` | Tauri 创窗失败 | "无法打开浏览器窗口，请检查 Golish 是否有窗口权限" |
| `CAPTURE_TIMEOUT` | 5 min 未登录 | "5 分钟未完成登录，已取消" |
| `CAPTURE_RULE_FAILED` | 某条 rule required=true 且没抓到 | "未找到 cookie '{name}'，可能未完整登录" |
| `STORAGE_WRITE_FAILED` | 抓到了但写 vault 失败 | "已抓取但保存失败，请重试" |

错误码统一走现有 `GolishError::Internal(msg)` 框架（`backend/crates/golish/src/error.rs`），message 前缀 `[CAPTURE_*]` 让 frontend `mapErr()` 拆出去。

---

## 8. 安全模型

### 8.1 必守的红线

| 风险 | 缓解 |
|---|---|
| **Cookie 跨进程泄露** | 每个 capture session 独立 `data_directory()`，session 结束删干净 |
| **第三方网站 XSS 把恶意 JS 注进 Golish** | 用 `WebviewUrl::External()` + 默认 CSP；不允许 capture webview 调任何 Tauri command（IPC 桥仅供 capture engine 内部用） |
| **重定向钓鱼** | navigation handler 检查 `new_url.origin() == login_url.origin()`，跨域跳转禁止（除非 schema 显式声明白名单） |
| **抓取规则越权** | 仅按 schema 声明的 `domain + name` 抠 cookie，不会扫整个 cookie store；rule type 是 enum，schema 改不出意外行为 |
| **抓到的 cleartext 在内存停留过久** | 后端抠完立刻 write vault + zero out 缓冲区；前端从未持有 cleartext |
| **TTL 内 cookie 累积** | TTL 到 → 强制关窗 → 清 data dir |
| **用户误点⚡按钮** | 二次确认 dialog 明示「即将打开 X、登录后会抠 Y」|
| **同时跑多个 session** | 同 (tool, group) 互斥（409）；不同 group 可以并行 |
| **schema 被人改坏（domain=evil.com）** | schema 在仓库内（toolsconfig + intel-provider 代码内），用户改自己电脑 = 自担风险；不防自己人 |

### 8.2 IDOR 适用性

本项目是单机 Tauri 桌面，不存在多用户。但等价问题：

- 「用户 A 的 capture 能否抓到主窗（Golish 主界面）登录态？」→ 不能，data_directory 隔离
- 「capture webview 能否调 vault read？」→ 不能，IPC bridge 仅暴露 capture engine 内部 callback
- 「外部脚本能否伪造 session_id 调 capture_cancel 别人的 session？」→ 后端 cancel 时校验 session 所有权（`state.sessions.get(sid)`）

---

## 9. 实施分期

| Phase | 范围 | 工量 | 验证 |
|---|---|---|---|
| **P1 · MVP** | Schema 加 capture + CaptureEngine 仅支持 `Cookie` rule + 3 IPC command + 前端⚡按钮 + ENScan-AQC 试点 + Confirm Dialog + 三态 UI + 自动/手动双信号 | 1 会话 | `cargo nextest -p golish-integrations` 含 capture 单测全绿；`pnpm vitest` capture hook + 按钮组件全绿；手动跑 `just dev` 完成 1 次 AQC cookie 抓取 |
| **P2 · 多规则** | `CookieJoined` / `LocalStorage` / `SessionStorage` / `PageContent` / `UrlQuery` 全支持 + ENScan 其它 4 个 group + FOFA / 0.zone | 1 会话 | 5 个 group / 2 个 provider 实测成功 |
| **P3 · 高级** | 多步骤 recipe（先访问 A 再访问 B 才能抠到）+ XHR 响应抠取（点某按钮触发 XHR、拦 response body）+ multi-domain origin 白名单 | 后续按需 |  |

---

## 10. 风险与未决问题

| # | 风险 / 问题 | 当前对策 | 后续 |
|---|---|---|---|
| R1 | Tauri 2 `webview.cookies_for_url(url)` API 在 Linux webkit2gtk 上行为差异 | 实施前在 macOS + Linux 双平台跑 spike，写 1 个小 demo 验证 cookie 读取 | 若 Linux 不支持，限定 P1 只支持 macOS + Windows |
| R2 | `WebviewUrl::External(...)` + `data_directory()` 在 Tauri 2 release 版的 API 命名可能不一样 | spike 时锁定 Tauri 2.x 版本号 + 验 API 签名 | 若 API 名变了，更新 design doc |
| R3 | 某些站点用 HttpOnly cookie——`webview.cookies(...)` 是否能读？ | 是的，Tauri webview API 在 native 层走的是浏览器 cookie store（不是 JS document.cookie），HttpOnly 可读 | 写一条单测验证 |
| R4 | 抓到的 cleartext 怎么在主进程到 vault 过程中防泄露 | 实施时只用 `secrecy::Secret<String>` 包；不 log；不进 trace span（即便 trace_id 全链路） | 用 `secrecy` crate（项目应该有，没有就加） |
| R5 | 用户登录某些站要 2FA／滑块／手机验证码——能否「半自动」？ | P1 仅支持纯密码登录站点；2FA 站点等用户搞定后 success_url 仍能命中，原则上也能工作 | 实测后定 |
| R6 | success_url_pattern 写错导致永远不触发自动抓取 | P1 提供「手动点抓取完成」兜底按钮 | 显示当前 webview URL 让用户对照 pattern 调 |
| R7 | 多 capture session 并发对 IPC bridge 干扰 | session_id 在 bridge message 里强相关 | 不允许同 group 并发；不同 group 各开独立 webview / bridge |
| R8 | 用户在 capture webview 里登录后**手动关窗**而不是等自动 | webview close event → 触发一次 try_extract，抓到啥算啥（partial 也 OK） | 这其实是个 happy path 而不是 risk |
| R9 | 用户上传的 toolsconfig JSON 里 capture.login_url 是 javascript: 或 file:// | schema validation 时只允许 `http(s)://` | 加 url scheme 白名单校验 |
| R10 | rules.target_field 写错（不在 fields[] 中） | schema validation 时校验 cross-reference | 加 schema 自校验单测 |

---

## 11. 设计开放问题（待用户决策）

> 实施前需要拍板的小决策。

1. **登录失败后是否自动重试？** 推荐：不重试，用户重新点⚡ 即可（避免误用）
2. **抓取成功后是否自动测试连接？** 推荐：是（如果 `group.test` 也声明了，抓取成功 → 自动 fire test → 减少手动验证）
3. **⚡ 按钮位置**：放在 Save 旁边、还是在每个 field 旁边（一个 field 一个⚡）？ 推荐：group 级别一个，避免视觉繁琐
4. **capture webview 的标题栏样式**：透明跟主窗一致、还是显眼些（提示用户这是「Golish · 凭据抓取」）？ 推荐：显眼些
5. **是否在 vault 里多打一个标签 `tags=["capture-source", "auto"]`**，方便后续审计「哪些凭据是自动抓的、哪些是手填的」？ 推荐：是

---

## 12. 命名速查

| 概念 | Rust | TS | 备注 |
|---|---|---|---|
| 抓取食谱 | `CaptureRecipe` | `CaptureRecipe` | ts-rs 同步 |
| 一条规则 | `CaptureRule` (enum) | `CaptureRule` (union) | tagged union |
| 一次会话 | `CaptureSession` | session_id: string | UUID v4 |
| 引擎 | `CaptureEngine` | - | Tauri `State<CaptureEngine>` |
| 后端 command | `integrations_capture_*` | `api.integrations.capture*` | 跟 integrations 命名 |
| 事件 | `app.emit("integration-capture", ...)` | `listen("integration-capture", ...)` | 单数命名跟当前事件惯例 |
| 前端 hook | - | `useCaptureSession` | |
| 前端按钮 | - | `<CaptureButton>` | 放 IntegrationGroup.tsx |

---

## 13. 验收清单（passing 标准）

1. ENScan AQC：用户点⚡ → 弹窗 → 在 `aiqicha.baidu.com` 登录 → 自动关窗 → AQC group 的 cookies.aqc 字段显示「已配置」徽章
2. ENScan AQC：用户点⚡ → 5 分钟内不操作 → 弹窗自动关 → 卡片显示「超时」+ 提示重试
3. ENScan AQC：用户点⚡ → 中途点「取消」 → 弹窗立刻关 → 卡片回 idle
4. 同 group 重复点⚡ 在第一次未结束时 → 409 错误 + 友好提示「正在抓取中」
5. capture webview 退出后 → `~/Library/Application Support/com.golish.platform/capture-sessions/<sid>/` 目录被清干净
6. 抓到的 cookie 在 vault_entries 表里有 1 行；vault entry 标签包含 `capture-source` (如设计)
7. 单测：`Cookie` rule 抠成功 / 抠失败 / domain 不匹配 / TTL 超时 → 4 个 case 各覆盖 1 测
8. 前端单测：useCaptureSession 状态机 6 个 state 转移各覆盖 1 测；CaptureButton 在 group.capture 存在与否的渲染差异覆盖 1 测
9. `cargo nextest -p golish-integrations -p golish` 整体绿
10. `pnpm vitest run frontend/components/Settings/IntegrationsSettings/` 整体绿
11. design + plan 文档进仓库 + feature_list.json 加 `capture-engine` 条目

---

## 14. 与上一轮 Integrations 的衔接

| Phase 1-5 已完成 | 本设计 P1 |
|---|---|
| `IntegrationGroup` 结构已稳定 | 加一个可选字段 `capture` → 完全向后兼容 |
| 3 个 storage backend + tester | 复用，无改动 |
| 前端 `IntegrationGroupForm` 已能渲染 fields | 加一个⚡按钮，老组件不动 |
| `integrations_*` 5 IPC + facade | 新增 3 个 `integrations_capture_*`，走同一 facade |
| i18n `integrations.*` 27 键 | 新增 `integrations.capture.*` 约 10 键 |

**零回退**：本设计不动 schema 已有字段、不动现有 IPC、不动现有前端组件 contract。开关：`group.capture` 不填，⚡ 按钮不出，体验跟现在一模一样。

---

> 设计完成后下一步：
>
> 1. 用户审完确认这版 design 没大问题（或提修改意见）
> 2. 写实施计划 `docs/superpowers/plans/2026-05-21-credential-capture-engine.md`（按 P1 切 task）
> 3. `feature_list.json` 添加 `capture-engine` 条目（status: `not_started`）
> 4. 把当前 `integrations` 从 `in_progress` 切到 `passing`（Phase 1-5 已 commit），再把 `capture-engine` 切到 `in_progress`
> 5. 按 plan 推 P1
