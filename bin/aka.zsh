#!/bin/zsh

# Check if aka is disabled via killswitch
aka_killswitch() {
    # Check for killswitch first
    if [ -f ~/aka-killswitch ]; then
        return 1
    fi
    return 0
}

# Expands a keyword into a longer command using `aka query` upon pressing space
expand-aka-space() {
    aka_killswitch
    if [ $? -eq 0 ]; then
        OUTPUT=$(aka query "$BUFFER")
        RC=$?

        if [ $RC -eq 0 ] && [ -n "$OUTPUT" ]; then
            BUFFER="$OUTPUT"
            CURSOR=$(expr length "$BUFFER")
        else
            zle self-insert
        fi
    else
        zle self-insert
    fi
}
zle -N expand-aka-space
bindkey " " expand-aka-space
bindkey -M isearch " " magic-space

# Expands a keyword into a longer command using `aka --eol query` before executing the command
expand-aka-accept-line() {
    aka_killswitch
    if [ $? -eq 0 ]; then
        OUTPUT=$(aka --eol query "$BUFFER")
        RC=$?

        if [ $RC -eq 0 ] && [ -n "$OUTPUT" ]; then
            BUFFER="$OUTPUT"
            CURSOR=$(expr length "$BUFFER")
        fi
        zle .accept-line
    else
        zle .accept-line
    fi
}
zle -N accept-line expand-aka-accept-line

# Adds a trailing space to every command line before it's stored in history
function add-space-to-command() {
    if [[ "${BUFFER: -1}" != " " ]]; then
        BUFFER+=" "
        CURSOR=$#BUFFER
    fi
}
zle -N zle-line-finish add-space-to-command

# Recalls previous command from history and ensures it ends with a space upon pressing up-arrow
function up-line-or-add-space() {
    zle up-line-or-history
    [[ $LBUFFER != *' '* ]] && zle backward-delete-char
    LBUFFER+=' '
}
zle -N up-line-or-add-space
bindkey '^[[A' up-line-or-add-space

# Define a function to search through aka aliases
aka-search() {
    aka_killswitch
    if [ $? -eq 0 ]; then
        # Run 'aka ls', pipe the output to sk
        local alias=$(aka ls 2>/dev/null | sk --prompt "aka> ")

        # If an alias was selected (user didn't press escape)
        if [[ -n $alias ]]; then
            # Extract the alias name (everything before the first ':')
            local alias_name=${alias%%:*}

            # Type the alias at the command prompt
            LBUFFER+=$alias_name
        fi
    fi
}
zle -N aka-search

# Bind the function to a shortcut key
# Here we're using Ctrl+t, but you can choose a different shortcut
bindkey '^t' aka-search
