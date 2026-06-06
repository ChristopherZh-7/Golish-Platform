# golish-llm-providers / xiaomi

> **一句话职责**：Xiaomi MiMo Token Plan provider 常量与协议助手——`XiaomiRegion` 多集群（CN/SGP/AMS Token Plan + PayAsYouGo 全球），OpenAI 兼容与 Anthropic 兼容端点在不同路径前缀，一把 key 共享三集群。

- **类型**：目录模块（属于 crate [`golish-llm-providers`](../golish-llm-providers.md)）
- **路径**：`backend/crates/golish-llm-providers/src/xiaomi/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Xiaomi MiMo 端点/区域选择（Token Plan CN/SGP/AMS vs 按量付费）时
- 改 OpenAI 兼容 / Anthropic 兼容双协议路径前缀时
- MiMo key 在某集群被 401 时

## 职责

持有 Xiaomi MiMo 的 provider 常量与协议助手。`XiaomiRegion` 区分 Token Plan 三区域集群（`tp-…` key）与按量付费全球端点（`sk-…` key）；OpenAI 兼容与 Anthropic 兼容端点在不同路径前缀。设计见 `docs/design/2026-05-27-add-xiaomi-mimo-provider.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `XiaomiRegion`（`Cn`/`Sgp`/`Ams`/`PayAsYouGo`，`from_settings`） | 端点集群（带别名解析，未知回退 `Cn`） |
| （OpenAI / Anthropic 兼容端点 URL 助手） | 双协议路径前缀 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `XiaomiRegion` + 端点/协议助手 |

## 依赖

- 无（纯常量 + 解析）

## 注意事项 / 坑

- **key 类型与集群绑定**：`tp-…`（Token Plan）三集群；`sk-…`（按量付费）仅 `api.xiaomimimo.com`——按量付费 key 在 Token Plan 端点会 401（2026-05-27 实测）。
- OpenAI 兼容与 Anthropic 兼容端点路径前缀不同；改端点别混前缀。
- 关联 crate 卡：`xiaomi-mimo-provider` feature 在 feature_list 处于 `blocked`（待 tool-use 兼容层 + 真机 E2E）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-llm-providers xiaomi
```
