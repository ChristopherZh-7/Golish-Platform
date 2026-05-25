# LLM Models JSON-Driven Registry

> **状态**：Draft · 2026-05-25
> **作者**:主控中心（bajie-mcp）+ 用户授权
> **关联**：
> - `docs/superpowers/plans/2026-05-22-asset-intel-json-driven-providers.md`（同期 asset-intel JSON 驱动改造，本设计沿用同一架构理念）
> - `docs/superpowers/plans/2026-05-25-llm-models-json-driven.md`（本设计的实现计划）

## 目标

把当前散落在 `backend/crates/golish-models/src/providers/<provider>.rs` 和 `frontend/lib/ai/models.ts` 两端硬编码的 LLM 模型清单，迁移到 **JSON 驱动的资源文件**，让"新增/删除/更新模型"不需要改 Rust 代码、不需要重新编译。

## 当前痛点

| # | 痛点 | 实例 |
|---|------|------|
| 1 | 添加新模型需要改 Rust + TS 两边 + UI 分组 | 本轮同步 NVIDIA NIM 需要改 6 个文件 |
| 2 | 模型清单与 capability 数据耦合在 Rust 代码里 | `nvidia_models()` 函数 220 行手写 `ModelDefinition` |
| 3 | 前后端清单经常不同步 | `NVIDIA_MODELS` 常量比后端注册表多 20+ 个 ID |
| 4 | 上游官网更新模型时，必须等开发者发版才能让用户用上新模型 | NVIDIA NIM 一个月更新 10+ 个模型 |
| 5 | 测试 / mock 难以覆盖"模型清单变化" | 因为清单写死在 Rust 里 |

## 设计原则

1. **资源化（resource-driven）**：模型清单作为 `resources/llm-models/<provider>.json` 资源，运行时加载
2. **优雅降级**：JSON 文件缺失/损坏 → 回退到内嵌默认（保证应用能启动）
3. **可校验**：JSON Schema 严格定义结构 + Rust serde 反序列化失败即报错
4. **与既有架构对齐**：复用 asset-intel JSON-driven 同款 loader 模式
5. **零业务行为变化**：迁移完成后，runtime 表现与现在完全一致（先迁移再优化）
6. **增量推进**：先 NVIDIA 走通，其它 11 个 provider 后续单独迁移

## 架构

```
┌────────────────────────────────────────────────────────────────┐
│ runtime startup (Tauri main)                                   │
│                                                                │
│  ┌──────────────────────────────────────────────────┐         │
│  │  load_model_registry()                            │         │
│  │  ├─ for each provider:                            │         │
│  │  │  ├─ try read resources/llm-models/<p>.json     │  ← 编辑 JSON │
│  │  │  │  └─ ok → ModelDefinitionDescriptor → ModelDef       │         │
│  │  │  └─ err → fall back to embedded default        │         │
│  │  └─ merge all providers → MODEL_REGISTRY (HashMap) │         │
│  └──────────────────────────────────────────────────┘         │
│              │                                                 │
│              ▼                                                 │
│  ┌──────────────────────────────────────────────────┐         │
│  │  Tauri commands (model_registry_list, etc.)       │         │
│  └──────────────────────────────────────────────────┘         │
│              │                                                 │
│              ▼                                                 │
│  ┌──────────────────────────────────────────────────┐         │
│  │  frontend/lib/api/model-registry.ts               │         │
│  │  └─ frontend UI 直接消费 server-driven 模型清单   │         │
│  └──────────────────────────────────────────────────┘         │
└────────────────────────────────────────────────────────────────┘
```

## JSON Schema

```json
{
  "$schema": "https://golish.dev/schemas/llm-models-1.0.json",
  "provider": "nvidia",
  "default_capabilities_base": "nvidia_defaults",
  "models": [
    {
      "id": "moonshotai/kimi-k2.6",
      "display_name": "Kimi K2.6",
      "capabilities": {
        "base": "nvidia_large_defaults",
        "context_window": 256000,
        "supports_thinking_history": true
      },
      "aliases": ["kimi-k2.6", "kimi-k2-6"],
      "thinking_quirks": "explicit_thinking"
    }
  ]
}
```

### 字段说明

| 字段 | 类型 | 说明 |
|---|---|---|
| `provider` | `string` | 必填，匹配 `AiProvider` 枚举（"nvidia" / "openai" / ...） |
| `default_capabilities_base` | `string` | 可选，该 provider 默认的 capabilities 基线（如 `nvidia_defaults`） |
| `models[].id` | `string` | 必填，唯一识别 ID（含 namespace 前缀如 `moonshotai/kimi-k2.6`） |
| `models[].display_name` | `string` | 必填，UI 显示名 |
| `models[].capabilities.base` | `string` | 可选，capability 基线（覆盖 default）。合法值：`nvidia_defaults` / `nvidia_large_defaults` / `nvidia_small_defaults` 等已存在的函数 |
| `models[].capabilities.<field>` | `*` | 直接覆盖基线值。`context_window` / `max_output_tokens` / `supports_vision` / `supports_thinking_history` 等 |
| `models[].aliases` | `string[]` | 可选，模型 ID 别名 |
| `models[].thinking_quirks` | `string` | 可选，将 `quirks.rs` 里的字符串匹配迁出来。合法值：`explicit_thinking` / `qwen3_hybrid` / `none` |

## Capabilities 基线机制

为了不丢失现有 capability 函数的语义，JSON 通过 **`base` 字段引用** 命名好的基线函数：

```rust
fn resolve_capabilities_base(name: &str) -> ModelCapabilities {
    match name {
        "nvidia_defaults" => ModelCapabilities::nvidia_defaults(),
        "nvidia_large_defaults" => ModelCapabilities::nvidia_large_defaults(),
        "nvidia_small_defaults" => ModelCapabilities::nvidia_small_defaults(),
        // ... openai_defaults / anthropic_defaults 等以后陆续接入
        _ => ModelCapabilities::conservative_defaults(),
    }
}
```

然后用 JSON 里的 override 字段合并：

```rust
let mut caps = resolve_capabilities_base(&desc.capabilities.base);
if let Some(cw) = desc.capabilities.context_window { caps.context_window = cw; }
if let Some(v) = desc.capabilities.supports_vision { caps.supports_vision = v; }
// ...
```

这样保留了 "vibe" `..ModelCapabilities::nvidia_large_defaults()` 的语义，同时让 JSON 易读。

## 资源文件位置

```
resources/
  llm-models/
    nvidia.json        ← Phase 1（本次迁移）
    openai.json        ← Phase 2（后续）
    anthropic.json     ← Phase 2
    gemini.json        ← Phase 2
    ...
```

Tauri 打包时会自动包含 `resources/` 目录（参考 `tauri.conf.json` 现有配置）。运行时从 `app.path().resource_dir()` 读取。

## 加载顺序与 Fallback

```rust
fn load_provider_models(provider: AiProvider, app: &AppHandle) -> Vec<ModelDefinition> {
    let path = app.path().resolve(
        format!("resources/llm-models/{}.json", provider.slug()),
        BaseDirectory::Resource
    );

    match fs::read_to_string(&path).and_then(|s| serde_json::from_str::<ProviderModelsFile>(&s).map_err(Into::into)) {
        Ok(file) => file.into_definitions(),
        Err(e) => {
            tracing::warn!("LLM models JSON for {:?} not loaded ({}); using embedded defaults", provider, e);
            embedded_defaults_for(provider)
        }
    }
}
```

`embedded_defaults_for()` 返回 `include_str!("../../../resources/llm-models/<provider>.json")` 经 build script 编译进二进制的 JSON 解析结果（也即"内嵌的同一份 JSON"）。这样**资源文件丢失也不会 crash**。

## 前后端类型同步（ts-rs）

新增 `ModelDefinitionDto` (`#[derive(TS)]`)，生成到 `frontend/lib/generated/`。前端：

- **方案 1（推荐）**：前端不再维护 `NVIDIA_MODELS` 常量，改为通过 Tauri command `model_registry_list_by_provider("nvidia")` 异步获取
- **方案 2**：保留 `NVIDIA_MODELS` 常量但用 build-time 脚本从 JSON 生成（package.json 加 `prebuild` 钩子）

第一阶段先做 Tauri command + 保留旧常量做 fallback；第二阶段评估去掉常量。

## 与 thinking quirks 的关系

`backend/crates/golish-llm-providers/src/model_capabilities/quirks.rs` 里的"explicit thinking model 字符串匹配"会从硬编码迁到 JSON 的 `thinking_quirks` 字段，由 quirks.rs 在解析时聚合：

```rust
fn is_explicit_thinking_model(provider: AiProvider, model_id: &str) -> bool {
    MODEL_REGISTRY.lookup(provider, model_id)
        .map(|m| m.thinking_quirks == ThinkingQuirks::ExplicitThinking)
        .unwrap_or(false)
}
```

## 迁移路线

### Phase 1（本计划范围）：NVIDIA PoC
- 完整把 NVIDIA NIM 28 个模型迁到 JSON
- 后端 `providers/nvidia.rs` 改为"从 JSON 加载"
- 保留前端 `NVIDIA_MODELS` 常量（暂不动）
- ts-rs DTO + Tauri command 暴露给前端可选使用

### Phase 2（后续单独 PR）：其他 provider 增量迁移
- OpenAI / Anthropic / Gemini / Vertex / Groq / xAI / ZAI SDK / DeepSeek / Ollama / OpenRouter 一个一个搬

### Phase 3（后续）：前端去硬编码
- 前端改为通过 Tauri command 获取所有 provider 的模型清单
- 删除 `frontend/lib/ai/models.ts` 里的 `*_MODELS` 常量

### Phase 4（可选）：远程同步
- 允许用户配置一个 URL（如 build.nvidia.com 自有 API）拉取 catalogue
- 加 SSRF 校验 + 鉴权 + 缓存 + 定期刷新

## 风险与缓解

| # | 风险 | 缓解 |
|---|------|------|
| R1 | JSON 损坏导致应用启动失败 | Fallback 到内嵌 default，并 tracing::warn |
| R2 | 用户编辑 JSON 后 schema 不匹配 | serde 反序列化失败 → 报错 + fallback；附 JSON Schema 给编辑器提示 |
| R3 | capability base 函数被删除/改名后 JSON 找不到 | base 字符串解析失败 → fall back 到 `conservative_defaults()` + warn |
| R4 | 前后端 ID 不同步（前端常量 vs 后端 JSON） | Phase 3 完成后彻底消除，过渡期通过 ts-rs DTO + 单测兜底 |
| R5 | 迁移过程中破坏现有 capability 行为 | Phase 1 任务 T4 强制要求"加载后的模型集与现有 `nvidia_models()` 输出序列化完全相同" |

## 不变量保证

- I5（AGENTS.md §5）：跨 IPC 类型用 ts-rs 同步 → 新增 `ModelDefinitionDto` 严格走 ts-rs
- I7：安全任务的阶段交付必须有 evidence → 本设计的"完成"标准包含 `just precommit` + JSON 解析单测 + 与现有 registry 一致性测试

## Open Questions

1. Phase 1 完成后是否立即推动 Phase 2，还是先看一两周稳定性？
2. JSON Schema 文件放在哪儿？`resources/schemas/` 还是 `docs/schemas/`？
3. 用户编辑资源 JSON 后的"重载"——支持热重载还是只走"重启应用"？

## 决策记录

| 日期 | 决策 | 理由 |
|------|------|------|
| 2026-05-25 | 选 capabilities base 引用 + override，而非纯 JSON 全量字段 | 保留 Rust 端已有的 default 函数语义，降低初次迁移工作量 |
| 2026-05-25 | Phase 1 只做 NVIDIA | 用户主动诉求来源；asset-intel 也是单一 provider 试点 |
| 2026-05-25 | 前端常量先不动 | 避免一次性改动过大、降低破坏面 |
