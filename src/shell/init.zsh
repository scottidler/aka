# aka - [a]lso [k]nown [a]s: shell alias expansion
# Add to your .zshrc: eval "$(aka shell-init zsh)"
#
# Features:
#   - Space triggers alias expansion while typing
#   - Enter (accept-line) expands aliases before execution
#   - Up/Down arrows: prefix search when typing, cursor-at-end when empty
#   - Ctrl+t opens fuzzy search for aliases (requires sk or fzf)
#   - Killswitch: create ~/aka-killswitch file to disable

# -----------------------------------------------------------------------------
# Killswitch - create ~/aka-killswitch to disable all aka functionality
# -----------------------------------------------------------------------------
_aka_killswitch() {
    [[ -f ~/aka-killswitch ]] && return 1
    return 0
}

# -----------------------------------------------------------------------------
# Space key handler - expand aliases as you type
# -----------------------------------------------------------------------------
_aka_expand_space() {
    _aka_killswitch || { zle self-insert; return }

    local output rc
    output=$(aka query "$BUFFER" 2>/dev/null)
    rc=$?

    if [[ $rc -eq 0 && -n "$output" ]]; then
        BUFFER="$output"
        CURSOR=${#BUFFER}
    else
        zle self-insert
    fi
}

# -----------------------------------------------------------------------------
# Accept-line handler - expand aliases before command execution
# -----------------------------------------------------------------------------
_aka_accept_line() {
    _aka_killswitch || { zle .accept-line; return }

    local output rc
    output=$(aka --eol query "$BUFFER" 2>/dev/null)
    rc=$?

    if [[ $rc -eq 0 && -n "$output" ]]; then
        BUFFER="$output"
        CURSOR=${#BUFFER}
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

    alias_line=$(aka ls 2>/dev/null | $fuzzy_cmd --prompt "aka> " 2>/dev/null)

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
# Smart history navigation - hybrid prefix search / cursor-at-end
# -----------------------------------------------------------------------------
# When buffer has text: prefix search (find commands starting with typed text)
# When buffer is empty: recall history with cursor at end + trailing space
_aka_history_up() {
    if [[ -n "$BUFFER" ]]; then
        zle history-beginning-search-backward
    else
        zle up-line-or-history
        [[ "${BUFFER: -1}" != " " ]] && BUFFER+=" "
        CURSOR=$#BUFFER
    fi
}

_aka_history_down() {
    if [[ -n "$BUFFER" ]]; then
        zle history-beginning-search-forward
    else
        zle down-line-or-history
        [[ "${BUFFER: -1}" != " " ]] && BUFFER+=" "
        CURSOR=$#BUFFER
    fi
}

# -----------------------------------------------------------------------------
# Key bindings
# -----------------------------------------------------------------------------
bindkey " " _aka_expand_space
bindkey -M isearch " " magic-space
bindkey "^t" _aka_search

# History navigation (both normal and application mode escape sequences)
zle -N _aka_history_up
zle -N _aka_history_down
bindkey '^[[A' _aka_history_up
bindkey '^[[B' _aka_history_down
bindkey '^[OA' _aka_history_up
bindkey '^[OB' _aka_history_down

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

