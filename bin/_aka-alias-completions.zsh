# bin/_aka-alias-completions.zsh

# Skip if aka isn't available
if ! hash aka 2>/dev/null; then
  return 0
fi

# Custom command name completer for aka aliases
_aka_aliases_commands() {
  local -a commands
  commands=("${(@f)$(aka __complete_aliases)}")
  _describe -t commands 'aka alias' commands
}

# Attach our command name provider into the command context
compdef _aka_aliases_commands command
