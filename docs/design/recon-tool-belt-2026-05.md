# Recon Tool Belt 盘点 & 缺口

- **作者**: MCP-1（全栈工程师）
- **日期**: 2026-05-20
- **目的**: 回答"Recon 阶段需要什么工具 / 什么字段 / 什么流程"。配合 [`2026-05-20-agent-harness-strategy.md`](2026-05-20-agent-harness-strategy.md) §5 阶段 2 使用。

---

## 0. TL;DR

- **现有 tool manager 严格说够用**，因为有 `run_pty_cmd` 这个万能口可以调本地任何 CLI 工具
- **但"够用"≠"能上 harness"**。要装 harness 必须做 3 件事：
  1. **加 Recon 阶段约束**：限制 `run_pty_cmd` 只能跑白名单命令
  2. **包 2-3 个结构化高阶工具**：`dns_resolve` / `http_probe` 等，让 agent 拿到的是 JSON 而不是 stdout
  3. **加 barrier 工具**：`submit_recon_deliverable`，agent 调它才算 Recon 完成
- **暂不需要引入新的外部库**（如 Amass / Shodan SDK），用现有 `web_fetch` 接 crt.sh / Shodan REST API 即可

---

## 1. 现有工具能力盘点（按域分类）

### 1.1 通用文件 / shell（`tool_executors/`，写代码用，Recon 也能用一部分）

| 工具 | 现状 | Recon 是否用 |
|---|---|---|
| `read_file` / `edit_file` / `write_file` / `create_file` / `delete_file` | ✅ 完整 | ❌ Recon 不写文件 |
| `grep_file` / `list_files` | ✅ | ❌ |
| `ast_grep` / `ast_grep_replace` | ✅ | ❌ |
| **`run_pty_cmd`** | ✅ 万能 shell | **✅ 关键**——能跑 dig / nslookup / whois / subfinder / amass 等任何已安装的 CLI |
| `update_plan` | ✅ | ✅ agent 自我跟踪 |

### 1.2 Web 工具（`tool_executors/web.rs`）

| 工具 | 现状 | Recon 是否用 |
|---|---|---|
| **`web_fetch`** | ✅ GET 网页，注入 `WebFetchProvider` | ✅ HTTP 探测、crt.sh、Shodan REST 等都靠它 |

### 1.3 安全工具（`tool_executors/security.rs`）

**重要认知**：这些**不是扫描工具**，它们是**写库工具**。它们的作用是把 agent 在 shell / web_fetch 里"发现"的东西**结构化落库**。

| 工具 | 用途 | Recon 阶段用？ |
|---|---|---|
| `log_operation` | 写操作审计日志 | ✅ 每次重要动作记一笔 |
| `discover_apis` | 落库 API endpoints | ⚠️ Recon 偏早，主要给 Enumeration 阶段 |
| `save_js_analysis` | 落库 JS 分析结果 | ⚠️ 中后期 |
| **`fingerprint_target`** | 落库技术指纹 | ✅ 关键 |
| `log_scan_result` | 落库扫描结果 | ✅ 部分（被动结果） |
| `query_target_data` | 查询 target 已有数据 | ✅ Recon 开头先 query 看已知信息 |

### 1.4 Knowledge Base 工具

| 工具 | 用途 | Recon 用？ |
|---|---|---|
| `kb_query` / `kb_search` / `kb_read` / `kb_wiki` | 查内部知识库 | ✅ 让 agent 查"上次扫这个目标时记录了什么" |

### 1.5 Memory 工具

| 工具 | 用途 | Recon 用？ |
|---|---|---|
| `memory_*`（write/read/query/list/delete/purge） | agent 长期记忆 | ✅ 跨 session 复用上次 Recon 结果 |

---

## 2. Recon 阶段需要的能力 vs 现有能力的映射

| Recon 能力 | 优先级 | 现有覆盖 | 落地办法 |
|---|---|---|---|
| **DNS 解析**（A/AAAA/MX/NS/TXT/CNAME） | P0 必需 | ✅ 间接 | 先期：`run_pty_cmd("dig +short example.com A")` <br/>中期：包成 `dns_resolve(domain, record_types)` 高阶工具 |
| **子域名枚举（被动）** | P0 必需 | ✅ 间接 | 先期：`run_pty_cmd("subfinder -d example.com -silent")` 需装 subfinder<br/>替代：`web_fetch("https://crt.sh/?q=%25.example.com&output=json")` |
| **WHOIS 查询** | P1 推荐 | ✅ 间接 | `run_pty_cmd("whois example.com")` |
| **HTTP 指纹**（status / title / server / X-Powered-By） | P0 必需 | ✅ 直接 | `web_fetch(url)` 拿 headers + body，agent 解析 |
| **技术栈识别** | P1 推荐 | ✅ 间接 | `run_pty_cmd("whatweb url")` 或基于 `web_fetch` 内容启发式（如 `wp-content/` → WordPress） |
| **证书透明度（CT logs）** | P1 推荐 | ✅ 直接 | `web_fetch("https://crt.sh/?q=example.com&output=json")` |
| **Shodan / FOFA / Censys 查询** | P2 可选 | ✅ 直接 | `web_fetch("https://api.shodan.io/...?key=...")` 接 REST API |
| **ICMP / ping** | P3 一般 | ✅ 间接 | `run_pty_cmd("ping -c 1 ...")` |
| **端口扫描**（主动） | P3 偏 Enum 阶段 | ⚠️ 间接 | `run_pty_cmd("nmap -sV -T4 ...")` 但**应放到 Enumeration 阶段**，不在 Recon |
| **落库结构化数据** | P0 必需 | ✅ 直接 | `fingerprint_target` / `log_scan_result` |
| **submit_recon_deliverable barrier** | P0 必需 | ❌ **不存在** | 必须新增（见 §4） |

---

## 3. 现有工具的 3 个坑

### 坑 1：没有阶段约束 → agent 在 Recon 阶段可以瞎调

`run_pty_cmd` 没有命令白名单。理论上 agent 可以在 Recon 调 `sqlmap` / `metasploit`。需要在 harness 层做"阶段 × 工具 × 命令"三维白名单。

### 坑 2：`run_pty_cmd` 输出是无结构 stdout

agent 拿到 `nslookup example.com` 的输出还要自己 parse，容易丢字段、容易解析错。

**最简办法**：包装"Recon 高阶工具"——内部仍调 `run_pty_cmd` 或 `web_fetch`，但出口 JSON 结构化。例如：

```rust
fn dns_resolve(domain: &str, record_types: &[&str]) -> DnsResolveResult {
    // 内部跑 dig，解析输出，返回结构化
    DnsResolveResult {
        a: vec!["93.184.216.34"],
        aaaa: vec![],
        mx: vec![...],
        ...
    }
}
```

不需要重写底层，只是给 LLM 一个**更窄、更可预测**的工具签名。

### 坑 3：没有 barrier 工具

agent 现在"完成"是靠 LLM 自己说一句话。需要新增 `submit_recon_deliverable(json)`，让 LLM **不调它就不算完**。barrier 工具的实现 = 解析 JSON + 跑 gate + 返回 ack/blocking_reasons。

---

## 4. Recon 阶段流程（含 harness 约束）

```text
┌────────────────────────────────────────────────────────────┐
│ harness 注入 Phase Charter 到 LLM system prompt           │
│   - 你是 pentester_recon                                  │
│   - 输入: scope, known_assets                             │
│   - 必须用 submit_recon_deliverable 收尾                  │
│   - allowed_tools: dns_resolve, http_probe, web_fetch,    │
│     subdomain_enum, whois_lookup, fingerprint_target,     │
│     log_operation, log_scan_result,                       │
│     submit_recon_deliverable                              │
│   - forbidden: sqlmap / nmap -sS / metasploit / 任何主动   │
└────────────────────────────────────────────────────────────┘
                            ↓
┌────────────────────────────────────────────────────────────┐
│ agent 循环：                                                │
│   1. query_target_data 看已有信息                          │
│   2. for each target:                                      │
│        a. dns_resolve → 解析记录                          │
│        b. subdomain_enum → 找子域                          │
│        c. whois_lookup → 注册信息                          │
│        d. http_probe → HTTP 指纹                          │
│        e. fingerprint_target ← 落库                       │
│        f. log_operation ← 写审计日志                      │
│   3. 收齐后 → submit_recon_deliverable(json)              │
└────────────────────────────────────────────────────────────┘
                            ↓
┌────────────────────────────────────────────────────────────┐
│ harness barrier 处理：                                     │
│   - parse JSON → 失败则要求重新提交                       │
│   - validate_recon_gate(deliverable)                      │
│     - in_scope?                                            │
│     - 域名有 DNS 结果或 skip 原因?                         │
│     - 每个 open port 有 service?                          │
│     - HTTP 有 tech?                                       │
│     - evidence 非空?                                       │
│   - 过 → 进 Enumeration                                   │
│   - 不过 → blocking_reasons 回灌, agent 补                │
└────────────────────────────────────────────────────────────┘
```

---

## 5. Recon Deliverable 字段全集（补充版）

这是基于旧 `harness-recon-mvp.md` 草案扩展出的字段全集；该旧草案已移除，本文只保留工具盘点和字段参考价值。

```rust
struct ReconDeliverable {
    // 基础（必填）
    target: ReconTarget,             // { value, kind: domain|ip|url|cidr }
    scope: ScopeStatus,              // in_scope | out_of_scope | unknown
    gate_status: ReconGateStatus,    // pending | passed | blocked

    // 网络层
    dns_records: Vec<DnsRecord>,             // A/AAAA/MX/NS/TXT/CNAME
    resolved_ips: Vec<ResolvedIp>,
    open_ports: Vec<OpenPort>,               // P3，可放 Enum 阶段
    services: Vec<ServiceFingerprint>,

    // 应用层
    http_services: Vec<HttpService>,         // url / status / title / server header
    technologies: Vec<TechnologyFinding>,    // CMS / framework / library

    // **新增 3 个之前漏的字段**
    subdomains: Vec<Subdomain>,              // 子域名清单
    whois: Option<WhoisRecord>,              // 注册信息
    certificate_transparency: Vec<CtEntry>,  // crt.sh 等 CT log 抓的证书

    // 证据 / 跳过
    evidence_items: Vec<EvidenceItem>,       // 所有发现的证据 id 引用
    skipped_checks: Vec<SkippedCheck>,       // 哪些 check 跳了 + 原因
}
```

下面补充旧草案遗漏字段的 schema：

```rust
struct Subdomain {
    name: String,                    // foo.example.com
    source: SubdomainSource,         // ct_log | passive_dns | brute_force
    resolved_ips: Vec<String>,       // 可选
    evidence_id: String,
}

struct WhoisRecord {
    registrar: Option<String>,
    created_at: Option<String>,
    expires_at: Option<String>,
    name_servers: Vec<String>,
    raw_summary: String,
    evidence_id: String,
}

struct CtEntry {
    issuer: String,
    not_before: String,
    not_after: String,
    common_name: String,
    sans: Vec<String>,
    evidence_id: String,
}
```

---

## 6. 建议落地顺序（叠在策略文档 §5 路线图之上）

| 步骤 | 内容 | 工时 | 是否需要新依赖 |
|---|---|---|---|
| 6.1 | 写 `phase_charter_recon.md`（注入用） | 1 天 | 无 |
| 6.2 | 在 harness 层加"Recon Tool Belt"白名单约束 | 0.5 天 | 无 |
| 6.3 | 包 3 个高阶工具：`dns_resolve` / `http_probe` / `subdomain_enum` | 1-2 天 | **可选**：装 `subfinder` 或纯走 crt.sh `web_fetch` |
| 6.4 | 新增 barrier 工具 `submit_recon_deliverable` + gate 函数 | 1-2 天 | 无 |
| 6.5 | DB schema：把 `ReconDeliverable` 字段全集映射到现有 targets 表的扩展或新表 | 0.5 天 | 无 |
| 6.6 | 跑端到端 demo（用 `example.com` 这种安全靶子） | 0.5 天 | 无 |

**所有步骤都不需要引入新的外部 Rust 依赖**。可选的系统级 CLI 工具（subfinder / whois / dig）可作为运行时检测，不在 Cargo.toml 里。

---

## 7. 关键决策点

| # | 决策 | 选项 | 建议 |
|---|---|---|---|
| T1 | 子域名枚举源 | A) 装 subfinder 走 shell <br/>B) 纯走 crt.sh `web_fetch` <br/>C) 接 Shodan API | **先 B，后续叠 A**——零依赖、纯被动 |
| T2 | `run_pty_cmd` 白名单实现位置 | A) tool_policy 模块加阶段维度 <br/>B) harness 层包一层调用 | **A**——Golish 已有 `tool_policy/manager.rs` |
| T3 | Recon DB 持久化 | A) 扩展现有 targets 表加 jsonb 列 <br/>B) 新建 recon_deliverables 表 | **B**——单独表方便查询和版本化 |
| T4 | `dns_resolve` 等高阶工具放哪 | A) 新建 `tool_executors/recon.rs` <br/>B) 散到 web.rs / shell.rs | **A**——按阶段聚合更清晰 |
| T5 | 端口扫描放 Recon 还是 Enum | A) Recon 内 <br/>B) Enum 独立阶段 | **B**——主动扫描应该单独受权 |

---

## 8. 风险

| 风险 | 缓解 |
|---|---|
| 高阶工具封装太薄，与 `run_pty_cmd` 重复 | 工具签名保持窄（只接受 typed 参数），不要做"任意 shell 转发" |
| crt.sh 限流 | `web_fetch` 已有 timeout / retry，加缓存即可 |
| Recon 阶段把 Enum 工作做了（如端口扫描） | Phase Charter 明确写"port_scan 属于 Enumeration"+ Tool Belt 不放该工具 |
| 系统未安装 subfinder/whois | 工具调用前先 `which` 检测，没装就 fallback web_fetch 路径 |

---

## 9. 与其他文档关系

```text
2026-05-20-agent-harness-strategy.md     ← 总策略
    ├── <future recon deliverable design> ← 待信息收集闭环稳定后重写
    ├── recon-tool-belt-2026-05.md       ← 本文档：工具盘点 / 流程 / 字段补全
    └── <future harness plan>             ← 旧 recon-first 计划已删除
```

---

## 10. 变更日志

| 日期 | 作者 | 变更 |
|---|---|---|
| 2026-05-20 | MCP-1 | 初稿——工具盘点 + 缺口 + 流程 + 字段补全 + 落地顺序 |
