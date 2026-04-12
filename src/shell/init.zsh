# aka - [a]lso [k]nown [a]s: shell alias expansion
# Add to your .zshrc: eval "$(aka shell-init zsh)"
#
# Features:
#   - Space triggers alias expansion while typing
#   - Enter (accept-line) expands aliases before execution
#   - Ctrl+t opens fuzzy search for aliases (requires sk or fzf)
#   - Killswitch: create ~/aka-killswitch file to disable
#   - Circuit breaker: disables after 5 consecutive failures per session

# -----------------------------------------------------------------------------
# Killswitch - create ~/aka-killswitch to disable all aka functionality
# -----------------------------------------------------------------------------
_aka_killswitch() {
    [[ -f ~/aka-killswitch ]] && return 1
    return 0
}

# -----------------------------------------------------------------------------
# Circuit breaker state - per session, never resets while shell is open
# -----------------------------------------------------------------------------
typeset -gi _AKA_FAIL_COUNT=0
typeset -gi _AKA_SESSION_DISABLED=0

_aka_on_failure() {
    (( _AKA_FAIL_COUNT++ ))
    if (( _AKA_FAIL_COUNT == 1 )); then
        zle -M "aka: config error - run 'aka check'"
    elif (( _AKA_FAIL_COUNT >= 5 && !_AKA_SESSION_DISABLED )); then
        _AKA_SESSION_DISABLED=1
        zle -M "aka: disabled this session after 5 failures - run 'aka check' or restart shell"
    fi
}

_aka_on_success() {
    _AKA_FAIL_COUNT=0
}

# -----------------------------------------------------------------------------
# Space key handler - expand aliases as you type
# -----------------------------------------------------------------------------
_aka_expand_space() {
    _aka_killswitch || { zle self-insert; return }
    (( _AKA_SESSION_DISABLED )) && { zle self-insert; return }

    local output rc
    output=$(aka query "$BUFFER" 2>/dev/null)
    rc=$?

    if [[ $rc -eq 0 && -n "$output" ]]; then
        _aka_on_success
        POSTDISPLAY=""  # Clear autosuggestion ghost text first
        BUFFER="$output"
        CURSOR=${#BUFFER}
        zle reset-prompt  # Full prompt redraw to clear visual artifacts
    elif [[ $rc -ne 0 && -z "$output" ]]; then
        _aka_on_failure
        zle self-insert
    else
        zle self-insert
    fi
}

# -----------------------------------------------------------------------------
# Accept-line handler - expand aliases before command execution
# -----------------------------------------------------------------------------
_aka_accept_line() {
    _aka_killswitch || { zle .accept-line; return }
    (( _AKA_SESSION_DISABLED )) && { zle .accept-line; return }

    local output rc
    output=$(aka --eol query "$BUFFER" 2>/dev/null)
    rc=$?

    if [[ $rc -eq 0 && -n "$output" ]]; then
        _aka_on_success
        POSTDISPLAY=""  # Clear autosuggestion ghost text first
        BUFFER="$output"
        CURSOR=${#BUFFER}
        zle reset-prompt  # Full prompt redraw to clear stale syntax highlighting
    elif [[ $rc -ne 0 && -z "$output" ]]; then
        _aka_on_failure
    fi
    zle .accept-line
}

# -----------------------------------------------------------------------------
# History helper - adds trailing space for cleaner history recall
# -----------------------------------------------------------------------------
_aka_add_space_to_command() {
    if [[ "${BUFFER: -1}" != " " ]]; then
        BUFFER+=" "
        CURSOR=$#BUFFER
    fi
}

# -----------------------------------------------------------------------------
# Fuzzy alias search - Ctrl+t to search and insert alias
# -----------------------------------------------------------------------------
_aka_search() {
    _aka_killswitch || return

    local alias_line fuzzy_cmd

    # Prefer sk (skim) over fzf, fall back gracefully
    if command -v sk >/dev/null 2>&1; then
        fuzzy_cmd="sk"
    elif command -v fzf >/dev/null 2>&1; then
        fuzzy_cmd="fzf"
    else
        zle -M "aka-search requires sk or fzf"
        return
    fi

    alias_line=$(aka ls 2>/dev/null | $fuzzy_cmd --ansi --prompt "aka> " 2>/dev/null)

    if [[ -n "$alias_line" ]]; then
        # Extract alias name (everything before first space and arrow)
        local alias_name="${alias_line%%->*}"
        alias_name="${alias_name%% *}"
        alias_name="${alias_name## }"  # trim leading spaces
        LBUFFER+="$alias_name"
    fi
    zle reset-prompt
}

# -----------------------------------------------------------------------------
# Register ZLE widgets
# -----------------------------------------------------------------------------
zle -N _aka_expand_space
zle -N _aka_accept_line
zle -N _aka_search
zle -N zle-line-finish _aka_add_space_to_command

# Override accept-line with our wrapper
zle -N accept-line _aka_accept_line

# -----------------------------------------------------------------------------
# Key bindings
# -----------------------------------------------------------------------------
bindkey " " _aka_expand_space
bindkey -M isearch " " magic-space
bindkey "^t" _aka_search

# -----------------------------------------------------------------------------
# Completion support - register aka alias completion using -first- context
# This runs BEFORE other completers and adds aka aliases without blocking them
# See: man zshcompsys, search for "-first-"
# -----------------------------------------------------------------------------
if (( $+functions[compdef] )); then
    _aka_complete_commands() {
        # Only add completions when in command position
        [[ $compstate[context] == command ]] || return 1

        local -a aka_aliases
        aka_aliases=(${(f)"$(aka __complete_aliases 2>/dev/null)"})
        (( ${#aka_aliases} )) && compadd -a aka_aliases

        # Return 0 but don't set _compskip - allows other completers to also run
        return 0
    }
    compdef _aka_complete_commands -first-
fi

# -----------------------------------------------------------------------------
# Startup health check - warn once if config is already broken when shell opens
# -----------------------------------------------------------------------------
if command -v aka >/dev/null 2>&1; then
    aka check --quiet 2>/dev/null || \
        echo "⚠️  aka: config error - run 'aka check' to diagnose" >&2
fi
