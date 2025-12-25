#!/usr/bin/env zsh
# aka-loader.zsh - Shell integration loader for aka
#
# Uses eval pattern to ensure shell integration matches binary version

if hash aka 2>/dev/null; then
    export EXPAND_AKA=yes
    eval "$(aka shell-init zsh)"
fi
