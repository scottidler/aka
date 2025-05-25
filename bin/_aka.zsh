# ~/.shell-functions.d/_aka.zsh
# This file defines the core completion function for aka, used by:
# - native Zsh completion
# - zsh-autocomplete
# - fzf-tab
# - zsh-autosuggestions (completion strategy)

# Generate the list dynamically
_aka() {
  local -a aliases
  aliases=("${(@f)$(aka __complete_aliases)}")
  _describe 'aka aliases' aliases
}
