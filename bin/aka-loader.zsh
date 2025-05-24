# aka-loader.zsh — bootstrap for aka expansion
aka_zsh="${XDG_CONFIG_HOME:-$HOME/.config}/aka/aka.zsh"

if hash aka 2>/dev/null && [ -f "$aka_zsh" ]; then
  export EXPAND_AKA=yes
  . "$aka_zsh"
fi
