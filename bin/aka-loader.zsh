#!/usr/bin/env zsh
# bin/aka-loader.zsh

# live-expansion hooks
aka_zsh="${XDG_CONFIG_HOME:-$HOME/.config}/aka/aka.zsh"
if hash aka 2>/dev/null && [ -f "$aka_zsh" ]; then
  export EXPAND_AKA=yes
  source "$aka_zsh"
fi
