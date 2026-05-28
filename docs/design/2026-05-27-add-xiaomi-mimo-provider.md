# 接入小米 MiMo 平台为新增 LLM Provider

> Status: Design Draft (2026-05-27)
> Author: Cursor Agent (bajie-mcp-agent-1)
> Scope: 给 Golish 平台新增小米 MiMo（`xiaomimimo.com`）作为 LLM 提供商，OpenAI 与 Anthropic 双协议兼容
> Related: AGENTS.md §2.2 / `backend/crates/golish-llm-providers/`

## 1. 目标

给 Golish 用户多一个国产模型选择：小米 MiMo 系列（`mimo-v2.5-pro` 等）。
小米 Token Plan 平台同时提供 **OpenAI 兼容** 与 **Anthropic 兼容** 两套协议端点，复用同一把 API Key。
我们让用户在设置里像配 DeepSeek 一样填 API Key + 区域，模型就出现在选择器中。

## 2. 非目标

- 不做小米**按量付费**（`sk-xxxxx`）链路的特殊封装：协议接口与 Token Plan 一致，把 base url 作为可改字段即可。
- 不做小米平台**模型自动发现**：先硬编码注册 `mimo-v2.5-pro` 等少量代表模型，后续按需扩。
- 不做 Token Plan 用量配额可视化、订阅管理 UI（属于 vendor portal 范畴，不是 Golish 关注的）。

## 3. 上游事实（来自 `https://platform.xiaomimimo.com/docs/zh-CN/price/tokenplan/quick-access`）

| 项 | 值 |
|---|---|
| OpenAI 兼容 base url | `https://token-plan-cn.xiaomimimo.com/v1`（中国） / `.../sgp/...` (新加坡) / `.../ams/...` (欧洲) |
| OpenAI 兼容路径 | `POST /chat/completions` |
| Anthropic 兼容 base url | `https://token-plan-cn.xiaomimimo.com/anthropic`（中国/新加坡/欧洲三集群同前缀规则） |
| Anthropic 兼容路径 | `POST /v1/messages`（请求 path 是 `${BASE_URL}/v1/messages`） |
| 认证 header | `api-key: $MIMO_API_KEY` |
| API Key 格式 | `tp-xxxxx`（Token Plan）或 `sk-xxxxx`（按量付费），相互独立 |
| 代表模型 | `mimo-v2.5-pro`（其它待 vendor 文档列出） |

**未确认事项**（需联调验证）：
1. 小米端点是否同时接受标准 `Authorization: Bearer $KEY`（多数 OpenAI 兼容平台都接受），如果接受则 rig-core 默认 client 零改动直接通；如果只接受 `api-key:`，需走 §6.2 的 header 注入方案。
2. Anthropic 兼容路径是否也接受 `x-api-key:`（标准 Anthropic header），还是只接受 `api-key:`。
3. 各模型的 reasoning / vision / tool-use / streaming 能力具体支持到哪个程度（先按"标准 chat completion + tool 调用"假设，能力探测推 Phase 2）。

## 4. 项目侧现状

`backend/crates/golish-llm-providers/` 已有 14 个 provider 接入，**最贴近本次需求的样板**：

- **DeepSeek**（`provider_trait/deepseek.rs`，30 行）→ OpenAI 兼容 + 自定义 base_url 的标准模板
- **Anthropic**（`provider_trait/anthropic.rs`，44 行）→ Anthropic 协议样板
- **Z.AI**（`rig-zai-sdk` crate）→ 完整自定义 fork 路径（本次不走，太重）

settings 侧 `golish-settings/src/schema/llm.rs` 已有 `DeepSeekSettings { api_key, base_url, show_in_selector }` 完全对应的样板。

模型 capabilities 注册在 `golish-llm-providers/src/model_capabilities/`（具体写法见 §7.4）。

枚举 `AiProvider`（`golish-settings/src/schema/enums.rs:11`）现在 13 个变体，新增 `Xiaomi` 即可。

## 5. 设计

### 5.1 用户视角

设置页（`frontend` Settings → AI providers）多一行 **Xiaomi MiMo (Token Plan)**，字段：

| 字段 | 用途 | 默认 |
|---|---|---|
| API Key | 小米 Token Plan 颁发的 `tp-xxxxx` 或按量付费 `sk-xxxxx` | _(空)_ |
| Region | 选择集群 cn/sgp/ams（影响默认 base url） | `cn` |
| Protocol | `OpenAI` 或 `Anthropic` 或 `Auto`（基于模型决定） | `Auto` |
| OpenAI Base URL | 进阶覆盖（默认按 region 推导） | _(留空走默认)_ |
| Anthropic Base URL | 进阶覆盖（默认按 region 推导） | _(留空走默认)_ |
| Show in selector | 是否在模型选择器里显示小米模型 | `true` |

### 5.2 协议路由策略

每个小米模型在 `model_capabilities` 注册时携带一个 `transport_protocol` 字段标记它走 OpenAI 还是 Anthropic 兼容路径。`XiaomiProviderImpl::create_client` 据此返回不同的 `LlmClient` 变体：

```text
mimo-v2.5-pro (OpenAI 兼容) → LlmClient::RigXiaomi
mimo-v2.5-pro (Anthropic 兼容) → LlmClient::RigXiaomiAnthropic
```

如果用户想强制某模型走特定协议，可以在 model id 末尾加 `@anthropic` / `@openai` 后缀（参考现有 `claude-opus-4-5@20251101` 风格）；不带后缀按 capability 注册的默认协议。

### 5.3 LlmClient 变体新增

`golish-llm-providers/src/lib.rs::LlmClient`（当前 14 个变体）新增两个：

```rust
RigXiaomi(rig_openai::completion::CompletionModel),
RigXiaomiAnthropic(rig_anthropic::completion::CompletionModel),
```

变体不复用 `RigOpenAi` / `RigAnthropic` 是因为：

1. **provider_name() 必须能区分**（用于 trace、用量统计、错误归因）
2. **未来如果小米要走 header 注入路径**，可在 `XiaomiProviderImpl::create_client` 内独立构造 `reqwest::Client` 而不影响别的 provider
3. **is_openai() / is_anthropic() 判定**需要返回正确分类（`RigXiaomi` → `is_openai() = true`，`RigXiaomiAnthropic` → `is_anthropic() = true`），让上层逻辑（如 `supports_openai_web_search`）正确决策

### 5.4 dispatch 宏 arm

`dispatch_llm_client!` 与 `dispatch_llm_client_split!` 各加 2 个 arm（小米 OpenAI 走 generic 路径，小米 Anthropic 也走 generic，**不**复用 VertexAnthropic 的 thinking 路径，因为小米 anthropic 协议本身不一定支持 extended thinking）。

### 5.5 settings schema

`golish-settings/src/schema/llm.rs` 新增 `XiaomiSettings`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct XiaomiSettings {
    pub api_key: Option<String>,
    pub region: Option<String>,           // "cn" | "sgp" | "ams"
    pub default_protocol: Option<String>, // "openai" | "anthropic" | "auto"
    pub openai_base_url: Option<String>,  // override
    pub anthropic_base_url: Option<String>, // override
    pub show_in_selector: bool,
}
```

`AiSettings { ..., xiaomi: XiaomiSettings::default() }`。

### 5.6 ProviderSettings + ProviderExtraSettings

`provider_trait/mod.rs::ProviderExtraSettings` 加：

```rust
pub xiaomi_region: Option<String>,
pub xiaomi_default_protocol: Option<String>,
pub xiaomi_anthropic_base_url: Option<String>,
```

（OpenAI base url 复用现有 `ProviderSettings::base_url`。）

### 5.7 XiaomiProviderImpl

新文件 `golish-llm-providers/src/provider_trait/xiaomi.rs`：

```rust
pub struct XiaomiProviderImpl {
    pub api_key: String,
    pub region: XiaomiRegion,                // 默认 Cn
    pub default_protocol: XiaomiProtocol,    // 默认 Auto
    pub openai_base_url: Option<String>,
    pub anthropic_base_url: Option<String>,
}

#[derive(Clone, Copy)]
pub enum XiaomiRegion { Cn, Sgp, Ams }

#[derive(Clone, Copy)]
pub enum XiaomiProtocol { OpenaiCompatible, AnthropicCompatible, Auto }

impl LlmProvider for XiaomiProviderImpl {
    async fn create_client(&self, model: &str) -> Result<LlmClient> {
        let protocol = resolve_protocol(model, self.default_protocol);
        match protocol {
            XiaomiProtocol::OpenaiCompatible => /* rig_openai::Client + base_url */,
            XiaomiProtocol::AnthropicCompatible => /* rig_anthropic::Client + base_url */,
            Auto => /* fallback to model registry capability transport_protocol */,
        }
    }
}
```

`resolve_protocol(model, default)`：先看 model id 后缀（`@openai` / `@anthropic`）→ 再看 model registry capability → 再回退到 `default`（Auto 时按 capability 决定，仍未决定时报错引导用户配置）。

## 6. 关键技术风险与回退路径

### 6.1 ⚠️ 风险 A：`api-key:` header 是否兼容标准 Bearer

- **乐观路径（多数情况）**：小米同时接受 `Authorization: Bearer`，rig-core 零改动通过。
- **回退路径**：rig-core 的 `rig::providers::openai::Client::builder()` 不支持自定义 header，但支持自定义 `reqwest::Client`。我们可以构造一个 `reqwest::Client::builder().default_headers(headers)` 注入 `api-key: $KEY`，把它喂给 `Client::builder().http_client(client)`。代价 ~30 行。
- **保险路径（最重）**：在 `rig-openai-responses` fork 里加一个 `XiaomiHeaderTransport`，复制 OpenAI client 但 header 走 `api-key:`。除非前两条都不通才考虑。

实施顺序：先按乐观路径写代码 → 用真实 API Key curl 实测 → 不通则升级到回退路径。

### 6.2 ⚠️ 风险 B：Anthropic 兼容路径 base url

rig-core 的 `rig::providers::anthropic::Client::new(api_key)` 默认走 `https://api.anthropic.com`，需要用 `Client::builder().base_url(...)` 形式覆盖。需要确认 rig-core 0.36 的 anthropic client 是否暴露了 builder。如果没暴露，回退到自己实现 anthropic 子集（避免叉得太开 Phase 2 再处理）。

### 6.3 风险 C：模型注册体系

`golish-models::get_model_capabilities` 是个静态注册表，需确认新加 `Xiaomi` 时如何调用。如果它是 hashmap 注册路径，按 deepseek 已有写法照抄即可。

### 6.4 风险 D：双变体在 `dispatch_llm_client!` 宏里铺开导致编译时间增加

每加一个 `LlmClient` 变体，所有用到 `dispatch_llm_client!` 的调用站点会展开多一个 match arm。当前有 ~15 个变体已经能通过编译，再加 2 个不构成压力。

## 7. 实施步骤（细节见 plan）

按 Phase 切分：

- **Phase 1**：枚举 + settings schema + ProviderSettings + provider_trait 注册
- **Phase 2**：XiaomiProviderImpl + LlmClient 变体 + dispatch 宏
- **Phase 3**：模型 capabilities 注册（mimo-v2.5-pro 等）
- **Phase 4**：前端 settings UI（pnpm 端）
- **Phase 5**：联调（curl + 真实 API Key），收尾

每个 Phase 完成后跑 `cargo check -p golish-llm-providers` 确认增量绿，最终 `just precommit` 全绿。

## 8. 不变量自检

| Golish 不变量（AGENTS.md §5） | 本设计如何遵守 |
|---|---|
| I1 错误码 | provider 创建失败用 `anyhow::anyhow!` 包裹原 rig-core 错误，上层会按现有 `LlmClient::create_client_for_model` 错误链处理 |
| I4 命令命名 | 本 PR 不引入新 Tauri command，纯 Rust 内部能力扩展 |
| I5 ts-rs 类型同步 | `XiaomiSettings` 是 settings 层结构，已有 `golish-settings` ts-rs 派生流程；如果有 ts-rs derive 缺失，按现有 settings 结构补 `#[derive(ts_rs::TS)]` 即可 |
| I6 设计变更走新 markdown | 本文件是新设计，不覆盖旧 |
| I8 已检查为空 ≠ 未检查 | 本 PR 与 pentest 数据语义无关，不触发 |

## 9. 测试计划

- **单测**（必须）：
  1. `XiaomiProviderImpl::validate_credentials` 空 key 报错 / 非空通过
  2. `resolve_protocol` 矩阵：4 种后缀场景 × 3 种 default × 2 种 capability = 完整覆盖
  3. `XiaomiRegion::Cn/Sgp/Ams` → base url 推导正确
- **联调**（用户提供 key 后）：
  1. curl 实测 OpenAI 路径 `/chat/completions` 返回 200
  2. curl 实测 Anthropic 路径 `/v1/messages` 返回 200
  3. `just dev` 启动后选 `mimo-v2.5-pro` 发一个 prompt，看到返回流式

## 10. 回滚

本 PR 是纯增量（新枚举变体 + 新 struct + 新文件），不动现有逻辑。
回滚 = `git revert <commit>` 单 commit 回退，所有现有 provider 不受影响。
