use crate::pty::ShellType;

pub(crate) const INTEGRATION_VERSION: &str = "1.1.0";

// =============================================================================
// Zsh Integration Script
// =============================================================================

const INTEGRATION_SCRIPT_ZSH: &str = r#"# ~/.config/golish/integration.zsh
# Golish Shell Integration v1.1.0
# Do not edit - managed by Golish

# Guard against double-sourcing
[[ -n "$QBIT_INTEGRATION_LOADED" ]] && return
export QBIT_INTEGRATION_LOADED=1

# Only run inside Golish
[[ -z "$QBIT" ]] && return

# ============ OSC Helpers ============

__golish_osc() {
    printf '\e]133;%s\e\\' "$1"
}

__golish_report_cwd() {
    printf '\e]7;file://%s%s\e\\' "${HOST:-$(hostname)}" "$PWD"
}

__golish_report_venv() {
    # Report Python virtual environment via OSC 1337
    if [[ -n "$VIRTUAL_ENV" ]]; then
        # Extract venv name from path (last component)
        local venv_name="${VIRTUAL_ENV##*/}"
        printf '\e]1337;VirtualEnv=%s\e\\' "$venv_name"
    else
        # Clear virtual env indicator
        printf '\e]1337;VirtualEnv=\e\\'
    fi
}

__golish_notify() {
    printf '\e]9;%s\e\\' "$1"
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
    QBIT_CMD_START=$EPOCHREALTIME
}

__golish_cmd_end() {
    local exit_code=${1:-0}
    __golish_osc "D;$exit_code"

    if [[ -n "$QBIT_CMD_START" ]]; then
        local duration=$(( ${EPOCHREALTIME%.*} - ${QBIT_CMD_START%.*} ))
        if (( duration > 10 )); then
            __golish_notify "Command finished (${duration}s)"
        fi
    fi
    unset QBIT_CMD_START
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

// =============================================================================
// Bash Integration Script
// =============================================================================

const INTEGRATION_SCRIPT_BASH: &str = r#"# ~/.config/golish/integration.bash
# Golish Shell Integration v1.1.0
# Do not edit - managed by Golish

# Guard against double-sourcing
[[ -n "$QBIT_INTEGRATION_LOADED" ]] && return
export QBIT_INTEGRATION_LOADED=1

# Only run inside Golish
[[ "$QBIT" != "1" ]] && return

# ============ OSC Helpers ============

__golish_osc() {
    printf '\e]133;%s\e\\' "$1"
}

__golish_report_cwd() {
    printf '\e]7;file://%s%s\e\\' "${HOSTNAME:-$(hostname)}" "$PWD"
}

__golish_report_venv() {
    # Report Python virtual environment via OSC 1337
    if [[ -n "$VIRTUAL_ENV" ]]; then
        # Extract venv name from path (last component)
        local venv_name="${VIRTUAL_ENV##*/}"
        printf '\e]1337;VirtualEnv=%s\e\\' "$venv_name"
    else
        # Clear virtual env indicator
        printf '\e]1337;VirtualEnv=\e\\'
    fi
}

# ============ Hook Functions ============

# Track if preexec already ran (DEBUG trap fires multiple times)
__golish_preexec_ran=0

__golish_prompt_command() {
    local exit_code=$?
    __golish_osc "D;$exit_code"
    __golish_report_cwd
    __golish_report_venv
    __golish_osc "A"
    __golish_preexec_ran=0
}

__golish_debug_trap() {
    # Skip if we already ran preexec for this command
    [[ $__golish_preexec_ran -eq 1 ]] && return
    # Skip if this is the PROMPT_COMMAND itself
    [[ "$BASH_COMMAND" == "$PROMPT_COMMAND" ]] && return
    [[ "$BASH_COMMAND" == "__golish_prompt_command"* ]] && return
    __golish_preexec_ran=1
    __golish_osc "C"
}

# ============ Register Hooks ============

# Append to PROMPT_COMMAND (preserving existing)
if [[ -z "$PROMPT_COMMAND" ]]; then
    PROMPT_COMMAND="__golish_prompt_command"
else
    PROMPT_COMMAND="__golish_prompt_command;$PROMPT_COMMAND"
fi

# Set DEBUG trap for preexec behavior
trap '__golish_debug_trap' DEBUG

# Emit B marker in PS1 (prompt end)
PS1="\[\e]133;B\e\\\]$PS1"

__golish_report_cwd
__golish_report_venv
"#;

// =============================================================================
// Fish Integration Script
// =============================================================================

const INTEGRATION_SCRIPT_FISH: &str = r#"# ~/.config/fish/conf.d/golish.fish
# Golish Shell Integration v1.1.0
# Do not edit - managed by Golish

# Guard against double-sourcing
if set -q QBIT_INTEGRATION_LOADED
    exit
end

# Only run inside Golish
if test "$QBIT" != "1"
    exit
end

set -gx QBIT_INTEGRATION_LOADED 1

# ============ OSC Helpers ============

function __golish_osc
    printf '\e]133;%s\e\\' $argv[1]
end

function __golish_report_cwd
    printf '\e]7;file://%s%s\e\\' (hostname) $PWD
end

function __golish_report_venv
    # Report Python virtual environment via OSC 1337
    if set -q VIRTUAL_ENV
        # Extract venv name from path (last component)
        set venv_name (basename $VIRTUAL_ENV)
        printf '\e]1337;VirtualEnv=%s\e\\' $venv_name
    else
        # Clear virtual env indicator
        printf '\e]1337;VirtualEnv=\e\\'
    end
end

# ============ Hook Functions ============

function __golish_preexec --on-event fish_preexec
    __golish_osc "C"
end

function __golish_postexec --on-event fish_postexec
    __golish_osc "D;$status"
    __golish_report_cwd
    __golish_report_venv
end

# ============ Prompt Wrapper ============

# Save original fish_prompt if it exists
if functions -q fish_prompt
    functions -c fish_prompt __golish_original_prompt
else
    function __golish_original_prompt
        echo -n '$ '
    end
end

# Wrap fish_prompt to emit A/B markers
function fish_prompt
    __golish_osc "A"
    __golish_original_prompt
    __golish_osc "B"
end

__golish_report_cwd
__golish_report_venv
"#;

// =============================================================================
// Script Selection
// =============================================================================

/// Get the integration script for a specific shell type
pub fn get_integration_script(shell_type: ShellType) -> &'static str {
    match shell_type {
        ShellType::Zsh => INTEGRATION_SCRIPT_ZSH,
        ShellType::Bash => INTEGRATION_SCRIPT_BASH,
        ShellType::Fish => INTEGRATION_SCRIPT_FISH,
        ShellType::Unknown => INTEGRATION_SCRIPT_ZSH, // Default to zsh for unknown
    }
}

#[cfg(test)]
/// Get the integration script file extension for a shell type
pub(crate) fn get_integration_extension(shell_type: ShellType) -> &'static str {
    match shell_type {
        ShellType::Zsh => "zsh",
        ShellType::Bash => "bash",
        ShellType::Fish => "fish",
        ShellType::Unknown => "zsh",
    }
}
