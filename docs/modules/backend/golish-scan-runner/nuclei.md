# golish-scan-runner / nuclei

> **一句话职责**：只读 Nuclei template selector——用 current-owner exact-origin fingerprint 从本地 KB 选择安全、去重的 template id 和可追溯 rationale。

- **类型**：目录模块（属于 crate [`golish-scan-runner`](../golish-scan-runner.md)）
- **路径**：`backend/crates/golish-scan-runner/src/nuclei/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改指纹→Nuclei template 选择、template id 安全策略或 rationale 时

## 职责

`poc_match` 加载 current in-scope/project-bound target guard 与 exact `web_origin_id`，只读该 origin 有来源映射的
有界 fingerprint，从本地 `vuln_kb_pocs` 的 strict-CVE、`cve`-tagged Nuclei HTTP/SSL 模板中选择与 CVE 相同的安全 template id，并返回
具体 fingerprint/PoC rationale。选择完成前后复核同一 guard。模块不 backfill、不启动
Nuclei、不写 passive log/Finding；执行与 evidence-first landing 由
`golish-pentest-app` stage adapter 拥有。

## 公开接口

| 符号 | 说明 |
|---|---|
| `select_nuclei_templates_for_origin` | current-owner exact-origin fingerprint → 去重 template selection |
| `NucleiTemplateSelection` | template id + rationale 集合 |
| `NucleiTemplateRationale` | fingerprint id/name/version + PoC id/CVE/name/severity |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | selector re-export |
| `poc_match.rs` | current-owner 查询、纯选择策略、template id 校验与去重 |

## 依赖

- crate 内；DB 只读（fingerprints + 本地 PoC KB）

## 注意事项 / 坑

- 不得调用裸 `fingerprints::list_by_target` 或做 target-global fallback/backfill；target move 后旧 project 指纹不能跟随 stable target id，同 target 的 sibling origin 也不能互借指纹。
- 只接受 `poc_type=nuclei` + `source=nuclei_template`；模板 `id:` 只在前 20 行读取，限 ASCII `[A-Za-z0-9._-]` 且最多 128 bytes。content 必须明确包含顶层 `http` / legacy `requests` / `ssl`，并拒绝混入 code/javascript/headless/file/tcp/network/dns/workflow/websocket/whois/flow。
- 同 template id 的多个 PoC/fingerprint 合并成一个 selection，rationale 去重并稳定排序。
- fingerprint、匹配 PoC、selection 和 rationale 都有硬上限，超限 fail closed，不把静默截断伪装成完整选择。
- selector 结果只是执行计划输入，不是漏洞证据；不得在这里恢复 process/Finding 写入。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-scan-runner nuclei
```
