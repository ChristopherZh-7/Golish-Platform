# golish-intel-providers / shodan

> **一句话职责**：Shodan provider——`IntelProvider`，`key` query string 认证，Site 映射到 `/shodan/host/search`（ip+port+org+http+ssl+ASN+location），Domain/Cert/Asn/Cidr 经重写为 Shodan DSL 走同端点。

- **类型**：目录模块（属于 crate [`golish-intel-providers`](../golish-intel-providers.md)）
- **路径**：`backend/crates/golish-intel-providers/src/shodan/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Shodan 查询/映射、DSL 重写（把用户输入改写成 Shodan 子句）时

## 职责

`IntelProvider` for Shodan（API `https://api.shodan.io/shodan/host/search`）。`key` query string 认证，DSL 明文发送（不 base64）。Site 映射到 host/search 全 banner（ip+port+org+http+ssl+ASN+location）；Domain/Cert/Asn/Cidr 通过把用户输入重写成对应 Shodan DSL 子句走同端点。付费计划限速 1 req/s（免费仅 host lookup，无 search）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ShodanProvider`（impl `IntelProvider`） | Shodan provider |
| 支持 QueryType | Site（`/host/search`）+ Domain/Cert/Asn/Cidr（DSL 重写） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `IntelProvider` impl |
| `client.rs` | HTTP + key query |
| `mapper.rs` / `types.rs` | 响应映射 / wire（`ShodanSearchEnvelope`/`ShodanMatch`/…） |

## 依赖

- crate 内 `shared`、`error`、`types`、`IntelProvider`、`async-trait`

## 注意事项 / 坑

- DSL 明文发送（不 base64）；免费账户**无 search**（仅 host lookup），付费才 search、1 req/s。
- 多 QueryType 复用同一 search 端点，靠输入重写——加新类型在重写层处理。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-intel-providers shodan
```
