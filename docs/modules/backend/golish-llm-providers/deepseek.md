# golish-llm-providers / deepseek

> **一句话职责**：DeepSeek provider 常量——OpenAI 兼容 Chat Completions，官方端点 `https://api.deepseek.com`（**故意不带 `/v1`**）。

- **类型**：目录模块（属于 crate [`golish-llm-providers`](../golish-llm-providers.md)）
- **路径**：`backend/crates/golish-llm-providers/src/deepseek/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 DeepSeek 端点/默认 base URL 时
- DeepSeek 请求 404/路径不对（误加 `/v1`）时

## 职责

持有 DeepSeek 的 provider 级常量。DeepSeek 走 OpenAI 兼容 Chat Completions，但官方端点**不带 `/v1`**，与多数 OpenAI 兼容家不同——这是本模块存在的主要原因（避免误拼端点）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `DEEPSEEK_DEFAULT_BASE_URL` | `https://api.deepseek.com`（无 `/v1`） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | DeepSeek 常量 |

## 依赖

- 无（纯常量）

## 注意事项 / 坑

- **端点不带 `/v1`**：别"顺手"补上，会 404。
- client 创建走 `provider_trait`（OpenAI 兼容路径）；本模块只供 base URL 常量。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-llm-providers deepseek
```
