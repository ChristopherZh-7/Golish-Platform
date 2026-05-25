# LLM Models JSON-Driven Registry 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `executing-plans` 逐任务实现此计划；实现代码前遵守 TDD，先写失败测试。
> **关联设计：** `docs/design/2026-05-25-llm-models-json-driven.md`
> **关联前置工作：** `docs/superpowers/plans/2026-05-22-asset-intel-json-driven-providers.md`（同款架构理念，asset-intel CLI 工具的 JSON 驱动改造）

## 目标

把 `backend/crates/golish-models/src/providers/nvidia.rs` 当前硬编码的 28 个 NVIDIA NIM 模型迁到 `resources/llm-models/nvidia.json`，改 JSON 不改 Rust 代码，编译期 / 运行期都能让新模型生效；并暴露 ts-rs DTO 供前端使用。其它 11 个 provider 留作 Phase 2 增量迁移。

## 文件结构

- `backend/crates/golish-models/src/providers/nvidia.rs`：删除手写 vec! 注册表，改为 JSON 加载函数
- `backend/crates/golish-models/src/descriptors/mod.rs`：新增模块，定义 `ProviderModelsFile` / `ModelDescriptor` 反序列化类型
- `backend/crates/golish-models/src/descriptors/loader.rs`：新增 loader 函数 `load_provider_models(provider, app_resource_dir) -> Vec<ModelDefinition>` 含 fallback
- `backend/crates/golish-models/src/descriptors/capabilities_base.rs`：新增 `resolve_capabilities_base()` 字符串→函数映射
- `backend/crates/golish-models/src/descriptors/dto.rs`：新增 ts-rs 导出类型 `ModelDescriptorDto`
- `resources/llm-models/nvidia.json`：新建，承载 28 个 NVIDIA NIM 模型
- `backend/crates/golish/src/ai/commands/core/lifecycle.rs`：注册 Tauri command `model_registry_list_by_provider`（供前端 Phase 3 使用）
- `frontend/lib/generated/`：ts-rs 自动生成的 DTO 文件（不手动编辑）
- `docs/superpowers/plans/2026-05-25-llm-models-json-driven.md`：完成后标记 status: completed

## 任务 1：定义 JSON 反序列化类型与最小化测试

**文件：** `backend/crates/golish-models/src/descriptors/mod.rs`（新建）

### 1.1 先写失败测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_provider_models_file() {
        let raw = r#"{
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
        }"#;
        let parsed: ProviderModelsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.provider, "nvidia");
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].id, "moonshotai/kimi-k2.6");
        assert_eq!(parsed.models[0].capabilities.context_window, Some(256_000));
        assert_eq!(
            parsed.models[0].thinking_quirks,
            Some("explicit_thinking".to_string())
        );
    }

    #[test]
    fn rejects_missing_id() {
        let raw = r#"{
            "provider": "nvidia",
            "models": [{ "display_name": "X" }]
        }"#;
        let parsed = serde_json::from_str::<ProviderModelsFile>(raw);
        assert!(parsed.is_err());
    }
}
```

### 1.2 跑测试确认红灯

```bash
cargo nextest run -p golish-models descriptors::tests::parses_minimal_provider_models_file
```

预期失败原因：`ProviderModelsFile` 类型不存在。

### 1.3 增加类型实现

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelsFile {
    pub provider: String,
    #[serde(default)]
    pub default_capabilities_base: Option<String>,
    pub models: Vec<ModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub capabilities: CapabilitiesDescriptor,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub thinking_quirks: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesDescriptor {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub context_window: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
    #[serde(default)]
    pub supports_thinking_history: Option<bool>,
    // ...其他 capability 字段以同样模式新增
}
```

### 1.4 跑测试确认绿灯

```bash
cargo nextest run -p golish-models descriptors::tests
```

两个测试均应通过。

## 任务 2：实现 capabilities base 字符串→函数解析

**文件：** `backend/crates/golish-models/src/descriptors/capabilities_base.rs`（新建）

### 2.1 先写测试

```rust
#[test]
fn resolves_nvidia_large_defaults_base() {
    let caps = resolve_capabilities_base(Some("nvidia_large_defaults"));
    let expected = ModelCapabilities::nvidia_large_defaults();
    assert_eq!(caps, expected);
}

#[test]
fn unknown_base_falls_back_to_conservative() {
    let caps = resolve_capabilities_base(Some("zzz_unknown_base"));
    let expected = ModelCapabilities::conservative_defaults();
    assert_eq!(caps, expected);
}

#[test]
fn none_base_returns_conservative() {
    let caps = resolve_capabilities_base(None);
    assert_eq!(caps, ModelCapabilities::conservative_defaults());
}
```

### 2.2 实现

```rust
pub fn resolve_capabilities_base(name: Option<&str>) -> ModelCapabilities {
    match name {
        Some("nvidia_defaults") => ModelCapabilities::nvidia_defaults(),
        Some("nvidia_large_defaults") => ModelCapabilities::nvidia_large_defaults(),
        Some("nvidia_small_defaults") => ModelCapabilities::nvidia_small_defaults(),
        // 后续 Phase 2 接入其他 provider 默认
        _ => ModelCapabilities::conservative_defaults(),
    }
}

pub fn merge_capabilities(base: ModelCapabilities, desc: &CapabilitiesDescriptor) -> ModelCapabilities {
    let mut out = base;
    if let Some(cw) = desc.context_window { out.context_window = cw; }
    if let Some(mo) = desc.max_output_tokens { out.max_output_tokens = mo; }
    if let Some(v) = desc.supports_vision { out.supports_vision = v; }
    if let Some(t) = desc.supports_thinking_history { out.supports_thinking_history = t; }
    out
}
```

确保 `ModelCapabilities` 满足 `PartialEq`，如果还没派生则在 `capabilities.rs` 加 `#[derive(PartialEq, Eq)]`。

## 任务 3：实现 loader 与 fallback 机制

**文件：** `backend/crates/golish-models/src/descriptors/loader.rs`（新建）

### 3.1 先写测试

```rust
#[test]
fn loader_returns_embedded_defaults_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let models = load_provider_models(AiProvider::Nvidia, dir.path());
    assert!(!models.is_empty(), "embedded fallback must yield models");
}

#[test]
fn loader_prefers_resource_file_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{
        "provider": "nvidia",
        "models": [{
            "id": "test/synthetic",
            "display_name": "Synthetic",
            "capabilities": { "base": "nvidia_defaults" },
            "aliases": []
        }]
    }"#;
    std::fs::write(dir.path().join("nvidia.json"), json).unwrap();
    let models = load_provider_models(AiProvider::Nvidia, dir.path());
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "test/synthetic");
}
```

### 3.2 实现 loader

```rust
pub fn load_provider_models(provider: AiProvider, resource_dir: &Path) -> Vec<ModelDefinition> {
    let slug = provider_slug(provider);
    let path = resource_dir.join(format!("{slug}.json"));
    match read_file(&path) {
        Ok(file) => file.into_definitions(provider),
        Err(e) => {
            tracing::warn!(
                provider = ?provider,
                error = %e,
                "LLM models JSON not loaded; falling back to embedded defaults"
            );
            embedded_defaults_for(provider)
        }
    }
}

fn embedded_defaults_for(provider: AiProvider) -> Vec<ModelDefinition> {
    match provider {
        AiProvider::Nvidia => {
            let raw = include_str!("../../../../../../resources/llm-models/nvidia.json");
            serde_json::from_str::<ProviderModelsFile>(raw)
                .expect("embedded nvidia.json must parse")
                .into_definitions(provider)
        }
        // 其他 provider Phase 2 接入
        _ => Vec::new(),
    }
}
```

`include_str!` 路径**必须**指向 `resources/llm-models/nvidia.json`（任务 4 创建后才能编译通过；属于交叉依赖，临时把任务 3 标 ignore 直到任务 4 完成）。

> 实践技巧：任务 3 实现可以先用 `OnceLock<Vec<ModelDefinition>>` 兜底空 vec，跑过单测后在任务 4 切换。

## 任务 4：迁移 28 个 NVIDIA 模型到 `resources/llm-models/nvidia.json`

### 4.1 创建 JSON

直接根据现有 `providers/nvidia.rs` 的 `nvidia_models()` 函数内容**逐条**填进 JSON。**必须**保持模型集合、capability 数值、aliases 完全一致。

```json
{
  "provider": "nvidia",
  "default_capabilities_base": "nvidia_defaults",
  "models": [
    {
      "id": "nvidia/nemotron-3-super-120b-a12b",
      "display_name": "Nemotron 3 Super 120B",
      "capabilities": {
        "base": "nvidia_large_defaults",
        "context_window": 1000000,
        "max_output_tokens": 8192
      },
      "aliases": ["nemotron-120b", "nemotron-super"]
    },
    // ... 27 more
  ]
}
```

### 4.2 加一致性测试（最关键）

在 `backend/crates/golish-models/src/descriptors/loader.rs` 加：

```rust
#[test]
fn nvidia_json_matches_hardcoded_registry() {
    let hard = crate::providers::nvidia_models();
    let json = embedded_defaults_for(AiProvider::Nvidia);
    assert_eq!(hard.len(), json.len(), "model count must match");
    for (h, j) in hard.iter().zip(json.iter()) {
        assert_eq!(h.id, j.id);
        assert_eq!(h.display_name, j.display_name);
        assert_eq!(h.aliases, j.aliases);
        assert_eq!(h.capabilities, j.capabilities, "caps mismatch for {}", h.id);
    }
}
```

**这个测试是迁移正确性的唯一兜底**——通过即说明 JSON 与硬编码版本完全等价。

### 4.3 跑

```bash
cargo nextest run -p golish-models descriptors loader nvidia_json_matches_hardcoded_registry
```

## 任务 5：替换 `providers/nvidia.rs` 为 JSON 加载

### 5.1 删除手写 vec! 实现

```rust
// before:
pub fn nvidia_models() -> Vec<ModelDefinition> {
    vec![ /* 220 行 */ ]
}

// after:
pub fn nvidia_models() -> Vec<ModelDefinition> {
    crate::descriptors::loader::embedded_defaults_for(AiProvider::Nvidia)
}
```

### 5.2 跑全部测试

```bash
cargo nextest run -p golish-models
```

预期：所有原有测试通过 + 新增 3-4 个 descriptor 测试通过。

### 5.3 跑 quirks 联动测试

```bash
cargo nextest run -p golish-llm-providers
```

预期：9/9 通过（之前在本任务上一次会话已确认）。

## 任务 6：ts-rs DTO + Tauri command（可选 Phase 1 完成版）

### 6.1 在 `descriptors/dto.rs` 新增

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/lib/generated/")]
pub struct ModelDescriptorDto {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub aliases: Vec<String>,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub supports_vision: bool,
    pub supports_thinking_history: bool,
}
```

### 6.2 注册 Tauri command

按 `docs/development.md` 五步走（函数 → facade → registry → 前端 wrapper → ts-rs 同步），新增：

- `model_registry_list_by_provider(provider: AiProvider) -> Vec<ModelDescriptorDto>`

### 6.3 写前端 wrapper

```ts
// frontend/lib/api/model-registry.ts (新增 / 增强)
export async function listModelsByProvider(provider: AiProvider): Promise<ModelDescriptorDto[]> {
  return await invoke("model_registry_list_by_provider", { provider });
}
```

## 任务 7：验证 + 提交

### 7.1 全套验证

```bash
just precommit
```

要求全绿（`just check + just test` 全过）。

### 7.2 手动 smoke test

启动应用 `just dev`，进入 Settings → Providers → NVIDIA，确认：

- 模型下拉里能看到所有 28 个 NVIDIA NIM 模型
- 任选一个有 thinking 的模型（如 Kimi K2.6 / DeepSeek V4 Flash）发请求能正常工作
- 删除 `resources/llm-models/nvidia.json`，重启应用 → 仍能用（fallback 生效）+ 日志里有 warn

### 7.3 更新 progress

- 更新 `agent-progress.md`：记录"LLM models JSON-driven 改造 Phase 1 完成"+ 跑过的命令 + 文件清单
- 更新 `feature_list.json`：若没条目就新加一条 `status: passing`，填 `evidence` 字段

## 回滚路径

如果 Phase 1 任意步骤跑出非预期错误：

1. 保留 `resources/llm-models/nvidia.json` 不动
2. 把 `providers/nvidia.rs` 的 `pub fn nvidia_models()` 改回之前的硬编码 vec!（git revert 即可）
3. 留下 `descriptors/` 模块作为后续 Phase 2 的脚手架

不会破坏 runtime，因为 nvidia.rs 函数签名保持不变。

## Phase 2/3 占位（不在本计划范围）

- Phase 2：把 openai / anthropic / gemini / vertex_ai / vertex_gemini / groq / xai / zai_sdk / ollama / openrouter / deepseek 一个一个迁过去；每 provider 单独 PR
- Phase 3：把 `frontend/lib/ai/models.ts` 里的 `*_MODELS` 常量删掉，改为 Tauri command 动态获取
- Phase 4：（可选）从用户配置的 URL 自动同步 catalogue

## Success Metrics

| 指标 | 目标 |
|------|------|
| `nvidia_json_matches_hardcoded_registry` 测试通过 | 必须，0 容忍 |
| `just precommit` 全绿 | 必须 |
| 新增模型实测可在 JSON 里只加一行就上线 | 可选验证 |
| 应用启动时 JSON 缺失不 crash | 必须 |
| ts-rs DTO 类型正确同步到 frontend | 必须（保持 I5 不变量） |
