# Plan: 统一安装代理接管（平台 Proxy = 唯一真相源）

> 设计: `docs/design/2026-06-10-unified-install-proxy-takeover.md`（Draft，实现前先过 §14 三个待确认决策）。
> 全程 TDD：每个行为变化先红后绿。验证命令均在 `backend/` 下执行。
> 纯后端、跨 2 crate（`golish-pentest` / `golish-pentest-app`，可选 `golish-settings`）、不动前端 / 命令注册 / ts-rs / DB。

## Task 0 · 前置确认（不写代码）
1. 与用户确认设计 §14 三点：loader/env.rs 是否对称化、git≥2.31 硬要求、helper 落点。
2. 确认本机 `git --version` ≥ 2.31（`git --version`）。
3. 把 feature_list.json 追加一条 `unified-install-proxy-takeover`，状态 `not_started` → 选中设 `in_progress`。

## Task 1 · 新增 proxy helper 纯模块
1. 新建 `golish-pentest/src/handlers/proxy.rs`：`normalize_proxy` / `apply_http_proxy_env` / `apply_git_proxy_env` / `git_proxy_config_args`（签名见设计 §5.2）。
2. `golish-pentest/src/handlers/mod.rs` 加 `pub mod proxy;`。
3. RED（先写单测，见设计 §9）：
   - `normalize_proxy`：None/""/空白 → None；正常值 → Some。
   - `git_proxy_config_args`：Some → `-c http.proxy=<v> -c https.proxy=<v>`；None → 值为空串。
   - `apply_http_proxy_env`：Some 设 6 变量；None 时 `Command::get_envs()` 对应项为 `(key, None)`（移除）。
   - `apply_git_proxy_env`：Some → `GIT_CONFIG_COUNT=2` + KEY_0/VALUE_0=http.proxy、KEY_1/VALUE_1=https.proxy；None → VALUE 为空。
4. GREEN: 实现四个函数。
5. 验证: `cargo nextest run -p golish-pentest proxy`

## Task 2 · dispatch.rs 接线
1. `apply_proxy` 调用点全部迁移到 `proxy::apply_http_proxy_env`（带 None 清除语义）。
2. github 分支：`git clone` argv 用 `proxy::git_proxy_config_args(proxy)` prepend 到 `clone` 前 + `apply_http_proxy_env`。
3. gem 分支：`apply_http_proxy_env`。
4. 验证: `cargo nextest run -p golish-pentest`（含既有 winget/elevation 单测不回归）。

## Task 3 · homebrew.rs 接线
1. install_homebrew(curl bash) / install_brew_package / install_brew_cask / install_gem_package 全部改用 `proxy::apply_http_proxy_env`。
2. 验证: `cargo check -p golish-pentest`。

## Task 4 · github.rs None 直连
1. `github_client`：`normalize_proxy` 后 Some → `.proxy()`、None → `.no_proxy()`。
2. RED（可选）：构造 `github_client(None)` 并断言其不使用环境代理（或保留为手动验证，见 Task 7）。
3. 验证: `cargo check -p golish-pentest`。

## Task 5 · install/mod.rs（git clone 命令 + bundler）
1. `pentest_git_clone_tool`：`git clone` argv prepend `proxy::git_proxy_config_args` + `apply_http_proxy_env`（替换现有 2 变量写法）。
2. bundler `gem install bundler` / `bundle install`：`apply_git_proxy_env` + `apply_http_proxy_env`。
3. 验证: `cargo check -p golish-pentest-app`。

## Task 6 · install/runtime.rs（pip / conda）
1. `pentest_pip_install_tool` / `pentest_pip_install` / `pentest_install_requirements` / `pentest_install_dep_file`：
   - SET 时保留 pip `--proxy <url>`；
   - 一律调用 `proxy::apply_git_proxy_env`（覆盖 pip-nested `git+` clone）+ `proxy::apply_http_proxy_env`。
2. `pentest_conda_install_tool`：`apply_http_proxy_env`。
3. 验证: `cargo check -p golish-pentest-app` + `cargo nextest run -p golish-pentest-app`。

## Task 7 · loader/env.rs 对称化（用户「那就搞 7」已确认 → 已完成）
1. `apply_proxy_env`：proxy 配置 → 设 6 大小写变量 + NO_PROXY；None/空 → `remove_var` 全部（含 NO_PROXY/no_proxy）。
2. 单测（先红后绿）：设了→6 变量全设；没设→全清；blank→视为未设；no_proxy 未设→清。
3. 验证: `cargo nextest run -p golish-settings`（60 passed）+ clippy -D + fmt。✓

## Task 8 · 手动复现验证（real evidence）
1. `git config --global http.proxy socks5://127.0.0.1:6153`（造死代理）。
2. 平台 Proxy 留空 → 重启 Golish → 安装 theHarvester / searchsploit / responder：期望直连成功。
3. 平台 Proxy 设为可用代理 → 期望流量经该代理（代理访问日志 / 仅代理可达 host 佐证）。
4. `git config --global --unset http.proxy` 清理。
5. 证据（命令 + 退出码 + 关键输出）落 `agent-progress.md`。

## Task 9 · 收口
1. `cargo fmt` + `cargo clippy -p golish-pentest -p golish-pentest-app --lib -- -D warnings`。
2. `cd backend && cargo nextest run -p golish-pentest -p golish-pentest-app`（+ 若动了 settings 则 `-p golish-settings`）。
3. `just precommit` 全绿。
4. ReadLints 所有改动文件；agent-progress.md + feature_list.json `evidence` 填写；本设计 Status 改 Approved/Done。
