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

`runner` 只在 `AuthorizedScanTarget` 的 raw witness 复核通过后起 `nuclei`，解析输出后用同 guard 原子写 finding + passive log；`poc_match` 用 current-owner target 指纹匹配缓存 PoC，为 runner 选种模板，并跨整个模板准备持有 immutable target guard，空指纹 backfill 也在 guarded batch transaction 中完成；`severity_rank` 两者共用（critical=4 … low=1）。

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
- 模板匹配不得调用裸 `fingerprints::list_by_target` / unguarded upsert：target move 后旧 project 指纹不能跟随 stable target id；guard 在模板查询前后都要复核，漂移直接返回错误。
- targeted runner 要求 1..=512 个显式、安全的 exact template id；禁 wildcard/路径/positive tags/template_path/proxy/extra_args，固定禁 redirect、Interactsh 和 unsigned template。只有 template id 非空、安全且属于本次请求、`matched-at` 是 launch exact origin 上的绝对 URL、`info.name` 非空且 `info.severity` 合法的 JSONL 记录才算 finding；不得在 `matched-at` 缺失时回退 launch URL。畸形或语义无效 JSONL 必须让整次结果进入 partial，不能伪装 checked-empty；stdout/stderr pump 的 read/join 错误、stderr runtime/network failure及非零 exit 都是 scan failure。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-scan-runner nuclei
```
