#!/bin/zsh

log() {
  if [[ -n $AKA_LOG ]]; then
    echo "$@" >> ~/aka.txt
  fi
}

# Health check function with caching
aka_health_check() {
    # Check for killswitch first
    if [ -f ~/aka-killswitch ]; then
        return 1
    fi

    # Perform health check
    aka __health_check 2>/dev/null
    local health_result=$?

    # Handle different health check results
    case $health_result in
        0)
            # All good
            return 0
            ;;
        1)
            # Config file not found - disable aka temporarily
            log "aka: config file not found, disabling expansions"
            return 1
            ;;
        2)
            # Config file invalid - disable aka temporarily
            log "aka: config file invalid, disabling expansions"
            return 2
            ;;
        3)
            # No aliases defined - continue but no expansions needed
            log "aka: no aliases defined"
            return 3
            ;;
        *)
            # Other errors - disable aka temporarily
            log "aka: health check failed with code $health_result, disabling expansions"
            return $health_result
            ;;
    esac
}

# Expands a keyword into a longer command using `aka query` upon pressing space
expand-aka-space() {
    aka_health_check
    if [ $? -eq 0 ]; then
        log "expand-aka-space: BUFFER=$BUFFER"
        OUTPUT=$(aka query "$BUFFER" 2>/dev/null)
        RC=$?
        log "expand-aka-space: OUTPUT=$OUTPUT RC=$RC"

        if [ $RC -eq 0 ] && [ -n "$OUTPUT" ]; then
            BUFFER="$OUTPUT"
            log "expand-aka-space: CURSOR=$CURSOR"
            CURSOR=$(expr length "$BUFFER")
            log "expand-aka-space: CURSOR(after assignment)=$CURSOR"
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
    aka_health_check
    if [ $? -eq 0 ]; then
        log "expand-aka-accept-line: BUFFER=$BUFFER"
        OUTPUT=$(aka --eol query "$BUFFER" 2>/dev/null)
        RC=$?
        log "expand-aka-accept-line: OUTPUT=$OUTPUT RC=$RC"

        if [ $RC -eq 0 ] && [ -n "$OUTPUT" ]; then
            BUFFER="$OUTPUT"
            log "expand-aka-accept-line: CURSOR=$CURSOR"
            CURSOR=$(expr length "$BUFFER")
            log "expand-aka-accept-line: CURSOR(after assignment)=$CURSOR"
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
    aka_health_check
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
