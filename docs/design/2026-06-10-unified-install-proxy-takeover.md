# 统一安装代理接管（平台 Proxy = 唯一真相源）

> **Status**: Implemented（2026-06-10 用户「按计划开工」+「那就搞 7」授权全部实现，含 §5.6 loader/env.rs 进程级对称化。验证证据见 `feature_list.json` `unified-install-proxy-takeover-2026-06-10`）
> **Author**: BaJie MCP-agent-3
> **来源**: 2026-06-10 工具安装连环失败排查——theHarvester / searchsploit / responder / wpscan 均装不上，根因落到「子进程继承了系统里的野代理（git 全局 config `http.proxy=socks5://127.0.0.1:6153`、已死的 Surge 6152/6153、进程环境变量 HTTP_PROXY）」
> **关联**: `frontend/components/ToolManager/hooks/useToolInstall.ts`（已修的 github release 404 → git clone 兜底）、`.cursor/skills/tool-installation`、`.cursor/skills/systematic-debugging`

---

## 0. 决策（TL;DR）

| # | 现状缺口 | 设计 |
|---|---|---|
| D1 | 每条安装路径只在「设了代理」时加环境变量；「没设」时什么都不做 → 子进程继承系统野代理 | 新增统一 helper，**对称处理**：设了→注入；没设→**主动清除**继承的代理变量 |
| D2 | git 的 `http.proxy` 全局 config 优先级**高于** `HTTP_PROXY` 环境变量；现有代码只设环境变量，盖不住 `socks5://6153` | 直接 git 调用注入 `-c http.proxy=<v|空>`（全版本）；间接 git（pip/bundler 内部 spawn）注入 `GIT_CONFIG_COUNT/KEY/VALUE`（git≥2.31） |
| D3 | `pip install git+https://…` 的 `--proxy` 只作用于 pip 自身 HTTP，**不传播**给 pip 内部 spawn 的 `git clone` | pip 路径同时注入 `GIT_CONFIG_*` 环境变量，覆盖 pip-nested git |
| D4 | `github.rs::github_client` 当代理为 None 时仍读进程环境代理（reqwest 默认行为） | None 时 `.no_proxy()` 强制直连 |
| D5 | 各路径代理变量集合不一致（有的 6 个、有的 2 个、有的只 `--proxy`） | 统一为同一组语义（大小写 + ALL_PROXY + NO_PROXY + git config） |

**一句话**：把「平台 Proxy 设置」做成所有安装子进程（git / pip / gem / brew / conda / reqwest）的**唯一真相源**——设了全走它，没设全强制直连，外部野代理一律不得劫持。

---

## 1. 背景：三类实测失败

均为 2026-06-10 本机（macOS）实跑 + 日志坐实，详见 systematic-debugging 排查记录。

### 1.1 theHarvester（`pip install git+https://…`）
- 配置改为 `method:pip / source:git+https://github.com/laramies/theHarvester.git`（修 PyPI 0.0.1 空壳包）。
- 失败：`pip` 内部 `git clone --filter=blob:none … github.com:443` → `LibreSSL SSL_connect: SSL_ERROR_SYSCALL`。
- 根因：pip 拿到了 `--proxy`，但它 spawn 的 `git` 子进程读到全局 `http.proxy=socks5://127.0.0.1:6153`（已死代理）→ 握手失败。`--proxy` 不传播给 nested git。

### 1.2 searchsploit / responder（git clone 兜底）
- 仓库无 GitHub Release（滚动更新型），release 404 后落到 `git clone` 兜底（前端已修兜底逻辑）。
- 失败：同一个全局 `http.proxy=socks5://6153` 死代理 → clone 失败。

### 1.3 wpscan（gem）
- `Gem::FilePermissionError`（系统只读 Ruby gem 目录）+ `SSL_connect SYSCALL rubygems.org`（走死代理 6152）。
- 权限部分已由 `find_homebrew_gem()` 规避系统 Ruby（见 §11）；剩余 SSL 部分正是本设计要解决的代理问题。

### 1.4 决定性对照实验（已验证）
| 测试 | 结果 |
|---|---|
| `curl https://github.com` 直连 | HTTP 200（curl 不读 git config，直连通） |
| `git ls-remote https://github.com/…` | SSL_ERROR_SYSCALL（git 读到死代理） |
| `git config --global http.proxy` | `socks5://127.0.0.1:6153` |
| `git -c http.proxy= ls-remote …` | **成功**（命令行 `-c` 盖掉全局 config，强制直连） |

→ `git -c http.proxy=`（空值）强制直连这一机制已被实验证实，是本设计的基石。

---

## 2. 现状全景：所有代理处理路径（代码地图）

> 以下逐条核对过源码。「git config 覆盖?」一列回答「能否盖住全局 `http.proxy`」。

| 层 | 位置 | 代理设了时 | 代理没设时 | git config 覆盖? |
|---|---|---|---|---|
| 进程启动 | `golish-settings/src/loader/env.rs::apply_proxy_env` L115 | `set_var` HTTP_PROXY/HTTPS_PROXY/ALL_PROXY(+NO_PROXY) 到**整个进程** | **什么都不做**（从不 unset） | 否 |
| 工具安装分发 | `golish-pentest/src/handlers/dispatch.rs::apply_proxy` L9 | 设 6 变量（大小写+ALL）到 cmd | **什么都不做** | 否 |
| git clone（dispatch github 分支） | `dispatch.rs` L474-482 | `apply_proxy`（仅环境变量） | 无 | **否 → 全局 http.proxy 胜出** |
| gem（dispatch + homebrew） | `dispatch.rs` L499 / `homebrew.rs` L70 | `apply_proxy` 环境变量 | 无 | n/a |
| brew / cask / homebrew bootstrap | `homebrew.rs` L7/44/103 | `apply_proxy` 环境变量 | 无 | n/a |
| git clone 命令 | `golish-pentest-app/.../install/mod.rs::pentest_git_clone_tool` L88-100 | **仅** HTTP_PROXY/HTTPS_PROXY（2 个） | 无 | **否 → 全局 http.proxy 胜出** |
| bundler / bundle | `install/mod.rs` L135-160 | **仅** HTTP_PROXY/HTTPS_PROXY | 无 | n/a |
| pip --target | `install/runtime.rs::pentest_pip_install_tool` L20-26 | pip `--proxy` 参数 | 无 | **pip-nested git 不覆盖** |
| conda | `runtime.rs::pentest_conda_install_tool` L49-55 | HTTP_PROXY/HTTPS_PROXY | 无 | n/a |
| conda-run pip | `runtime.rs::pentest_pip_install` L90-99 | 4 变量（大小写） | 无 | **pip-nested git 不覆盖** |
| pip -r requirements / dep_file | `runtime.rs` L388-400 / L487-499 | pip `--proxy` | 无 | **pip-nested git 不覆盖** |
| GitHub API / 下载 | `golish-pentest/src/github.rs::github_client` L8 | reqwest 显式 `.proxy()`（盖过环境变量） | **None 时仍读环境代理**（reqwest 默认） | n/a |

**观察**：
1. 全表唯一「没设代理时也做事」的只有 github_client（显式 proxy 时盖环境，但 None 时不 `.no_proxy()`）。其余全部**只在设了代理时动作**。
2. 全表**没有任何一处**盖得住 git 全局 config 的 `http.proxy`。
3. 变量集合不统一：6 个 / 4 个 / 2 个 / 仅 `--proxy`。

---

## 3. 根因（三类）

**R1 · 不对称**：每层只在代理 SET 时注入；NONE 时无人清除继承的环境代理、无人盖掉 git 全局 config → 野代理劫持子进程。这是「没设代理却还走代理」的直接原因。

**R2 · git config 优先级**：git 选代理的优先级是 `-c http.proxy`（命令行）> `http.proxy`（config 文件）> `HTTP_PROXY`/`http_proxy`/`ALL_PROXY`（环境变量）。现有代码只设环境变量，**永远盖不住**已写进 `~/.gitconfig` 的 `http.proxy=socks5://6153`。所有直接 git clone + pip-nested git clone 因此而死。

**R3 · 间接 git 不可达**：`pip install git+https://…` 时，pip 拿到的 `--proxy` 只控制 pip 自己的 HTTP；它 spawn 的 `git clone` 是独立子进程，只认 git config / git 能识别的环境变量。要管住它，必须注入 git 专用的环境变量（`GIT_CONFIG_*`）。

---

## 4. 用户需求（原话归纳）

> 「能不能让这个系统 无论是 git 还是怎么样 走的都是这个平台的代理。因为这个平台会有一个网络的代理设置，我这边设置了就走，不设置就不走代理。」

即 **平台 Proxy 设置 = 唯一真相源**：
- 设了 → 所有安装子进程（git/pip/gem/brew/conda/reqwest）都走它。
- 没设 → 全部**强制直连**，不被外部野代理（git 全局 config、系统 HTTP_PROXY、死掉的 Surge socks）劫持。

---

## 5. 设计：统一代理接管 helper

### 5.1 代理判定来源（single source）

代理值来源保持现状：`Settings → Network → proxy_url`（`golish-settings` 的 `network.proxy_url`，类型 `Option<String>`；另有 `network.no_proxy`）。前端 `useToolInstall.ts::getProxy()` 已从该字段读取并作为命令参数透传到后端。

判定函数统一归一化：
```rust
/// None / Some("") / Some(全空白) 一律视为「未配置 → 强制直连」。
pub fn normalize_proxy(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}
```

> 设计取舍：是否改为「后端直接读 settings」而非「前端透传参数」？**本设计保持透传**（改动小、与现有命令签名兼容）。后端读 settings 作为可选增强列入 §13 挂账。

### 5.2 helper API（新模块 `golish-pentest/src/handlers/proxy.rs`，`pub`）

放在 `golish-pentest`：它是底层 crate，`dispatch.rs`/`homebrew.rs` 在本 crate 内，`golish-pentest-app` 依赖它（已用 `golish_pentest::handlers::find_conda_bin` 等）。对外经 `golish_pentest::handlers::proxy::*` 暴露。

```rust
use std::process::Command;

/// 通用联网子进程（curl / brew / gem / conda / pip 自身 HTTP）。
/// SET  → 设 HTTP_PROXY/HTTPS_PROXY/http_proxy/https_proxy/ALL_PROXY/all_proxy（+ NO_PROXY/no_proxy 若有）。
/// NONE → env_remove 上述全部 → 子进程不继承野代理。
pub fn apply_http_proxy_env(cmd: &mut Command, proxy: Option<&str>, no_proxy: Option<&str>);

/// 会（直接或间接）跑 git 的子进程（pip git+ / bundler）。
/// SET  → 注入 GIT_CONFIG_COUNT=2 + http.proxy/https.proxy = url。
/// NONE → 注入同样的 key，但 VALUE 为空字符串 → 强制直连，盖掉全局 http.proxy。
pub fn apply_git_proxy_env(cmd: &mut Command, proxy: Option<&str>);

/// 我们自己控制 argv 的直接 git 调用。
/// 返回 ["-c","http.proxy=<v|空>","-c","https.proxy=<v|空>"]，prepend 到 "clone" 之前。
/// 用命令行 -c（全 git 版本支持、最高优先级），不依赖 GIT_CONFIG_*。
pub fn git_proxy_config_args(proxy: Option<&str>) -> Vec<String>;
```

### 5.3 三种注入机制（为何要三种）

| 机制 | 用于 | 原理 | 版本要求 |
|---|---|---|---|
| `git -c http.proxy=<v|空>` argv | 我们直接调 `git` 的地方 | 命令行 `-c` 最高优先级，空值=直连 | 全版本 |
| `GIT_CONFIG_COUNT/KEY/VALUE` 环境变量 | pip / bundler **内部** spawn 的 git | 等价于 `-c`，但通过环境传给 nested git | git ≥ 2.31（2021） |
| `HTTP_PROXY` 等环境变量（设/清） | curl / brew / gem / conda / pip 自身 HTTP / reqwest | 通用 HTTP 客户端约定 | 全版本 |

> 为什么直接 git 用 argv 而非 `GIT_CONFIG_*`？argv `-c` 全版本可用、零环境污染、可读性高。`GIT_CONFIG_*` 仅在我们**控制不到 nested git argv**（pip/bundler）时才不得不用。

### 5.4 各调用点改造

| 文件 | 调用点 | 改造 |
|---|---|---|
| `golish-pentest/handlers/dispatch.rs` | `apply_proxy`（保留并增强为 §5.2 `apply_http_proxy_env`，对称清除） | 全局替换语义 |
| 同上 | github 分支 `git clone` | argv prepend `git_proxy_config_args` + `apply_http_proxy_env` |
| 同上 | gem 分支 | `apply_http_proxy_env` |
| `golish-pentest/handlers/homebrew.rs` | install_homebrew(curl bash)/brew/cask/gem | `apply_http_proxy_env`（None 时清野变量，避免 brew 走死代理） |
| `golish-pentest-app/.../install/mod.rs` | `pentest_git_clone_tool` 的 `git clone` | argv prepend `git_proxy_config_args` + `apply_http_proxy_env` |
| 同上 | bundler / bundle | `apply_git_proxy_env` + `apply_http_proxy_env`（bundler 内部调 git） |
| `golish-pentest-app/.../install/runtime.rs` | `pentest_pip_install_tool` / `pentest_pip_install` / `pentest_install_requirements` / `pentest_install_dep_file` | SET 时保留 pip `--proxy` + **新增** `apply_git_proxy_env`（管 pip-nested git）+ `apply_http_proxy_env` |
| 同上 | `pentest_conda_install_tool` | `apply_http_proxy_env` |
| `golish-pentest/github.rs` | `github_client` | None 时 `.no_proxy()`；Some 时维持 `.proxy()`（已正确） |

### 5.5 github_client 的 None 分支

```rust
fn github_client(proxy_url: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().user_agent(USER_AGENT);
    match normalize_proxy(proxy_url) {
        Some(p) => builder = builder.proxy(reqwest::Proxy::all(&p)?),
        None => builder = builder.no_proxy(), // 强制直连，忽略进程继承的环境代理
    }
    builder.build().map_err(Into::into)
}
```
reqwest 默认会读 `HTTP_PROXY/HTTPS_PROXY/ALL_PROXY`；`.no_proxy()` 关掉这个默认行为，落实「没设就直连」。

### 5.6 进程级 `apply_proxy_env`（loader/env.rs）的处理

现状：启动时若 proxy 设了就 `set_var` 到整个进程；没设不动。这与「单一真相源」冲突点在于：① 不对称（不清）；② 运行时用户在 UI 改了代理，进程级环境不会更新。

**决策 D6（已落地）**：**不依赖进程级环境**作为子进程代理来源——以 §5.2 的**每子进程 helper 为权威**（它在每次 spawn 前用当前设置即时注入/清除）。`loader/env.rs::apply_proxy_env` 保留（服务于进程内、不显式传 proxy 的 reqwest 客户端：LLM/web fetch/telemetry），且**已改对称**：proxy 配置 → 设 6 个大小写变量；proxy 为 None/空 → `remove_var` 掉全部 6 个 + NO_PROXY，避免进程内 HTTP 客户端被启动环境里的野代理带偏。

> **副作用（用户已知悉并接受）**：以前可以「在终端里 `HTTP_PROXY=xxx` 启动 app → AI/LLM 调用走该代理，但不在平台里配」。对称化后这种「外部 env 旁路」会**静默失效**（平台留空 = AI 也直连）。这正是「平台 Proxy = 唯一真相源」的预期语义：要让 AI 走代理，就在 `Settings → Network → Proxy` 里配。

---

## 6. git 版本要求与备选

- `GIT_CONFIG_COUNT/KEY/VALUE`：git ≥ 2.31.0（2021-03）。macOS 自带 / Homebrew git 均满足。
- 备选（git ≥ 2.32）：`GIT_CONFIG_GLOBAL=<Golish 托管的临时 gitconfig>` 完全**替换** `~/.gitconfig`。
  - 优点：彻底屏蔽用户全局 config 里的野代理。
  - 缺点：同时屏蔽 `credential.helper` 等有用配置，可能破坏私有仓库 clone。
  - **结论**：选 `GIT_CONFIG_COUNT`（叠加式、只覆盖 proxy、保留其余 config）；`GIT_CONFIG_GLOBAL` 仅作 fallback 备注，不默认启用。
- 极旧 git（< 2.31）：直接 git 调用走 argv `-c`（无版本要求）仍生效；只有 pip-nested git 的覆盖会失效——此场景在 macOS/现代 Linux 不会触发，列为已知限制。

---

## 7. no_proxy 处理

`settings.network.no_proxy` 已存在。SET 代理时一并注入 `NO_PROXY/no_proxy`；NONE 时连同代理变量一起清除（直连场景 no_proxy 无意义）。reqwest 侧 Some 分支可叠加 no_proxy 列表（可选增强）。

---

## 8. 影响面

- **跨 2 crate、5 文件**（纯后端）：
  - `golish-pentest/src/handlers/proxy.rs`（**新增** helper 模块）
  - `golish-pentest/src/handlers/mod.rs`（导出 proxy 模块）
  - `golish-pentest/src/handlers/dispatch.rs`（apply_proxy 增强 + github/gem 分支）
  - `golish-pentest/src/handlers/homebrew.rs`（brew/cask/gem/bootstrap）
  - `golish-pentest/src/github.rs`（github_client None 分支）
  - `golish-pentest-app/src/pentest/packages/install/mod.rs`（git clone + bundler）
  - `golish-pentest-app/src/pentest/packages/install/runtime.rs`（pip/conda 四处）
  - （可选）`golish-settings/src/loader/env.rs`（apply_proxy_env 对称化）
- **不动**：前端（`useToolInstall.ts` 已修兜底，签名不变）、命令注册、ts-rs 类型、DB schema、migration。
- 编译 + `cargo nextest` 验证（Rust 编译数分钟）。

---

## 9. 测试策略（TDD）

**单元测试（`proxy.rs` 纯函数 / 用 `Command` 断言 env）**：
- `normalize_proxy`：None / "" / "  " → None；"http://x" → Some。
- `git_proxy_config_args`：Some → `["-c","http.proxy=http://x","-c","https.proxy=http://x"]`；None → `["-c","http.proxy=","-c","https.proxy="]`。
- `apply_http_proxy_env`：Some → cmd 含 6 个代理 env；None → 这些 env 被标记移除（`Command::get_envs` 中对应项为 `(key, None)`）。
- `apply_git_proxy_env`：Some → `GIT_CONFIG_COUNT=2` + KEY/VALUE 正确；None → VALUE 为空串。

**集成 / 手动复现（real）**：见 §10。

---

## 10. 验证（手动复现脚本）

复刻 1.4 的环境，证明「没设代理时强制直连成功」：
```bash
# 制造野代理（模拟死 Surge）
git config --global http.proxy socks5://127.0.0.1:6153
# 平台 Proxy 留空 → 期望：以下三个工具安装走直连成功
#   theHarvester / searchsploit / responder
# 平台 Proxy 设为可用代理 → 期望：安装流量经该代理（用只通代理可达的 host 或代理访问日志佐证）
git config --global --unset http.proxy   # 复现完清理
```
- 后端验证命令：`cd backend && cargo nextest run -p golish-pentest`、`just check-rust`、`just precommit`。
- 证据落 `agent-progress.md`（命令 + 退出码 + 关键输出）。

---

## 11. 范围外 / 挂账

- **wpscan `Gem::FilePermissionError`**：与代理无关，是「系统只读 Ruby gem 目录」。现有 `install_gem_package` 已用 `find_homebrew_gem()` 优先 rbenv/Homebrew 的可写 gem（`tool_manager/paths.rs` L148），权限部分**已规避**；wpscan 剩余失败的 SSL 部分由本设计解决。若仍报权限错，单列后端任务（让 gem 路径强制走可写 Ruby）。
- **github release 404 → git clone 兜底**：已在 `useToolInstall.ts` 修复（前一批改动），本设计不重复。
- **后端直接读 settings（替代前端透传 proxy 参数）**：可选架构增强，单列。
- **impacket**：纯 PyPI 包，清野代理后预期可装；不需专门改造。

---

## 12. 风险

| 风险 | 缓解 |
|---|---|
| 用户依赖 git 全局 `http.proxy` 走私有网络，被我们强制直连后 clone 私有仓库失败 | 我们只覆盖**安装子进程**的 git 代理，不改用户全局 config；且仅当平台 Proxy 留空才注入空值。文档提示「需要代理就在平台设」 |
| git < 2.31 的 pip-nested git 覆盖失效 | macOS/现代 Linux 不触发；列为已知限制 + argv `-c` 对直接 git 仍生效 |
| `apply_http_proxy_env` 清除 env 误伤其它继承变量 | 只 `env_remove` 固定的代理变量白名单（HTTP_PROXY/HTTPS_PROXY/大小写/ALL_PROXY/NO_PROXY），不动其它 |
| 代理凭据经 env 传子进程的泄漏面 | 与现状一致（现状已用 env 传）；不新增日志打印 proxy 值（保持 `tracing` 只记 `is_some()`） |
| 进程级 env 与每子进程 helper 双轨不一致 | D6 明确「每子进程 helper 为权威」；进程级仅服务进程内 reqwest，建议对称化 |

---

## 13. 安全考量（pentest 平台特有）

- 本平台是渗透测试工具，代理设置直接决定「扫描/安装流量从哪出口」。统一接管后，**唯一出口 = 平台 Proxy**，消除「以为没走代理实际走了野代理 / 以为走了代理实际直连」的不确定性——对溯源与合规有正面价值。
- 不在日志中明文打印 proxy_url（可能含凭据 `http://user:pass@host`）；沿用现有 `proxy.is_some()` 记法。
- 不引入任何「静默降级走系统代理」的兜底（与 winget `--proxy` 不静默降级的既有原则一致）。

---

## 14. 决策记录（实现前待确认 → 已定）

1. `apply_proxy_env`（loader/env.rs）同批对称化（D6）？→ **是**（用户 2026-06-10「那就搞 7」拍板；副作用见 §5.6）。
2. 接受 git ≥ 2.31 作为 pip-nested git 覆盖的硬要求？→ **是**（本机 git 2.50.1，目标平台均满足）。
3. helper 模块落点 `golish-pentest/src/handlers/proxy.rs`？→ **是**（最低公共依赖 crate，golish-pentest-app 经 `golish_pentest::handlers::proxy::*` 复用）。
