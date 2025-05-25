# File: aka-autosuggestions.zsh
# Provide the aka‐alias completer so that zsh-autosuggestions
# (when using the “completion” strategy) can pick up your keys.

_aka_aliases() {
  reply=("${(@f)$(aka __complete_aliases)}")
}

# register for “aka” (or use '' to allow first-word only)
compdef _aka_aliases ''
