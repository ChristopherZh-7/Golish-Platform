# Golish 文档索引

> Golish 的用户指南、子系统参考、开发文档、设计/计划/模块卡与风险登记。**本页是总导航**——根目录放「活文档」（指南/参考），点时点记录（设计/计划/分析）按日期归在子目录并各有自己的 INDEX。

## 🗂️ 子目录（各有索引）

| 目录 | 内容 | 入口 |
|---|---|---|
| `modules/` | **模块卡**：每个 crate / 目录子模块 / 前端子系统一张 AI 可读卡（职责/接口/依赖/坑/测试入口），185 张 | [模块索引](modules/INDEX.md) |
| `design/` | **设计决策**（按日期，不覆盖旧文件），60 篇 | [设计索引](design/INDEX.md) |
| `superpowers/plans/` | **实现计划**（按日期，分步执行），66 篇 | [计划索引](superpowers/plans/INDEX.md) |
| `analysis/` | **对标/研究**快照（point-in-time），3 篇 | [分析索引](analysis/README.md) |
| `risks/` | **风险登记**（依赖/平台/维护风险） | [风险索引](risks/README.md) |
| `img/` | 文档配图资源 | — |

---

## 🚀 上手

- [Getting started](getting-started.md) · [Configuration](configuration.md) · [Providers](providers.md) · [Workspaces](workspaces.md)

## 🧑‍💻 使用 Golish

- [Agent modes](agent-modes.md) · [Auto input mode](auto-input-mode.md) · [Agent skills](agent-skills.md)
- [Tool use](tool-use.md) · [Image handling](image-handling.md) · [Tab completion](tab-completion.md) · [Themes](theme-tokens.md)

## 🔌 集成

- [MCP (Model Context Protocol)](mcp.md) · [Web search (Tavily)](tavily-tools.md) · [Langfuse tracing](langfuse-tracing.md) · [AST-grep tools](ast-grep-tools.md)

## 🏗️ 开发

- [Development](development.md) · [Architecture](architecture.md) · [Browser-only frontend dev](browser-dev.md) · [Releasing](releasing.md) · [Windows support](windows-support.md)

## 🧩 子系统参考（internals）

- [System hooks](system-hooks.md) · [Planning system](planning-system.md) · [Concurrent sub-agents](concurrent-sub-agents.md)
- [System prompt guide](system-prompt-guide.md) · [Prompt contributions](prompt-contributions.md)
- [Database & tools](database-and-tools.md) · [Graph-flow integration](graph-flow-integration.md) · [Auth-probe contract](auth-probe-contract.md)

## 🗺️ 规划 / 概览

- [Phase 1: pentest platform](PHASE1_PENTEST_PLATFORM.md) · [Golish 思维导图（中文）](golish-mindmap-cn.md)

---

> 维护约定：根目录放**活文档**（kebab-case、随代码演进）；设计/计划/分析是**时点记录**（日期前缀、归子目录、作废只加 `> Superseded by …` 不删，见 AGENTS.md §2.4/I6）。改了某模块要同步更新它的 `modules/` 卡 + 索引。
