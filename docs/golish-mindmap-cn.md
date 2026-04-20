# Golish Platform — 架构脑图

> AI 驱动的安全测试终端 | Tauri 2 桌面应用 | Rust + React 19

---

## 系统架构总览

```mermaid
graph TB
    subgraph 桌面应用["桌面应用 (Tauri 2)"]
        FE["前端<br/>React 19 + Tailwind v4"]
        IPC["Tauri IPC 通道"]
        BE["后端<br/>Rust (28 Crates, 4 层架构)"]
        FE -->|invoke / listen| IPC
        IPC --> BE
    end

    subgraph 数据层["数据层"]
        DB[(PostgreSQL<br/>30+ 张表)]
        FS["本地文件系统<br/>~/.golish/"]
    end

    subgraph AI 引擎["AI 引擎"]
        LLM["LLM 提供商<br/>Anthropic · OpenAI · Gemini · 智谱"]
        MCP["MCP 外部工具服务器"]
    end

    subgraph 安全工具链["安全工具链 (托管二进制)"]
        RECON["侦察<br/>naabu · httpx · whatweb · katana"]
        SCAN["扫描<br/>nuclei · feroxbuster"]
        DAST["DAST<br/>OWASP ZAP"]
    end

    BE --> DB
    BE --> FS
    BE --> LLM
    BE --> MCP
    BE --> RECON
    BE --> SCAN
    BE --> DAST
```

## 项目脑图

```mermaid
mindmap
  root((Golish Platform))
    前端
      终端系统
        xterm.js + PTY
        分屏布局
        命令解析
      AI 对话
        流式消息
        工具调用展示
        人工审批
      安全测试面板
        目标管理
        扫描工具
        漏洞发现
        情报中心
      状态管理
        Zustand + Immer
        Tauri IPC
    后端 (4 层架构)
      应用层
        Tauri 命令
        CLI 入口
        安全工具编排
      领域层
        Agent 循环
        系统提示词
        上下文压缩
      基础设施层
        终端 PTY
        工具注册
        数据库 ORM
        LLM 适配 ×5
        MCP 客户端
      基石层
        核心类型
        运行时 Trait
    数据库
      资产: 目标 · 指纹 · API
      漏洞: 发现 · PoC · 情报
      扫描: 历史 · 队列 · 站点图
      日志: 审计 · 代理 · 终端
      配置: 方法论 · 凭据 · 规则
    安全工具链
      侦察: naabu → httpx → katana
      扫描: nuclei · feroxbuster
      DAST: OWASP ZAP
    DevOps
      构建: just · pnpm · Cargo
      测试: Vitest · Playwright · nextest
      质量: Biome · Clippy
```

## 后端分层架构

```mermaid
graph TB
    subgraph L4["第四层 — 应用层"]
        MAIN["golish 主 Crate"]
        TAURI_CMD["Tauri 命令"]
        CLI["CLI 模式"]
        TOOLS["安全工具模块<br/>流水线 · 扫描 · 解析 · ZAP"]
        MAIN --- TAURI_CMD
        MAIN --- CLI
        MAIN --- TOOLS
    end

    subgraph L3["第三层 — 领域层"]
        AI["golish-ai"]
        LOOP["Agent 循环"]
        PROMPT["系统提示词"]
        COMPACT["上下文压缩"]
        EXEC["工具执行器"]
        AI --- LOOP
        AI --- PROMPT
        AI --- COMPACT
        AI --- EXEC
    end

    subgraph L2["第二层 — 基础设施层 (18 Crates)"]
        PTY["golish-pty<br/>终端"]
        TOOL_SYS["golish-tools<br/>文件/AST"]
        DB_CRATE["golish-db<br/>数据库"]
        SESSION["golish-session<br/>会话"]
        CTX["golish-context<br/>Token 预算"]
        WEB["golish-web<br/>搜索/抓取"]
        MCP_C["golish-mcp<br/>MCP 客户端"]
        LLM_P["LLM 提供商 ×5"]
    end

    subgraph L1["第一层 — 基石层"]
        CORE["golish-core<br/>事件 · Trait · 类型"]
    end

    L4 --> L3
    L3 --> L2
    L2 --> L1
```

## 安全测试流水线

```mermaid
graph LR
    TARGET["目标输入<br/>IP / 域名"] --> NAABU["naabu<br/>端口扫描"]
    NAABU --> HTTPX["httpx<br/>HTTP 探测"]
    HTTPX --> WHATWEB["whatweb<br/>指纹识别"]
    HTTPX --> KATANA["katana<br/>爬虫"]
    WHATWEB --> DB_WRITE["写入数据库<br/>指纹 · 目标"]
    KATANA --> DB_WRITE
    DB_WRITE --> NUCLEI["nuclei<br/>漏洞扫描"]
    DB_WRITE --> FEROX["feroxbuster<br/>目录扫描"]
    DB_WRITE --> ZAP_SCAN["ZAP<br/>DAST 扫描"]
    NUCLEI --> FINDINGS["漏洞发现"]
    FEROX --> FINDINGS
    ZAP_SCAN --> FINDINGS
```

## 技术栈

| 领域 | 技术选型 | 说明 |
|------|---------|------|
| 桌面框架 | **Tauri 2** | Rust 后端 + WebView 前端 |
| 前端 | **React 19** + Vite + Tailwind v4 | shadcn/ui 组件库 |
| 后端 | **Rust** (28 Crates) | 4 层模块化架构 |
| 数据库 | **PostgreSQL** + sqlx | 30+ 张表，编译期 SQL 检查 |
| AI | **rig-core** | 5 个 LLM 适配器 (Anthropic/OpenAI/Gemini/智谱) |
| 终端 | **portable-pty** + xterm.js | 真实 PTY 会话 |
| 安全工具 | naabu / httpx / nuclei / ZAP 等 | 7 个外部二进制 |
| 测试 | Vitest + Playwright + nextest | 前端 + E2E + Rust |
