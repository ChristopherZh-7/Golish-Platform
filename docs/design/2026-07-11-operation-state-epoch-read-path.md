# Operation state epoch-only read path

## 背景

`operation_state::get` 会读取并反序列化整行，其中 `state_blob` 保存 harness resume 与
route checkpoint。该 JSONB 可以增长到数十 MB。Enumeration 的 preflight、browser、JS
extractor 和 route producer 在批次热路径中只需要判断当前 operation 是否仍属于同一个
stage attempt，却为每个 root 重复读取整个 `state_blob`，会放大连接池占用、数据库传输和
JSON 反序列化成本。

## 决策

在 `golish-db::repo::operation_state` 增加窄接口：

- `OperationEpochRow` 只包含 `operation_id`、`current_stage`、`stage_started_at`、
  `superseded_by`、`engagement_org_id`。
- `get_epoch(pool, operation_id)` 的 SQL 只选择上述五列，明确排除 `state_blob` 与三个
  cursor/evidence 字段。
- Enumeration preflight 的初始读取和重验、browser/JS 的 active-stage 读取、route
  checkpoint identity 读取统一改用 `get_epoch`。

完整 resume/cursor/checkpoint 读写仍使用既有专用路径或 `get`；本次不改变
`operation_state` schema，不改 stage epoch 的定义，也不缓存 epoch。

## 语义不变量

- active stage 仍要求 `current_stage == "enumeration"` 且 `superseded_by IS NULL`。
- restart/advance 检测仍精确比较 `stage_started_at`。
- preflight 仍比较 operation id、engagement org，并验证 organization 位于 engagement
  subtree。
- route checkpoint 的 run/session/operation/stage/authorization/exact-origin/plan identity
  均保持原样；只有 active operation 行的读取变窄。
- 本改动不引入 batch epoch snapshot、信号量或网络并发调整。

## 验证

- SQL 契约测试证明五个 epoch 字段存在且大字段/cursor 字段不在查询中。
- row serde 测试覆盖五字段映射，并证明序列化形状不含 `state_blob`。
- 运行 `golish-db` operation-state 测试与四个 pentest bridge 的 focused tests。
- 运行相关 crate clippy、workspace fmt check 与 diff check。
