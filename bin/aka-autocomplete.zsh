# File: aka-autocomplete.zsh
# Hook your aka completer into zsh-autocomplete’s live menu

# ensure your custom functions dir is on fpath
fpath=( "${ZDOTDIR:-$HOME}/.shell-functions.d" $fpath )

# make sure _aka_aliases is autoloadable
autoload -Uz _aka_aliases

# register it for the aka command
compdef _aka_aliases aka
