#!/usr/bin/env bash
# init.sh — Golish Platform 一键环境验证脚本
#
# 用途：
#   每轮新会话开始前跑一次，确保依赖、构建、测试基线都没问题。
#   如果失败，必须先修基础环境，不要在坏的地基上叠新功能。
#
# 用法：
#   bash init.sh                # 默认：装依赖 + 验证（pnpm + cargo）
#   bash init.sh --skip-install # 跳过依赖安装，只跑验证
#   bash init.sh --quick        # 只跑最快的检查（typecheck + cargo check）
#   RUN_START_COMMAND=1 bash init.sh   # 验证完后直接启动 just dev

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ─────────────────────────────────────────────────────────────
# 配置
# ─────────────────────────────────────────────────────────────
INSTALL_CMD="just install"             # 装依赖（pnpm install）
VERIFY_CMD="just check"                # 全套静态检查 + 单测
QUICK_VERIFY_CMD_FE="just check-fe"    # 仅前端 biome + typecheck
QUICK_VERIFY_CMD_RUST="just check-rust" # 仅 cargo check + fmt
START_CMD="just dev"                   # 启动开发服务器

# ─────────────────────────────────────────────────────────────
# 参数解析
# ─────────────────────────────────────────────────────────────
SKIP_INSTALL=0
QUICK_MODE=0

for arg in "$@"; do
    case "$arg" in
        --skip-install) SKIP_INSTALL=1 ;;
        --quick)        QUICK_MODE=1 ;;
        -h|--help)
            cat <<EOF
Usage: bash init.sh [options]
Options:
  --skip-install    跳过依赖安装步骤
  --quick           只跑最快的检查（typecheck + cargo check），不跑测试
  -h, --help        显示此帮助
Env:
  RUN_START_COMMAND=1  验证通过后直接启动 just dev
EOF
            exit 0
            ;;
        *) echo "Unknown argument: $arg" >&2; exit 64 ;;
    esac
done

# ─────────────────────────────────────────────────────────────
# 输出辅助
# ─────────────────────────────────────────────────────────────
B="\033[1;36m"; G="\033[1;32m"; R="\033[1;31m"; Y="\033[1;33m"; N="\033[0m"

say()   { printf "${B}━━━ %s ━━━${N}\n" "$1"; }
ok()    { printf "${G}✓ %s${N}\n" "$1"; }
warn()  { printf "${Y}⚠ %s${N}\n" "$1"; }
fail()  { printf "${R}✗ %s${N}\n" "$1"; }
die()   { fail "$1"; exit 1; }

# ─────────────────────────────────────────────────────────────
# Step 0: 打印环境上下文
# ─────────────────────────────────────────────────────────────
say "Step 0: 环境上下文"
echo "  CWD       : $(pwd)"
echo "  Node      : $(command -v node >/dev/null && node -v || echo 'not found')"
echo "  pnpm      : $(command -v pnpm >/dev/null && pnpm -v || echo 'not found')"
echo "  cargo     : $(command -v cargo >/dev/null && cargo -V || echo 'not found')"
echo "  just      : $(command -v just >/dev/null && just --version || echo 'not found')"
echo "  rustc     : $(command -v rustc >/dev/null && rustc -V || echo 'not found')"
echo "  uname     : $(uname -sr)"

# ─────────────────────────────────────────────────────────────
# Step 1: 必需工具检查
# ─────────────────────────────────────────────────────────────
say "Step 1: 必需工具检查"
MISSING=0
for tool in node pnpm cargo rustc just; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        fail "$tool 未安装"
        MISSING=1
    else
        ok "$tool 可用"
    fi
done
[ "$MISSING" -eq 0 ] || die "请先安装缺失的工具（推荐：brew install pnpm just rustup-init，rustup 装 stable toolchain）"

# 推荐项检查
if ! command -v cargo-nextest >/dev/null 2>&1; then
    warn "cargo-nextest 未安装，just test-rust 会报错。建议跑：cargo install cargo-nextest --locked"
fi

# ─────────────────────────────────────────────────────────────
# Step 2: 装依赖
# ─────────────────────────────────────────────────────────────
if [ "$SKIP_INSTALL" -eq 0 ]; then
    say "Step 2: 装依赖 ($INSTALL_CMD)"
    $INSTALL_CMD || die "依赖安装失败"
    ok "依赖安装完成"
else
    warn "Step 2: 已跳过（--skip-install）"
fi

# ─────────────────────────────────────────────────────────────
# Step 3: 验证
# ─────────────────────────────────────────────────────────────
if [ "$QUICK_MODE" -eq 1 ]; then
    say "Step 3: 快速验证（前端 typecheck + Rust cargo check）"
    $QUICK_VERIFY_CMD_FE   || die "前端快速检查失败"
    $QUICK_VERIFY_CMD_RUST || die "Rust 快速检查失败"
    ok "快速验证通过"
    warn "注意：--quick 模式不跑单测，commit 前必须再跑 'just precommit'"
else
    say "Step 3: 完整验证 ($VERIFY_CMD)"
    $VERIFY_CMD || die "验证失败 — 请先修复基础环境再继续开发"
    ok "完整验证通过"
fi

# ─────────────────────────────────────────────────────────────
# Step 4: 提示下一步
# ─────────────────────────────────────────────────────────────
say "Step 4: 准备就绪"
echo ""
echo "  开发：   $START_CMD"
echo "  前端：   just dev-fe"
echo "  提交前： just precommit"
echo "  E2E：    just test-e2e"
echo "  全 CI：  just ci"
echo ""
echo "  阅读 AGENTS.md §1 开工流程，再开始动代码。"
echo ""

if [ "${RUN_START_COMMAND:-0}" = "1" ]; then
    say "RUN_START_COMMAND=1 — 启动开发服务器"
    exec $START_CMD
fi
