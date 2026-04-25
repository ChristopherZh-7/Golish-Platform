# Golish Platform: 数据库表 & 工具映射文档

## 数据库表总览

### A. AI 会话与执行追踪

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `sessions` | AI 会话主体 | id, status, workspace_path, model, provider, project_path | 后端创建会话时 | 前端分析、审计 |
| `session_data` | 会话详情(JSON) | session_id, messages, transcript, distinct_tools | 后端更新 | 分析/恢复 |
| `tasks` | 用户提示词任务 | id, session_id, input, result, status | AgentBridge | 任务追踪 |
| `subtasks` | Agent 分解的子任务 | id, task_id, agent, result, status | AgentBridge | 任务追踪 |
| `tool_calls` | 每次工具调用记录 | call_id, session_id, name, args, result, status, duration_ms | db_tracking.rs | 审计/分析 |
| `terminal_logs` | 终端 I/O 记录 | session_id, stream(stdin/stdout/stderr), content | db_tracking.rs | 审计 |
| `search_logs` | Web/API 搜索记录 | session_id, engine, query, result | db_tracking.rs | 审计 |
| `message_chains` | LLM 对话链 + token 统计 | session_id, agent, model, tokens_in/out, cost_in/out_usd | db_tracking.rs | 成本分析 |
| `msg_logs` | 逐条 LLM 消息日志 | session_id, agent, msg_type, message, result | db_tracking.rs | 细粒度审计 |
| `agent_logs` | Agent 活动日志 | session_id, agent_type, level, message | db_tracking.rs | 调试/审计 |
| `execution_plans` | 执行计划(多步骤) | session_id, title, steps(JSON), status, current_step | PlanManager | 任务续接 |

### B. 前端会话与 UI 持久化

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `conversations` | 前端聊天标签页 | id, title, ai_session_id, project_path | 前端创建 | 前端恢复 |
| `chat_messages` | 聊天消息 | id, conversation_id, role, content, tool_calls | 前端存储 | 前端恢复 |
| `timeline_blocks` | 时间线事件块 | id, session_id, block_type, data(JSON) | 前端存储 | 前端恢复 |
| `terminal_state` | 终端状态(滚动缓冲) | session_id, conversation_id, scrollback | 前端存储 | 前端恢复 |
| `workspace_preferences` | 工作区偏好 | project_path, active_conversation_id, ai_model | 前端存储 | 前端恢复 |

### C. 渗透测试核心数据 ⭐

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `targets` | 渗透测试目标 | name, target_type(domain/ip/cidr/url), value, scope, real_ip, cdn_waf, http_title, webserver | 前端 UI + AI 工具 | 前端 UI + AI |
| `target_groups` | 目标分组 | name, project_path | 前端 UI | 前端 UI |
| `findings` | 发现的漏洞 | title, sev(critical→info), cvss, url, target, description, steps, remediation, tool, status | 前端 UI + AI 工具 | 前端 UI + AI |
| `notes` | 实体附属笔记 | entity_type, entity_id, content, color | 前端 UI | 前端 UI |
| `vault_entries` | 凭证保险柜 | name, entry_type, value(混淆), username, tags | 前端 UI | 前端 UI |

### D. 安全分析数据 ⭐

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `target_assets` | 子域名/IP/服务发现 | target_id, asset_type, value, port, protocol, service, version | AI 工具: `log_operation` | 前端 UI + AI: `query_target_data` |
| `api_endpoints` | API 路由发现 | target_id, url, method, path, params, auth_type, risk_level, tested | AI 工具: `discover_apis` | 前端 UI + AI: `query_target_data` |
| `js_analysis_results` | JS 文件分析结果 | target_id, url, frameworks, endpoints_found, secrets_found | AI 工具: `save_js_analysis` | 前端 UI + AI: `query_target_data` |
| `fingerprints` | 技术指纹(CMS/WAF/框架) | target_id, category, name, version, confidence, cpe | AI 工具: `fingerprint_target` | 前端 UI + AI: `query_target_data` |
| `directory_entries` | 目录枚举结果(ffuf等) | target_id, url, status_code, content_length, tool | AI 工具: `log_operation` | 前端 UI |
| `passive_scan_logs` | 被动扫描日志(XSS/SQLi等) | target_id, test_type, payload, url, result, severity | AI 工具: `log_scan_result` | 前端 UI + AI: `query_target_data` |
| `sensitive_scan_results` | 敏感文件扫描结果 | base_url, probe_path, status_code, is_confirmed, ai_verdict | 后端扫描器 | 前端 UI |
| `sensitive_scan_history` | 扫描历史记录 | base_url, wordlist_id, probe_count, hit_count | 后端扫描器 | 前端 UI |

### E. 记忆与向量存储

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `memories` | 长期知识记忆 | content, mem_type(observation/technique/vulnerability), embedding(vector), doc_type | AI 自动(gatekeeper) + AI 工具(store_memory) | AI 工具(search_memories) |
| `vector_store_logs` | 向量操作审计 | session_id, action, query, result_count | db_tracking.rs | 审计 |

### F. 知识库

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `wiki_pages` | Wiki 知识页面 | path, title, category, tags, content | AI 工具: `write_knowledge` | AI 工具: `search_knowledge_base`, `read_knowledge` |
| `vuln_kb_links` | CVE→Wiki 关联 | cve_id, wiki_path | AI 工具 | 前端 UI |
| `vuln_kb_pocs` | PoC 脚本 | cve_id, name, poc_type, language, content | AI 工具: `save_poc` | 前端 UI + AI |
| `kb_research_log` | CVE 研究日志 | cve_id, session_id, turns(JSON) | 知识库研究会话 | 前端 UI |

### G. 漏洞情报

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `vuln_feeds` | 漏洞情报源配置 | id, name, feed_type, url, enabled | 前端配置 | 后端抓取 |
| `vuln_entries` | CVE 漏洞条目 | cve_id, title, description, sev, cvss_score, affected_products | 后端抓取 | 前端 UI + AI: `ingest_cve` |
| `vuln_scan_history` | CVE 扫描历史 | cve_id, target, result(vulnerable/not_vulnerable) | 后端扫描 | 前端 UI |

### H. 其他

| 表名 | 用途 | 主要字段 | 数据来源 | 查询方 |
|------|------|---------|----------|--------|
| `audit_log` | 审计日志 | action, category, details, entity_type | 各种操作 | 前端 UI |
| `sitemap_store` | 站点地图数据(JSON) | name, data, project_path | 前端 | 前端 |
| `methodology_projects` | 方法论项目实例(JSON) | data, project_path | 前端 | 前端 |
| `pipelines` | 自动化管道(JSON) | data, project_path | 前端 | 前端 |
| `recordings` | 终端录屏 | title, session_id, events(JSON), duration_ms | 前端 | 前端 |
| `scan_queue` | ZAP 扫描队列 | url, scan_id, progress, status, alerts | ZAP 集成 | 前端 UI |
| `custom_passive_rules` | 自定义被动规则 | name, pattern, scope, severity, enabled | 前端 UI | 后端扫描 |
| `prompt_templates` | 提示词模板覆盖 | template_name, content, is_active | 前端 UI | 后端 AI |
| `screenshots` | 浏览器截图 | url, file_path, size_bytes | 后端 | 前端 UI |

---

## AI 工具完整清单

### 文件操作工具

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `read_file` | path, line_start?, line_end? | 文件内容 | ❌ 不存储 |
| `write_file` | path, content | 成功/失败 | ⚠️ memories (摘要, technique) |
| `create_file` | path, content | 成功/失败 | ⚠️ memories (摘要, technique) |
| `edit_file` | path, old_text, new_text | 成功/失败 | ⚠️ memories (摘要, technique) |
| `delete_file` | path | 成功/失败 | ❌ 不存储 |
| `list_files` | path, pattern?, depth? | 文件列表 | ❌ 不存储 |
| `list_directory` | path | 目录内容 | ❌ 不存储 |
| `grep_file` | pattern, path?, include? | 匹配行 | ❌ 不存储 |

### 代码搜索工具

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `ast_grep` | pattern, language?, path? | 匹配结果 | ❌ 不存储 |
| `ast_grep_replace` | pattern, replacement, language? | 替换结果 | ❌ 不存储 |

### Shell 命令工具

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `run_command` | command, cwd?, timeout? | 命令输出 | ✅ memories (technique) + tool_calls |
| `run_pty_cmd` | command, cwd?, timeout? | PTY 输出 | ✅ memories (technique) + tool_calls |

### 记忆工具

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `search_memories` | query, category?, limit? | 记忆列表 | ❌ (读操作) |
| `store_memory` | content, category?, scope?, tags? | 确认 | ✅ memories (observation) |
| `list_memories` | category?, limit? | 最近记忆 | ❌ (读操作) |
| `search_code` | query, language?, limit? | 代码片段 | ❌ (读操作) |
| `save_code` | content, language?, description? | 确认 | ✅ memories (doc_type=code) |
| `search_guide` | query, limit? | 指南 | ❌ (读操作) |
| `save_guide` | content, title?, tags? | 确认 | ✅ memories (doc_type=guide) |

### 知识库工具

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `search_knowledge_base` | query, category?, limit? | Wiki 页面 | ❌ (读操作) → `wiki_pages` |
| `write_knowledge` | path, title, content, category? | 确认 | ✅ `wiki_pages` |
| `read_knowledge` | path | 页面内容 | ❌ (读操作) → `wiki_pages` |
| `ingest_cve` | cve_id, title, description, severity | 确认 | ✅ `vuln_entries` |
| `save_poc` | cve_id, name, content, poc_type?, language? | 确认 | ✅ `vuln_kb_pocs` |
| `list_cves_with_pocs` | - | CVE 列表 | ❌ (读操作) |
| `list_unresearched_cves` | limit? | CVE 列表 | ❌ (读操作) |
| `poc_stats` | - | 统计数据 | ❌ (读操作) |

### 安全分析工具 ⭐

| 工具名 | 输入 | 输出 | 目标数据库表 |
|--------|------|------|-------------|
| `log_operation` | target_id, operation_type, data | 确认 | ✅ `target_assets` + `audit_log` |
| `discover_apis` | target_id, endpoints[] | 确认 | ✅ `api_endpoints` |
| `save_js_analysis` | target_id, url, frameworks?, endpoints?, secrets? | 确认 | ✅ `js_analysis_results` |
| `fingerprint_target` | target_id, category, name, version?, confidence? | 确认 | ✅ `fingerprints` |
| `log_scan_result` | target_id, test_type, url, payload?, result, severity? | 确认 | ✅ `passive_scan_logs` |
| `query_target_data` | target_id, sections? | 聚合数据 | ❌ (读操作) → 多个结构化表 |

### Web 搜索工具 (Tavily, 动态注册)

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `tavily_search` | query | 搜索结果 | ⚠️ memories (observation) |
| `tavily_extract` | url | 页面内容 | ⚠️ memories (observation) |
| `web_search` | query | 搜索结果 | ⚠️ memories (observation) |
| `web_fetch` | url | 页面内容 | ⚠️ memories (observation) |

### 计划工具

| 工具名 | 输入 | 输出 | 数据库持久化 |
|--------|------|------|-------------|
| `update_plan` | plan JSON | 确认 | ✅ `execution_plans` (via PlanManager) |

---

## 数据存储状态分析

### ✅ 已有结构化存储的工具

这些工具的输出已经写入结构化表，可以直接查询：

```
log_operation       → target_assets
discover_apis       → api_endpoints
save_js_analysis    → js_analysis_results
fingerprint_target  → fingerprints
log_scan_result     → passive_scan_logs
write_knowledge     → wiki_pages
ingest_cve          → vuln_entries
save_poc            → vuln_kb_pocs
query_target_data   ← 读取多个结构化表 (已实现！)
```

### ⚠️ 只存入 memories 的工具（可以改进）

这些工具的输出现在只通过 gatekeeper 存入 `memories` 表的纯文本字段，
但理想情况下应该同时写入结构化表：

| 工具 | 当前存储 | 理想结构化表 | 改进方式 |
|------|---------|-------------|---------|
| `run_command` (nmap) | memories.content (text) | `target_assets` (端口/服务) | 解析 nmap 输出 → 写入 target_assets |
| `run_command` (nuclei) | memories.content (text) | `findings` (漏洞) | 解析 nuclei JSON → 写入 findings |
| `run_command` (ffuf/gobuster) | memories.content (text) | `directory_entries` | 解析目录枚举输出 → 写入 directory_entries |
| `run_command` (httpx) | memories.content (text) | `targets` (http_title/webserver等) | 解析 httpx → 更新 targets 字段 |
| `web_search` | memories.content (text) | 无需额外表 | 保持 memories 即可 (搜索结果本身就是非结构化的) |

### ❌ 完全不存储的工具

| 工具 | 理由 |
|------|------|
| `read_file` | 纯读操作，不产生新知识 |
| `list_files` / `list_directory` | 目录结构无需持久化 |
| `grep_file` / `ast_grep` | 代码搜索结果是临时的 |
| `delete_file` | 删除操作无需存储结果 |

---

## 推荐改进路径

### Phase 1: 让 AI 能查询结构化数据（已部分完成）

`query_target_data` 工具已经存在，可以查询 target_assets、endpoints、fingerprints 等。
但目前缺少：

- [ ] 查询 `findings` 的工具（或扩展 query_target_data 增加 findings section）
- [ ] 查询 `targets` 列表的工具（AI 目前不能自行列出所有目标）
- [ ] 查询 `directory_entries` 的工具

### Phase 2: 工具输出自动解析（核心改进）

给关键的安全工具添加"输出解析钩子"，在工具执行成功后自动提取结构化数据：

```
run_command("nmap -sV target.com")
  ↓ 输出解析钩子
  ↓ 检测到是 nmap 输出
  ↓ 解析端口/服务/版本
  ↓ 自动写入 target_assets 表
  ↓ 同时仍存入 memories (作为补充)
```

优先级：
1. **nmap** → `target_assets` (端口扫描是最基础的)
2. **nuclei** → `findings` (漏洞发现是核心价值)
3. **ffuf/gobuster/dirsearch** → `directory_entries`
4. **httpx** → 更新 `targets` 字段 (http_title, webserver, cdn_waf)

### Phase 3: 记忆系统精简

当 Phase 2 完成后，gatekeeper 可以调整：
- 已经写入结构化表的数据，不再重复存入 memories
- memories 只保留真正非结构化的内容（观察、经验、策略判断）
