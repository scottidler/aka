# aka — **[a]lso [k]nown [a]s**
Small, composable command-aliasing for Z shell powered by Rust & ZLE
[![Crates.io](https://img.shields.io/crates/v/aka.svg)](https://crates.io/crates/aka)

`aka` lets you write short _handles_ that get expanded into full command lines **while you type**, with no shell aliases to maintain and no functions polluting your namespace.
The heavy lifting happens in Z shell’s *Z*-line-*E*ditor (ZLE) — see the [ZLE manual](https://zsh.sourceforge.net/Doc/Release/Zsh-Line-Editor.html) for the underlying primitives.

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

<details>
<summary>If you already keep autoloadable functions in <code>~/.shell-functions.d/</code></summary>

```console
mkdir -p ~/.shell-functions.d
ln -sf "$HOME/.cargo/bin/aka-loader.zsh" ~/.shell-functions.d/00-aka-loader.zsh
```

Your `.zshrc` (or whatever `$ZDOTDIR/.zshrc` you use) should already source `~/.shell-functions.d/**/*`.
If not:

```zsh
# ~/.zshrc
fpath=(~/.shell-functions.d $fpath)
autoload -Uz ~/.shell-functions.d/00-aka-loader.zsh
```
</details>

<details>
<summary>If you prefer <code>$XDG_CONFIG_HOME</code> (recommended)</summary>

```console
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/aka"
ln -sf "$HOME/.cargo/bin/aka-loader.zsh" \
      "${XDG_CONFIG_HOME:-$HOME/.config}/aka/aka-loader.zsh"

# Then, in ~/.zshrc:
source "${XDG_CONFIG_HOME:-$HOME/.config}/aka/aka-loader.zsh"
```
</details>

`aka-loader.zsh` is a two-liner that checks for the `aka` binary **and** your config, then loads [`bin/aka.zsh`](bin/aka.zsh) which registers the ZLE widgets:

| Key binding                    | Widget                           | What it does |
|--------------------------------|----------------------------------|--------------|
| <kbd>Space</kbd>               | `expand-aka-space`               | Expand the first word if it matches an alias. |
| <kbd>Enter</kbd> (`accept-line`)| `expand-aka-accept-line`        | Rewrites the command just before execution. |
| <kbd>Ctrl-T</kbd>              | `aka-search` (via `sk`/`fzf`)    | Fuzzy-find & insert an alias. |
| ↑ (Up arrow)                   | `up-line-or-add-space`           | Recall history *and* keep trailing space for chaining. |

All widgets are defined in [`bin/aka.zsh`](bin/aka.zsh) and glued in with `zle -N`
— worth a read if you are curious about ZLE magic.

---

## 📂 Configuration

`aka` looks for the first existing file among

1. `./aka.yml`
2. `~/.aka.yml`
3. `${XDG_CONFIG_HOME:-$HOME/.config}/aka/aka.yml`

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
```

For everyday use you never call `aka` directly — the ZLE widgets call it for you.

---

## 🔍 Code tour

| Area | Description |
|------|-------------|
| [`src/main.rs`](src/main.rs) | CLI + core replace engine + tests |
| [`src/cfg/`](src/cfg) | YAML loader, alias/lookup deserialisation |
| [`bin/aka.zsh`](bin/aka.zsh) | ZLE widgets and key-bindings |
| [`bin/aka-loader.zsh`](bin/aka-loader.zsh) | Minimal bootstrap sourced from `.zshrc` |
| [`build.rs`](build.rs) | Injects `git describe` into the binary’s `--version` |

---

## 🩹 Troubleshooting

* **Nothing expands** – touching `~/aka-killswitch` disables expansions; delete the file.
* **Debug logging** – `export AKA_LOG=1` will append every query/answer to `~/aka.txt`.
* **Widgets conflict** – rebind them in your own shell after sourcing `aka-loader.zsh`.

---

## 📜 License

Licensed under MIT – see [LICENSE](LICENSE).

Happy aliasing! ✨
