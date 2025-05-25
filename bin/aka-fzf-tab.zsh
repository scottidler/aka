# File: aka-fzf-tab.zsh
# Integration for fzf-tab plugin to enable aka alias completion in the fzf-based menu.
#
# Requirements:
# - fzf-tab plugin must be installed and loaded after compinit
# - aka-tab-completion must be available (via aka-autocomplete.zsh or equivalent)

# Ensure our custom functions directory is on fpath so _aka_aliases is autoloadable
fpath=( "${ZDOTDIR:-$HOME}/.shell-functions.d" $fpath )

# Autoload the alias completion function provided by aka
autoload -Uz _aka_aliases

# Register it for the `aka` command so fzf-tab will include those aliases
compdef _aka_aliases aka
