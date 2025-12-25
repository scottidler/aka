# aka — **[a]lso [k]nown [a]s**
Small, composable command-aliasing for Z shell powered by Rust & ZLE
[![Crates.io](https://img.shields.io/crates/v/aka.svg)](https://crates.io/crates/aka)

`aka` lets you write short _handles_ that get expanded into full command lines **while you type**, with no shell aliases to maintain and no functions polluting your namespace.
The heavy lifting happens in Z shell's *Z*-line-*E*ditor (ZLE) — see the [ZLE manual](https://zsh.sourceforge.net/Doc/Release/Zsh-Line-Editor.html) for the underlying primitives.

---

## ✨ What makes it different?

* **Context-aware expansion** – spacebar expands only the first word, <kbd>Enter</kbd> can rewrite the whole line.
* **Lookups & templating** – splice dynamic values (`lookup:region[prod] -> us-east-1`).
* **`!` sudo triggers** – append **`!`** to re-exec the line under `sudo`.
* **Non-blocking killswitch** – create `~/aka-killswitch` to temporarily disable all expansions.
* **Zero-cost opt-out** – if no substitution happens the line is echoed untouched.

---

## 🛠 Installation

### 1. Get the binary

```console
cargo install aka
# or, from a local checkout
git clone https://github.com/scottidler/aka.git
cd aka
cargo install --path .
```

### 2. Wire ZLE into your shell

**Recommended: eval pattern** (industry standard, like starship/zoxide)

Add this to your `~/.zshrc`:

```zsh
if hash aka 2>/dev/null; then
    eval "$(aka shell-init zsh)"
fi
```

This ensures the shell integration always matches your installed binary version.

<details>
<summary>Alternative: Using a loader script</summary>

If you prefer using a loader file in `~/.shell-functions.d/`:

```console
mkdir -p ~/.shell-functions.d
cat > ~/.shell-functions.d/aka-loader.zsh << 'EOF'
#!/usr/bin/env zsh
if hash aka 2>/dev/null; then
    export EXPAND_AKA=yes
    eval "$(aka shell-init zsh)"
fi
EOF
```

Your `.zshrc` should source files from `~/.shell-functions.d/`:

```zsh
# ~/.zshrc
if [ -d ~/.shell-functions.d/ ]; then
    for f in ~/.shell-functions.d/*; do . $f; done
fi
```
</details>

The shell integration registers these ZLE widgets:

| Key binding                    | Widget                           | What it does |
|--------------------------------|----------------------------------|--------------|
| <kbd>Space</kbd>               | `_aka_expand_space`              | Expand the first word if it matches an alias. |
| <kbd>Enter</kbd> (`accept-line`)| `_aka_accept_line`              | Rewrites the command just before execution. |
| <kbd>Ctrl-T</kbd>              | `_aka_search` (via `sk`/`fzf`)   | Fuzzy-find & insert an alias. |

All widgets are defined in the embedded script (viewable via `aka shell-init zsh`).

---

## 📂 Configuration

`aka` looks for the first existing file among:

1. `~/.config/aka/aka.yml`
2. `~/.config/aka/aka.yaml`
3. `~/aka.yml`
4. `~/aka.yaml`
5. `~/.aka.yml`
6. `~/.aka.yaml`

### Minimal example

```yaml
# ~/.config/aka/aka.yml
defaults:
  version: 1
aliases:
  cat: "bat -p"          # replace 'cat' with bat
  '|c':
    value: "| xclip -sel clip"
    global: true         # works in the middle of a pipeline
lookups:                 # dynamic substitutions
  region:
    prod|apps: us-east-1
    dev|test|staging: us-west-2
```

* `$@`, `$1-$9`, and `$name` style tokens are all supported — see [`cfg::alias`](src/cfg/alias.rs) for the parser.
* `lookup:<table>[<key>]` resolves at runtime via the `lookups` map.

---

## 🚀 Running

```console
aka ls [-g] [PATTERN…]        # list aliases (optionally globals only)
aka query "some command line" # test what would expand
aka freq [-a]                 # show alias usage frequency
aka shell-init [SHELL]        # print shell initialization script
aka daemon --status           # check daemon status
```

For everyday use you never call `aka` directly — the ZLE widgets call it for you.

---

## 🔍 Code tour

| Area | Description |
|------|-------------|
| [`src/bin/aka.rs`](src/bin/aka.rs) | CLI entry point and command handling |
| [`src/lib.rs`](src/lib.rs) | Core replace engine and shared logic |
| [`src/cfg/`](src/cfg) | YAML loader, alias/lookup deserialisation |
| [`src/shell/`](src/shell) | Shell initialization scripts (embedded via `include_str!`) |
| [`src/shell/init.zsh`](src/shell/init.zsh) | ZSH widgets and key-bindings |
| [`build.rs`](build.rs) | Injects `git describe` into the binary's `--version` |

---

## 🩹 Troubleshooting

* **Nothing expands** – touching `~/aka-killswitch` disables expansions; delete the file.
* **Debug logging** – Logs are written to `~/.local/share/aka/logs/aka.log`
* **View shell script** – Run `aka shell-init zsh` to see the exact script being loaded.
* **Widgets conflict** – Override bindings in your `.zshrc` after the `eval` line.

---

## 📜 License

Licensed under MIT – see [LICENSE](LICENSE).

Happy aliasing! ✨
