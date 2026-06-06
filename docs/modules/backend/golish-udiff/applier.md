# golish-udiff / applier

> **一句话职责**：`UdiffApplier` 应用子系统——把 unified diff 按「精确 → 归一空白 → 模糊相似窗口」策略顺序应用到文件；按主题拆成 errors/direct/fuzzy/apply 四个子模块。

- **类型**：目录模块（属于 crate [`golish-udiff`](../golish-udiff.md)）
- **路径**：`backend/crates/golish-udiff/src/applier/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 diff 应用的匹配策略（精确 / 归一空白 / 模糊）或其执行顺序时
- 改 hunk 匹配失败的错误/结果类型（`ApplyResult`）时
- agent `edit_file`/diff 应用"匹配不上"或"模糊匹配过宽"时

## 职责

`UdiffApplier` 是单元 struct，作为所有应用函数的命名空间；各子模块各贡献一个 `impl UdiffApplier` 块。`apply` 是顶层 dispatcher，按 direct（精确 + 归一空白）→ fuzzy（`similar::TextDiff` 相似窗口）顺序尝试。

## 公开接口

| 符号 | 说明 |
|---|---|
| `UdiffApplier` | 单元 struct（应用函数命名空间） |
| `ApplyResult` | 应用结果（public） |

各匹配策略方法经子模块 `impl UdiffApplier` 贡献（`apply` 为顶层入口）。

## 关键文件

| 文件 | 作用 |
|---|---|
| `apply.rs` | 顶层 dispatcher（按序跑策略） |
| `direct.rs` | 精确 + 归一空白匹配 |
| `fuzzy.rs` | 相似窗口模糊匹配（`similar::TextDiff`） |
| `errors.rs` | `ApplyResult`（public）+ `HunkApplyError`（内部） |

## 依赖

- `similar`（文本 diff/相似度）

## 注意事项 / 坑

- 策略**有序回退**：精确不中才退到归一空白、再退模糊——改顺序会改变"哪个 hunk 命中哪里"的行为，需配套更新 `tests/`。
- 模糊匹配有相似度窗口，过宽会误改；调阈值要跑全量 `tests`。
- `HunkApplyError` 是内部类型，对外只暴露 `ApplyResult`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-udiff applier
```
