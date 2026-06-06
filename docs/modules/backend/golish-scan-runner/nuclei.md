# golish-scan-runner / nuclei

> **一句话职责**：Nuclei 定向扫描 + 指纹→PoC 匹配引擎——`run_nuclei_targeted` 起 `nuclei` 进程并解析/落库，`match_pocs_for_target` 用指纹匹配缓存 PoC 为 runner 种模板。

- **类型**：目录模块（属于 crate [`golish-scan-runner`](../golish-scan-runner.md)）
- **路径**：`backend/crates/golish-scan-runner/src/nuclei/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 nuclei 定向扫描（进程起停、输出解析、结果落库）时
- 改指纹→PoC 匹配（用指纹选模板）或 severity 排序时

## 职责

`runner` 起 `nuclei` 进程、解析输出、持久化结果；`poc_match` 用 target 指纹匹配缓存 PoC，为 runner 选种模板；`severity_rank` 两者共用（critical=4 … low=1）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_nuclei_targeted` / `NucleiScanOptions` | 定向扫描入口 + 选项 |
| `match_pocs_for_target` | 指纹 → PoC 匹配（种模板） |
| `severity_rank`（`pub(super)`） | severity 字符串 → 数值排序 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export + `severity_rank` |
| `runner.rs` | 起 nuclei + 解析 + 落库 |
| `poc_match.rs` | 指纹 → 缓存 PoC 匹配 |

## 依赖

- crate 内；进程（`nuclei` 二进制）、DB（结果落库）

## 注意事项 / 坑

- 依赖系统装了 `nuclei`；缺二进制要优雅报错（非 panic）。
- 指纹→PoC 匹配先于 runner（用匹配结果种模板），改顺序会变扫描范围。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-scan-runner nuclei
```
