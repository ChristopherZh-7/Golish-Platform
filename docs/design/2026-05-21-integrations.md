# 集成中心 (Integrations) · 架构设计文档

> 日期：2026-05-21
> 状态：Draft（待用户审核）
> 关联：
> - `docs/design/2026-05-20-asm-intel-providers.md`（Intel Providers 上一代设计，将被本文档替换 Settings 入口部分）
> - `resources/toolsconfig/enscan-go.json`（首个外部文件存储的工具）
> 分支：跟随 `asm-intel-providers` 继续推进，不另开新分支
>
> **命名说明**：本入口最初讨论时叫 "Credentials / 凭据"，因与 `target.credentials`（渗透发现的受害方账密 / `golish-pentest/src/zap/credential_detector.rs` / `frontend/store/types/tool-call.ts` 中的 `inputType: "credentials"`）撞名会让用户和 agent 都混淆，**用户已确认采用 Integrations / 集成**：
>
> - "Integrations" = Golish **去访问外部服务**用的钥匙（API key / Cookie / Token）
> - "Credentials"（target 域已占用）= Golish **打穿目标后拿到**的钥匙（账号 / 密码 / hash）
>
> 两者数据流方向相反、读写权限完全不同，必须从命名层就分开。
>
> **本轮 scope（用户已明确）**：仅把 **Intel Providers 5 家**（0.zone / fofa / quake / hunter / shodan）+ **GitHub Token** + **ENScan_GO 多家 cookie** 统一进 Integrations tab。**AI Settings（OpenAI / Anthropic / Vertex 等）保持独立**，不纳入本轮迁移。

---

## 0. Why · 上下文与动机

当前 Golish 在 Settings 里管理凭据的方式**碎片化**：

| 类型 | 当前位置 | 当前形态 | 痛点 |
|---|---|---|---|
| Intel Providers API key | Settings → Intel Providers | 5 张写死的 ProviderCard，每张一个 API key 单字段 | 不能容纳多字段凭据（如 TYC 需要 cookie + tycid + auth_token） |
| GitHub Token | Settings → Network | 单个明文输入框 | 与其它凭据视觉脱节 |
| 工具凭据（ENScan_GO 的爱企查/天眼查 Cookie） | **无入口** | 用户自己改 `~/.config/enscan/config.yaml` | 用户必须打开 yaml 手填，每周一次（cookie 1-7 天失效）；体验最差 |
| 未来工具的 API key | **无入口** | 新工具上线时只能在 Rust 里硬编码 UI | 加一个新工具就要改前端 |

用户的诉求（2026-05-21 对话）：

> 给所有这种需要 key 的东西能不能搞一个动态的那种 setting 界面。就是工具 json 文件有某个值，然后一旦有，setting 就会出现对应的添加的。然后给这些全部放在一个地方。包括这个现在的 intel providers。

**目标**：把所有"需要 key / token / cookie 的东西"统一到一个 schema-driven 的 Settings 入口，工具/Provider 在 JSON 里声明自己的凭据需求，UI 按 schema 自动渲染。新加一个工具 ≤ 改一份 JSON，不动前后端代码。

---

## 1. 核心决策

| 决策 | 选项 A（推荐 · 已选）| 选项 B | 选项 C |
|---|---|---|---|
| 1 · 入口名称 | **Integrations / 集成**（一级 Settings tab） | Connections / 连接 | Credentials / 凭据（已被 target 占用，不可用） |
| 2 · 现有 Intel Providers UI | **直接替换**（不并存） | 并存留迁移期 | 仅改文案 |
| 3 · 凭据 schema 来源 | **统一来自工具/Provider 自描述 JSON**（tool config / provider metadata） | 写在 Rust 代码里 | 写在前端 |
| 4 · Storage backend | **三种**：`vault`（加密表）/ `external_file`（写到外部 yaml/json）/ `settings`（Golish settings.toml） | 只用 vault | 只用文件 |
| 5 · 多字段凭据存储约定 | **vault 内：一组凭据 → 多行 vault entry**，按 `tags=["integration-group", "{tool_id}", "{group_id}"]` 聚合 | 一行 vault entry 存 JSON value | 新增 `integration_groups` 表 |
| 6 · External file backend | **支持 YAML/JSON 模板渲染**，写出可被 ENScan_GO 等外部进程直接读的文件 | 只支持 YAML | 用户必须用 vault |
| 7 · GitHub Token 处理 | **本轮顺带迁过来**，作为 `storage=settings` 的示例 | 留在 Network tab 不动 | 也迁但等下一轮 |
| 7B · AI provider key（OpenAI / Anthropic / Vertex）| **本轮不动**（用户明确：AI Settings 已经独立管得不错，不纳入） | 也迁 | - |
| 8 · 旧数据迁移 | **不破坏 vault_entries 现有行**，给老的 `tags=["intel-provider", X]` 一份 read alias，让新 UI 也能识别 | 写一次性迁移脚本 | 让用户重输 |

> 2026-05-21 用户已确认：1（名字 Integrations）/ 2（直接替换）/ 7B（AI Settings 不纳入）。其它项采用推荐方案。

### 1A · 为什么走 schema-driven JSON

- 让"新加一个数据源/工具"变成**配置任务**，而不是工程任务
- tool JSON 已经是 Tool Manager 的真理之源，integration schema 自然挂上去
- 前端 UI 通用化，单一组件 `<IntegrationGroup>` 渲染所有形态

### 1B · 为什么 vault + external_file + settings 三态

| 凭据形态 | 例子 | 该用哪个 |
|---|---|---|
| Golish 自己用、希望加密 | 0.zone / FOFA / Quake / Shodan API key | `vault` |
| 外部进程读它的配置文件 | ENScan_GO 读 `~/.config/enscan/config.yaml` | `external_file` |
| 已是 Golish 设置项 | GitHub Token（settings.toml）、proxy_url | `settings` |

如果只用 vault：外部进程读不到，要写一个"启动外部进程时把 vault 内容渲染成临时 yaml 注入"的胶水层，复杂且脆弱。
如果只用文件：失去加密保护。
三态共存才是务实方案，由 schema 声明走哪条路径。

### 1C · 为什么 vault 内多字段不打包成 JSON value

- vault `value` 字段是单字符串，最早就是为单 token 设计
- 打包 JSON 会让 `vault_get_value` 的语义变成"返回一个需要再解析的 blob"，破坏现有契约
- 用 tags 聚合多行更直观：`tags=["integration-group", "enscan-go", "tyc"]` → 三行 entry: cookie / tycid / auth_token

### 1D · 为什么不写一次性数据迁移

- 旧 `tags=["intel-provider", X]` 的 entry 还能用，新代码可以兼容读取
- 写迁移脚本意味着回滚成本高，凭据丢失就要重输一遍
- 给老格式一个 read alias 即可，下次保存自动转新格式

---

## 2. 架构（4 层）

```
┌────────────────────────────────────────────────────────────────┐
│ Frontend · Settings → Integrations 集成 (NEW)                   │
│   - <IntegrationsTab>                                           │
│     ├─ Search box / category filter                             │
│     └─ <IntegrationCard> × N                                    │
│        ├─ Header: name / health badge / test button             │
│        └─ <IntegrationGroup>（按 schema fields 动态渲染）       │
│           ├─ <SecretInput> / <SecretTextarea> / <UrlInput> ...  │
│           ├─ Save / Clear / Reveal                              │
│           └─ Help link                                          │
└──────────────────────┬─────────────────────────────────────────┘
                       │ IPC: integrations_* / vault_* / settings_*
                       ↓
┌────────────────────────────────────────────────────────────────┐
│ golish/src/integrations/  (IPC facade)                         │
│   - integrations_list_schemas()                                 │
│   - integrations_get(toolId, groupId)                           │
│   - integrations_set(toolId, groupId, fields)                   │
│   - integrations_clear(toolId, groupId)                         │
│   - integrations_test(toolId, groupId)                          │
└──────────────────────┬─────────────────────────────────────────┘
                       │ trait dispatch
                       ↓
┌────────────────────────────────────────────────────────────────┐
│ golish-integrations (新 crate)                                  │
│   SchemaResolver                                                │
│     ├─ 扫 resources/toolsconfig/*.json 取 integration 段        │
│     ├─ 收集 intel_providers 的 ProviderMeta.integration_schema  │
│     └─ 输出统一的 IntegrationSchema 列表                        │
│                                                                 │
│   StorageBackend trait                                          │
│     ├─ VaultBackend     (写 vault_entries)                      │
│     ├─ ExternalFileBackend (渲染模板 → 写外部 yaml/json)        │
│     └─ SettingsBackend  (写 golish settings.toml)               │
│                                                                 │
│   Tester                                                        │
│     └─ 执行 schema.test.cmd 或 schema.test.http，按 ok_regex    │
│       / status code 判定，返回 IntegrationHealth                │
└────────────────────────────────────────────────────────────────┘
```

---

## 3. 数据契约

### 3.1 IntegrationSchema（tool JSON 中的 `integration` 段）

```jsonc
{
  "tool": {
    "id": "enscan-go",
    "name": "ENScan_GO",
    "integration": {
      "category": "enterprise-intel",          // UI 分组：enterprise-intel / asm / code-host / ...
      "display_name": "ENScan_GO 企业情报",
      "storage": {
        "type": "external_file",
        "external_file": {
          "path": "~/.config/enscan/config.yaml",
          "format": "yaml",
          "template": "templates/enscan/config.yaml.tmpl",   // 可选；默认按 fields key 路径平铺
          "preserve_unknown_keys": true                       // 写入时不覆盖用户在 yaml 里的其它键
        }
      },
      "groups": [
        {
          "id": "aqc",
          "name": "爱企查 (AQC)",
          "description": "需要登录 aiqicha.baidu.com 后用 Burp 或浏览器开发者工具复制完整 cookie（含 http-only）",
          "icon": "🔑",
          "help_url": "https://github.com/wgpsec/ENScan_GO#aqc",
          "fields": [
            {
              "key": "cookies.aqc",
              "label": "Cookie",
              "type": "secret-textarea",
              "placeholder": "BAIDUID=...; BDUSS=...; ...",
              "required": true,
              "rows": 4
            }
          ],
          "test": {
            "kind": "exec",
            "cmd": "{{exec}} -n 小米 -type aqc -field icp",
            "ok_regex": "(?i)company_name|company\\s*name",
            "fail_regex": "(?i)cookie\\s*expired|auth\\s*failed",
            "timeout_secs": 30
          }
        },
        {
          "id": "tyc",
          "name": "天眼查 (TYC)",
          "description": "需要 3 个字段：完整 cookie + tycid + auth_token",
          "icon": "🔑",
          "fields": [
            { "key": "cookies.tyc", "label": "Cookie", "type": "secret-textarea", "required": true, "rows": 4 },
            { "key": "tyc.tycid", "label": "tycid", "type": "secret-text", "required": true },
            { "key": "tyc.auth_token", "label": "auth_token", "type": "secret-text", "required": true }
          ],
          "test": {
            "kind": "exec",
            "cmd": "{{exec}} -n 小米 -type tyc -field icp",
            "ok_regex": "(?i)company_name"
          }
        }
      ]
    }
  }
}
```

### 3.2 Field types（枚举）

| type | 渲染 | 备注 |
|---|---|---|
| `secret-text` | `<input type=password>` + reveal toggle | 默认遮罩 |
| `secret-textarea` | `<textarea>` + reveal toggle | 长 cookie / 证书 |
| `text` | 普通 input | 非敏感（如 endpoint URL） |
| `url` | input + URL 校验 | 加 https:// 自动补 |
| `port` | number input | 1-65535 |
| `select` | dropdown | 附 `options[]` |
| `boolean` | checkbox | |
| `proxy` | 复合：host + port + auth | 复用现有 proxy_url 格式 |

### 3.3 Intel Provider 的 integration schema

ProviderMeta 新增字段 `integration_schema: Option<IntegrationSchema>`，编译进 Rust：

```rust
// golish-intel-providers/src/zone/mod.rs
impl IntelProvider for ZoneProvider {
    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "0.zone",
            // ... 现有字段 ...
            integration_schema: Some(IntegrationSchema {
                category: "asm-cyberspace",
                display_name: "0.zone 零零信安",
                storage: Storage::Vault {
                    tag_prefix: vec!["integration-group", "0.zone"],
                },
                groups: vec![
                    IntegrationGroup {
                        id: "default",
                        name: "API Key",
                        fields: vec![Field::secret_text("api_key", "API Key", required: true)],
                        test: Some(TestKind::Builtin),  // 走 IntelProvider::test_connection
                    },
                ],
            }),
        }
    }
}
```

> 旧的 `vault.name = provider_id` 约定仍然兼容（read alias）；新写入走 `tags=["integration-group", provider_id, "default"]`。

### 3.4 Storage backend

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// 读出指定 (tool_id, group_id) 的所有 field 当前值
    /// 返回 HashMap<field_key, FieldValue>，敏感 field 实际值返回 None + has_value=true
    async fn read(&self, tool_id: &str, group_id: &str) -> Result<HashMap<String, FieldValue>>;

    /// 写入；secret 字段：vault 加密 / file 渲染 / settings 写文件
    async fn write(&self, tool_id: &str, group_id: &str, fields: HashMap<String, String>) -> Result<()>;

    /// 清除该组所有字段
    async fn clear(&self, tool_id: &str, group_id: &str) -> Result<()>;

    /// 读完整 cleartext（用于 reveal / 工具运行时注入）
    async fn read_cleartext(&self, tool_id: &str, group_id: &str) -> Result<HashMap<String, String>>;
}

pub struct FieldValue {
    pub has_value: bool,         // 是否已配置（即使敏感不返回值，UI 也能显示 ✅）
    pub display_hint: Option<String>,  // 比如 "AKIA****WXYZ"（前 4 后 4）
    pub updated_at: Option<i64>,
}
```

**VaultBackend** 约定：

```
对于 (tool_id, group_id) = ("enscan-go", "tyc"):
  vault_entries 写 3 行:
    name="enscan-go.tyc.cookies.tyc"        value=<cookie>      tags=["integration-group","enscan-go","tyc"]
    name="enscan-go.tyc.tyc.tycid"          value=<tycid>       tags=["integration-group","enscan-go","tyc"]
    name="enscan-go.tyc.tyc.auth_token"     value=<auth_token>  tags=["integration-group","enscan-go","tyc"]
  entry_type 按 type 映射：secret-* → ApiKey；其它 → Token
```

**ExternalFileBackend** 写入算法（YAML 示例）：

```
1. 解析 schema.storage.external_file.path → 绝对路径，~/$HOME 展开
2. 如果 preserve_unknown_keys=true：先 read 现有 yaml 到 serde_yaml::Value
3. 把 fields 按 "." 分层：cookies.tyc → cookies: { tyc: <value> }
4. merge 进 step 2 的 Value
5. atomic write：写到 .tmp → fsync → rename
6. 备份：第一次写入时把原文件 cp 到 path.bak.YYYYMMDD（最多保留 3 份）
```

**SettingsBackend** 写入：

```
schema.storage.settings.key = "network.github_token"
→ 调 SettingsManager.update("network.github_token", value)
```

### 3.5 Test mechanism

```rust
pub enum TestKind {
    Builtin,              // 用 IntelProvider::test_connection
    Exec {                // 跑命令
        cmd: String,      // 模板，{{exec}} = 工具 executable 绝对路径
        ok_regex: String,
        fail_regex: Option<String>,
        timeout_secs: u32,
    },
    Http {                // 发 HTTP 请求
        method: String,
        url: String,      // 模板，{{value:field_key}}
        headers: HashMap<String, String>,
        ok_status_range: (u16, u16),
    },
}
```

返回 `IntegrationHealth { status: Healthy/Invalid/Expired/RateLimited/Unknown, message, tested_at }`。

---

## 4. IPC 命令契约

```ts
// frontend/lib/api/integrations.ts

// 5 行
integrations_list_schemas() -> Schema[]
integrations_get(toolId, groupId) -> Record<fieldKey, FieldValue>
integrations_set(toolId, groupId, fields: Record<fieldKey, string>) -> { ok: true }
integrations_clear(toolId, groupId) -> { ok: true }
integrations_test(toolId, groupId) -> IntegrationHealth
```

错误码：

| code | 含义 |
|---|---|
| 40001 | 字段校验失败（required 缺失 / 类型不匹配） |
| 40401 | 找不到 schema（toolId / groupId 不存在） |
| 40901 | external_file 文件存在但无法解析（被外部进程改坏） |
| 50001 | 后端内部错误 |

---

## 5. 前端组件

```
frontend/components/Settings/IntegrationsSettings/
  index.tsx                  入口；调 integrations_list_schemas + 按 category 分组渲染
  IntegrationCard.tsx        可折叠卡（替代 ProviderCard）
  IntegrationGroup.tsx       渲染 fields[] + 保存 / 清除按钮
  fields/
    SecretInput.tsx          带 reveal toggle 的 password input
    SecretTextarea.tsx       带 reveal 的 textarea
    ProxyInput.tsx           复合 proxy 字段
    UrlInput.tsx             URL 校验
    SelectField.tsx          dropdown
    BooleanField.tsx         checkbox
  TestButton.tsx             调 integrations_test，渲染 IntegrationHealth
  CategoryNav.tsx            侧栏分组（enterprise-intel / asm / code-host）
```

UI 信息架构：

```
Settings → Integrations
├─ 🔍 搜索框（按 name / description fuzzy match）
├─ 📁 分类侧栏（左）
│   ├─ 全部
│   ├─ 企业情报（aqc / tyc / kc / rb / miit）
│   ├─ 网络空间测绘（0.zone / fofa / quake / hunter / shodan）
│   ├─ 代码托管（GitHub）
│   └─ 其它
└─ 内容区（右）
    └─ IntegrationCard × N
        ├─ 头：name · 状态 badge · 测试按钮
        ├─ 折叠区：
        │   ├─ 描述 + help_url
        │   └─ <IntegrationGroup> 动态字段
        └─ 操作：保存 / 清除
```

---

## 6. 迁移路径

### 6.1 老 vault entries 兼容（read alias）

```rust
// VaultBackend::read 的查询逻辑
async fn read(&self, tool_id, group_id) -> ... {
    let new_tag_pattern = format!("integration-group,{},{}", tool_id, group_id);
    // 先查新格式
    let rows = sqlx::query("...tags @> ARRAY[...]")...
    if !rows.is_empty() { return new_format(rows); }

    // Read alias: 旧 Intel Provider 单 key 约定
    if group_id == "default" {
        let legacy: Option<Row> = sqlx::query("SELECT * FROM vault_entries WHERE name=$1 AND entry_type='api_key' ORDER BY created_at DESC LIMIT 1")
            .bind(tool_id) ...;
        if let Some(row) = legacy {
            return Ok(/* 把单行映射成 fields["api_key"] */);
        }
    }
    Ok(empty)
}
```

### 6.2 老 UI 删除

- `frontend/components/Settings/IntelProvidersSettings/` 整个删掉
- `SettingsTabContent.tsx` 把 `intel-providers` nav item 改为 `integrations`
- i18n 翻译键 `intel.*` → `integrations.*`，保留旧键 fallback 半个版本周期

### 6.3 GitHub Token 顺带迁

- 在 hardcoded `core_integrations.json`（项目内置）加一项：

```json
{
  "id": "github",
  "integration": {
    "category": "code-host",
    "display_name": "GitHub",
    "storage": { "type": "settings", "settings": { "key": "network.github_token" } },
    "groups": [{
      "id": "default",
      "name": "Personal Access Token",
      "fields": [{ "key": "token", "label": "Token", "type": "secret-text", "required": true }],
      "test": { "kind": "http", "url": "https://api.github.com/user", "headers": { "Authorization": "Bearer {{value:token}}" }, "ok_status_range": [200, 200] }
    }]
  }
}
```

- Network tab 的 GitHub Token 输入框删掉（保留 proxy_url）

---

## 7. 安全考虑

| 项 | 处理 |
|---|---|
| vault 加密 | 复用 `golish_core::vault::obfuscate / deobfuscate`（base64 ofuscate；未来可升级 OS keychain） |
| external_file 明文 | 不可避免（外部进程的需求）；提示用户文件权限 0600 |
| settings 明文 | 同 external_file，下一轮再考虑 OS keychain |
| reveal 按钮 | 默认遮罩；reveal 后 30s 自动重新遮罩 |
| 日志 | **绝不**记录 secret 值；只记 has_value + field_key + tool_id + group_id + updated_at |
| 测试连接 | exec test 输出 truncate 到 1KB；HTTP test 不带 response body 到日志 |
| IDOR | 单机桌面应用无多用户场景；project_path 维度暂不引入（保留扩展位） |

---

## 8. 不在本计划范围

- OS keychain（macOS Keychain / Windows Credential Manager / Linux libsecret）— 留作下一轮升级
- Per-project credential override（按 project_path 隔离）— 当前桌面应用以单用户为主
- Cookie 自动刷新（无头浏览器登录）— 单独 PR
- Credential 共享（多 agent / 多端同步）— 设计期不考虑
- 加密 settings.toml — 同 OS keychain

---

## 9. 风险与缓解

| 风险 | 概率 | 缓解 |
|---|---|---|
| YAML 模板与 ENScan 真实 config schema 不一致 | 中 | 先读现有 yaml 保留未知键，写时只覆盖 schema 声明的字段 |
| 旧 vault entries read alias 漏掉某种情况 | 低 | 单测 + 用户审核期人工验证 |
| 多字段 vault entry 在删除某个 group 时漏删 | 中 | clear 用 tags 精确匹配 + 单测 |
| test 命令超时挂住 UI | 低 | 后端 timeout + 前端 disable 按钮 |
| field_key 中含 "." 在 vault.name 转义出错 | 中 | name 用 base64url 编码 group_id + field_key |
| 用户改了 external_file 后 Golish 又覆盖 | 中 | preserve_unknown_keys=true；写之前 backup |

---

## 10. 验收标准（feature_list.json verification）

```
1. resources/toolsconfig/enscan-go.json 加 integration 段；TYC 三字段 / AQC 单字段都能在 UI 渲染
2. ENScan_GO 卡片配 Cookie 后，~/.config/enscan/config.yaml 实际被写入，跑 `enscan -n 小米 -type aqc -field icp` 成功
3. 0.zone API key 通过新 UI 配好，跑 intel_query_provider 仍然返回结果（向后兼容验证）
4. GitHub Token 通过新 UI 配好，安装 GitHub 工具不再 403
5. 旧 IntelProvidersSettings 入口已删；新入口 Settings → Integrations 可见
6. AI Settings 维持原样（独立 tab）
7. `just precommit` 全绿
```

---

## 11. 用户已确认事项（2026-05-21）

1. ✅ 入口名称：**Integrations / 集成**
2. ✅ 直接替换现有 Intel Providers UI（不并存）
3. ✅ schema 来源：tool JSON / Provider 自描述
4. ✅ 三种 storage backend（vault / external_file / settings）
5. ✅ AI Settings（OpenAI / Anthropic / Vertex）**不纳入**本轮
6. ✅ external_file 备份保留 3 份滚动
7. ✅ tag 命名：`integration-group`
8. ✅ 跟随 `asm-intel-providers` 分支，不另开新分支

## 12. 下一步

进入实施计划：`docs/superpowers/plans/2026-05-21-integrations.md`（5 个 Phase）。
