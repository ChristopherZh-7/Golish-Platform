# golish-intel-providers / fofa

> **一句话职责**：FOFA（鹰图）provider 实现——`IntelProvider`，支持 Site/Domain/Cert；认证用 `"<email>|<key>"` 合并格式（vault 单串存），防御性限速 2 req/s。

- **类型**：目录模块（属于 crate [`golish-intel-providers`](../golish-intel-providers.md)）
- **路径**：`backend/crates/golish-intel-providers/src/fofa/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 FOFA 查询/映射、`email|key` 凭据拆分（`client::split_credentials`）时
- FOFA 401/凭据格式问题时

## 职责

`IntelProvider` for FOFA（API `https://fofa.info/api/v1/search/all`）。FOFA v1 需 email + API key，但 vault 一条只存单串，故用 `"<email>|<key>"` 规范格式，内部 `split_credentials` 拆分（Settings UI 必须提示用户按此格式填）。支持 Site/Domain/Cert（FOFA DSL）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `FofaProvider`（impl `IntelProvider`） | FOFA provider |
| 支持 QueryType | Site / Domain（`domain="..."`）/ Cert（`cert="..."`） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `IntelProvider` impl |
| `client.rs` | HTTP + `split_credentials`（`email|key` 拆分） |
| `mapper.rs` / `types.rs` | 响应映射 / wire 类型 |

## 依赖

- crate 内 `shared`（限速/key）、`error`、`types`、`IntelProvider`、`async-trait`

## 注意事项 / 坑

- **凭据格式 `"<email>|<key>"`**：vault 单串存 email+key，靠 `|` 拆；Settings UI 文案必须指明这格式，否则认证失败。
- FOFA 无 per-second 限速但有每日 F 点配额；仍默认 pace 2 req/s 防批量过猛。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers fofa
```
