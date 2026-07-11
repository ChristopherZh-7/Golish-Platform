# Browser 批次并发与工具内 AI 显式启用

## 背景

`browser_collect_js_api(target_urls=[...])` 在 2026-07-03 的批次设计中仍按 root 串行调用单目标实现。Test1 的 44-root 实跑虽全部闭合，但 browser 一步耗时 548 秒；当 worklist 扩展到数百个 exact Web Origin 时，串行总耗时会线性放大。

同一实跑还暴露了两个名称相近但语义分离的开关：调用方传了 `ai_assist=false`，Node helper 不再返回外层 recipe context，但 Rust 中未公开的 `ai` 默认仍为 `true`，因此两个 root 仍发起了工具内 one-shot LLM 请求。这违反 Enumeration 的 deterministic-first 合同。

本设计只改变 batch 调度和 AI 启用条件；单 root 的授权、exact-origin、evidence、三 sibling outcome 原子发布和 partial/error 语义保持不变。

## 决策

### 1. Browser batch 使用有界并发

- `browser_collect_js_api` schema 新增 `batch_concurrency`。
- 默认值与硬上限均为 4；小于 1 的输入归一为 1，大于 4 的输入归一为 4，且实际并发不超过 accepted root 数。
- accepted root 通过同一个 `execute_single` 合同执行，不复制或弱化单目标安全检查。
- 调度允许完成顺序不同，但聚合结果按原输入 index 排序。
- 单项返回错误只进入该项 `errors` 并尝试写它自己的 error marker，不取消已运行或待运行的 sibling。
- batch 响应显式返回实际 `batch_concurrency`。

这里不使用 `tokio::spawn`：`buffer_unordered` 在当前 tool task 内轮询 futures，保留现有 task-local session、operation、organization 和 tool-output context。

### 2. 工具内 AI 由显式 `ai=true` 启用

- schema 正式暴露 `ai: boolean`，默认 `false`。
- `ai=true` 只是必要条件；`ai_assist=false` 是 deterministic-only 硬开关，始终覆盖 `ai=true`。
- 因此 Enumeration 默认调用 `ai_assist=false` 时不会产生隐藏 LLM 请求，也不会触发 AI recipe 二次 browser helper。
- 显式启用时，既有最多 3 轮、同源 recipe sanitizer、60 秒默认 AI deadline 和失败降级语义保持不变。
- `js_extract_apis` 已是 `ai=false` 默认，本设计使 collect/extract 两端默认一致。

## 不变量

- 每个 exact origin 仍先写 JS / JSAPI / PARAM 三个 attempt partial marker，再由单目标路径准备并原子发布 terminal siblings。
- 每个 helper 仍受单 root hard deadline 约束；并发不把超时或 partial 转成 checked-empty。
- target_id / organization / workspace / scope / exact-origin 授权仍在每个单目标路径独立解析和重验。
- batch 中一个 root 的 capture、业务行或 outcome 失败不得改变其他 root 的结果。

## 验证

- 红测先证明并发解析、调度 helper 和 AI 开关不存在。
- 调度测试以 8 个 future + barrier 验证峰值恰为 4、输出 index 恢复为 0..7，且其中一个 `Err` 不取消最后一个 sibling。
- schema 测试验证 `batch_concurrency default=4/max=4` 与 `ai default=false`。
- AI 纯函数测试验证缺省关闭、显式开启，以及 `ai_assist=false` 覆盖 `ai=true`。
- 继续运行现有 browser batch、deadline、outcome、authorization focused tests，确保单 root 合同无回归。

## 与旧设计的关系

本文件收紧 [2026-07-03 enumeration batch 设计](2026-07-03-enumeration-batch-and-terminal-coverage.md) 中“内部循环复用单 target”的执行细节：外部仍是一批一个工具调用、每 root 独立落账，但 accepted roots 改为最多 4 路有界并发。其他批次和终态规则不变。
