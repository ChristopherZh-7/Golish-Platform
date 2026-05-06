---
name: tool-installation
description: "安装、配置或验证渗透测试工具时使用。处理包管理器、Python 环境、编译工具和 PATH 配置。"
---

# 工具安装与环境配置

按照项目 ToolManager 规范安装、配置和验证安全工具。

## 安装前检查

安装任何工具之前：

1. **检查现有安装**：`which <tool>`、`<tool> --version` 或在常见路径中 `find`
2. **检查 ToolManager 注册表**：后端 `tool_manager` 模块跟踪已安装工具——先查询避免冲突
3. **验证系统兼容性**：确定当前平台（macOS / Linux）的最佳安装方式

## 安装方式（按优先级）

| 优先级 | 方式 | 使用场景 |
|--------|------|---------|
| 1 | ToolManager API | 始终优先——调用 `pentest_install_tool` 或运行时检查路径 |
| 2 | Homebrew (`brew install`) | macOS，ToolManager 未覆盖的工具 |
| 3 | Go install | Go 工具（nuclei、httpx、katana、subfinder 等） |
| 4 | pip / pipx | Python 工具——**必须**使用虚拟环境 |
| 5 | cargo install | Rust 工具（feroxbuster 等） |
| 6 | 直接下载二进制 | 无包管理器覆盖时 |

## 工作流

```
1. 检查是否已安装  →  已存在则跳过
2. 先解析依赖  →  先装依赖再装工具
3. 用最高优先级的可用方式安装
4. 验证  →  运行 --help 或 --version
5. 按需配置 PATH  →  更新 shell 配置文件
6. 报告结果  →  版本号 + 安装路径
```

## 约束

- **禁止以 root 安装**，除非绝对必要（如 nmap 原始套接字模式）
- **Python 工具必须用虚拟环境**——禁止污染全局 site-packages
- **妥善处理依赖冲突**——清晰报告，不强制覆盖
- **尊重 ToolManager 状态**——手动安装后，更新 ToolManager 的已知工具注册表
- **失败时给出清晰错误信息**——包含失败的具体命令及其 stderr

## 包管理器能力

- **系统级**：brew、apt、dnf、pacman
- **语言级**：pip、gem、go install、cargo、npm
- **Python 环境**：venv、pyenv、pip 依赖解析
- **编译类**：Go 构建、Rust 编译、C/C++ make
- **容器类**：Docker 镜像拉取与管理

## 验证清单

安装完成后确认：

- [ ] 工具二进制存在于 PATH 中
- [ ] `--version` 或 `--help` 运行无报错
- [ ] 必需的运行时依赖已就位（如 Python 库、Go 模块）
- [ ] ToolManager 注册表已更新（如适用）
