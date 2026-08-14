# golish-skills

> **一句话职责**：Agent Skills 的发现/解析/匹配（agentskills.io 规范）——从 `~/.golish/skills/` 和 `<project>/.golish/skills/` 加载 `SKILL.md` 给 AI 注入专项指令。

- **类型**：crate（Layer 3）
- **路径**：`backend/crates/golish-skills/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 skill 发现/解析/匹配、SKILL.md frontmatter、skill 注入 prompt 时
- skill 没被匹配/加载时

## 职责

实现 agentskills.io 规范：发现 skill（全局 + 项目级，本地覆盖全局）、解析 SKILL.md、按用户 prompt 匹配打分、加载正文注入 system prompt。提供 `DefaultSkillProvider` 实现 `golish_core::SkillProvider`。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `discover_skills(project)` / `list_skill_files` / `read_skill_file` | 发现 |
| `SkillMatcher` / `extract_keywords` | 匹配打分 |
| `parse_skill_md` / `load_skill_body` / `validate_skill_name` | 解析 |
| `SkillMetadata` / `SkillInfo` / `SkillFrontmatter` / `MatchedSkill` | 类型 |
| `DefaultSkillProvider` | 实现 `golish_core::SkillProvider` |

## 依赖

- **内部**：`golish-core`（实现其 `SkillProvider` trait）

## 被谁依赖 / 改动影响面

`golish`、`golish-app-core`、`golish-sub-agents`。

## 关键文件（无目录子模块）

`discovery.rs`、`matcher.rs`、`parser.rs`、`types.rs`。

## 注意事项 / 坑

- 本地 skill（`<project>/.golish/skills/`）覆盖同名全局 skill（`~/.golish/skills/`）。
- 这是 Golish **产品内**的 skill 系统（给被测 agent 用），别和本仓库开发用的 `.cursor/skills/` 混淆。
- 相关：`docs/agent-skills.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-skills
```

## CyberStrike methodology catalog（2026-08-12）

`methodology_catalog` 与普通 `DefaultSkillProvider` 分离：它只发现 exact-case `SKILL.md` regular files，拒绝 symlink，重算 document/content-root hash，并按受控 tag 查询返回 bounded excerpt。结果固定声明 instruction/scope/authorization/evidence authority 全为 false；CyberStrike 正文不会成为 system instruction，也不会被直接执行。
