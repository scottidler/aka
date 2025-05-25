# bin/_aka-alias-completions.zsh
# Registers aka aliases as top-level command completions
# for zsh-autocomplete, zsh-autosuggestions, and fzf-tab.

if ! hash aka 2>/dev/null; then
  return 0
fi

_aka_aliases() {
  reply=("${(@f)$(aka __complete_aliases)}")
}

# Register completions as if aliases were real commands
compdef _aka_aliases -command-
