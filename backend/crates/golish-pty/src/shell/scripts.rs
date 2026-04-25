//! Embedded shell-integration scripts.
//!
//! These blobs are written to disk by [`super::ShellIntegration`] before
//! a PTY is spawned, and sourced by the shell to install OSC 133 hooks.

/// The zsh integration script that emits OSC 133 sequences.
/// Embedded in the binary to avoid file-path dependencies.
pub(super) const ZSH_INTEGRATION_SCRIPT: &str = r#"# Golish Shell Integration (auto-injected)
# Emits OSC 133 sequences for command tracking

# Debug: confirm script is being sourced
[[ -n "$QBIT_DEBUG" ]] && echo "[golish-integration] Loading integration script..."

# Guard against double-sourcing (use unique var to avoid conflict with old integration)
if [[ -n "$__QBIT_OSC133_LOADED" ]]; then
    [[ -n "$QBIT_DEBUG" ]] && echo "[golish-integration] Already loaded, skipping"
    return
fi
export __QBIT_OSC133_LOADED=1

[[ -n "$QBIT_DEBUG" ]] && echo "[golish-integration] Registering hooks..."

# ============ OSC Helpers ============

__golish_osc() {
    printf '\e]133;%s\a' "$1"
}

__golish_report_cwd() {
    printf '\e]7;file://%s%s\a' "${HOST:-$(hostname)}" "$PWD"
}

__golish_report_venv() {
    if [[ -n "$VIRTUAL_ENV" ]]; then
        local venv_name="${VIRTUAL_ENV##*/}"
        printf '\e]1337;VirtualEnv=%s\a' "$venv_name"
    else
        printf '\e]1337;VirtualEnv=\a'
    fi
}

# ============ Prompt Markers ============

__golish_prompt_start() {
    __golish_osc "A"
}

__golish_prompt_end() {
    __golish_osc "B"
}

__golish_cmd_start() {
    local cmd="$1"
    if [[ -n "$cmd" ]]; then
        __golish_osc "C;$cmd"
    else
        __golish_osc "C"
    fi
}

__golish_cmd_end() {
    local exit_code=${1:-0}
    __golish_osc "D;$exit_code"
}

# ============ Hook Functions ============

__golish_preexec() {
    __golish_cmd_start "$1"
}

__golish_precmd() {
    local exit_code=$?
    __golish_cmd_end $exit_code
    __golish_report_cwd
    __golish_report_venv
    __golish_prompt_start
}

__golish_line_init() {
    __golish_prompt_end
}

# ============ Register Hooks ============

autoload -Uz add-zsh-hook

add-zsh-hook -d preexec __golish_preexec 2>/dev/null
add-zsh-hook -d precmd __golish_precmd 2>/dev/null

add-zsh-hook preexec __golish_preexec
add-zsh-hook precmd __golish_precmd

if [[ -o zle ]]; then
    if (( ${+functions[zle-line-init]} )); then
        functions[__golish_orig_zle_line_init]="${functions[zle-line-init]}"
        zle-line-init() {
            __golish_orig_zle_line_init
            __golish_line_init
        }
    else
        zle-line-init() {
            __golish_line_init
        }
    fi
    zle -N zle-line-init
fi

__golish_report_cwd
__golish_report_venv
"#;

/// The bash integration script that emits OSC 133 sequences.
/// Uses `PROMPT_COMMAND` for `precmd` and a `DEBUG` trap for `preexec`.
///
/// IMPORTANT: the `DEBUG` trap is installed lazily on the first prompt to
/// avoid capturing commands from `.bashrc` during shell startup.
///
/// Note on OSC 133;B (PromptEnd): we emit `B` immediately after `A` in
/// `precmd`. This means:
/// - A→B transition happens atomically (Prompt region is effectively empty)
/// - PS1 renders in Input region (visible in terminal, filtered from timeline)
/// - PS2 continuation prompts are in Input region (visible)
/// - User input is in Input region (visible)
/// - C is emitted in `preexec` when command actually starts
/// - Command output is in Output region (shown in timeline)
pub(super) const BASH_INTEGRATION_SCRIPT: &str = r#"# Golish Shell Integration for Bash (auto-injected)
# Emits OSC 133 sequences for command tracking

# Guard against double-sourcing
if [[ -n "$__QBIT_OSC133_LOADED" ]]; then
    return 0 2>/dev/null || exit 0
fi
export __QBIT_OSC133_LOADED=1

# ============ State Variables ============

# Track whether we're at the start of a command (to avoid duplicate preexec)
__golish_at_prompt=0
# Flag to install DEBUG trap on first prompt (avoids capturing .bashrc commands)
__golish_trap_installed=0

# ============ OSC Helpers ============

__golish_osc() {
    printf '\e]133;%s\a' "$1"
}

__golish_report_cwd() {
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-$(hostname)}" "$PWD"
}

__golish_report_venv() {
    if [[ -n "$VIRTUAL_ENV" ]]; then
        printf '\e]1337;VirtualEnv=%s\a' "${VIRTUAL_ENV##*/}"
    else
        printf '\e]1337;VirtualEnv=\a'
    fi
}

# ============ Prompt Markers ============

__golish_prompt_start() {
    __golish_osc "A"
}

__golish_prompt_end() {
    __golish_osc "B"
}

__golish_cmd_start() {
    local cmd="$1"
    if [[ -n "$cmd" ]]; then
        __golish_osc "C;$cmd"
    else
        __golish_osc "C"
    fi
}

__golish_cmd_end() {
    __golish_osc "D;${1:-0}"
}

# ============ Hook Functions ============

# Preexec: called before each command via DEBUG trap
__golish_preexec() {
    # Only run if we're at a prompt (not during prompt rendering or subshells)
    [[ "$__golish_at_prompt" != "1" ]] && return

    # Get the command being executed
    local cmd="$BASH_COMMAND"

    # Skip our own functions
    [[ "$cmd" == *"__golish_"* ]] && return

    # Skip shell internals (return from functions, etc.)
    [[ "$cmd" == "return"* ]] && return

    __golish_at_prompt=0
    # Only emit CommandStart here - PromptEnd (B) was already emitted in precmd
    __golish_cmd_start "$cmd"
}

# Precmd: called before each prompt via PROMPT_COMMAND
__golish_precmd() {
    local exit_code=$?

    # Install DEBUG trap on first prompt (after .bashrc has finished)
    if [[ "$__golish_trap_installed" == "0" ]]; then
        __golish_trap_installed=1
        # Chain with any existing DEBUG trap
        local existing_trap
        existing_trap=$(trap -p DEBUG 2>/dev/null | sed "s/trap -- '\\(.*\\)' DEBUG/\\1/")
        if [[ -n "$existing_trap" && "$existing_trap" != "__golish_preexec" ]]; then
            eval "__golish_orig_debug_trap() { $existing_trap; }"
            trap '__golish_preexec; __golish_orig_debug_trap' DEBUG
        else
            trap '__golish_preexec' DEBUG
        fi
    fi

    # Emit command end if we ran a command
    if [[ "$__golish_at_prompt" != "1" ]]; then
        __golish_cmd_end $exit_code
    fi

    __golish_report_cwd
    __golish_report_venv
    __golish_prompt_start
    # Emit PromptEnd immediately after PromptStart
    # This makes the Prompt region effectively empty, and puts PS1/PS2/input
    # in the Input region where they are visible in the terminal but filtered
    # from command block output (which only shows Output region: C to D)
    __golish_prompt_end

    __golish_at_prompt=1
    return $exit_code
}

# ============ Setup ============

# Install PROMPT_COMMAND (DEBUG trap is installed lazily on first prompt)
if [[ -z "$PROMPT_COMMAND" ]]; then
    PROMPT_COMMAND="__golish_precmd"
elif [[ "$PROMPT_COMMAND" != *"__golish_precmd"* ]]; then
    PROMPT_COMMAND="__golish_precmd; $PROMPT_COMMAND"
fi
"#;

/// The wrapper `.zshrc` that sources our integration BEFORE the user's
/// config. This ensures our hooks run even if the user's `.zshrc` has old
/// integration lines.
pub(super) const ZSH_WRAPPER_ZSHRC: &str = r#"# Golish ZDOTDIR wrapper - sources integration + user config

# Debug: confirm wrapper is being sourced
[[ -n "$QBIT_DEBUG" ]] && echo "[golish-wrapper] ZDOTDIR wrapper .zshrc loading..."
[[ -n "$QBIT_DEBUG" ]] && echo "[golish-wrapper] QBIT_INTEGRATION_PATH=$QBIT_INTEGRATION_PATH"

# Source Golish integration FIRST (before user config)
# This ensures our OSC 133 hooks are always registered, even if user's
# .zshrc has an old integration line that would set QBIT_INTEGRATION_LOADED
if [[ -f "$QBIT_INTEGRATION_PATH" ]]; then
    source "$QBIT_INTEGRATION_PATH"
fi

# Now source the user's original .zshrc
# If it has an old integration line, the guard will skip it (QBIT_INTEGRATION_LOADED=1)
if [[ -n "$QBIT_REAL_ZDOTDIR" && "$QBIT_REAL_ZDOTDIR" != "$ZDOTDIR" ]]; then
    # Guard: skip sourcing when QBIT_REAL_ZDOTDIR points back at this wrapper
    # dir (nested Golish). Without this check we'd source ourselves infinitely.
    if [[ -f "$QBIT_REAL_ZDOTDIR/.zshrc" ]]; then
        ZDOTDIR="$QBIT_REAL_ZDOTDIR"
        source "$QBIT_REAL_ZDOTDIR/.zshrc"
    fi
elif [[ -z "$QBIT_REAL_ZDOTDIR" && -f "$HOME/.zshrc" ]]; then
    source "$HOME/.zshrc"
fi
"#;
