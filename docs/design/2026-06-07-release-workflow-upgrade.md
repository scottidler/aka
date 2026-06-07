# Release Workflow Upgrade

Bring `.github/workflows/binary-release.yml` up to the pattern shared by
`scottidler/manifest`, `scottidler/otto`, and `tatari-tv/claude-cost-usage`.

## What aka has today

`binary-release.yml` builds **3** targets and uses older action/runtime versions:

| Target | Built? |
|--------|:------:|
| `linux-amd64` (`x86_64-unknown-linux-gnu`) | yes |
| `linux-arm64` (`aarch64-unknown-linux-gnu`) | **no** |
| `macos-x86_64` (`x86_64-apple-darwin`) | yes |
| `macos-arm64` (`aarch64-apple-darwin`) | yes |

- `actions/checkout@v4`
- `actions/upload-artifact@v4`
- `actions/download-artifact@v4`
- `softprops/action-gh-release@v2` (runs on the now-deprecated Node.js 20)

## Target: 4 targets + current action/runtime versions

The reference repos build **4** targets (adds cross-compiled `linux-arm64`) and
pin newer actions. There are three changes to make. aka's extra packaging steps
(`_aka_commands`, `aka-loader.zsh`, `aka.zsh` alongside the binary) stay exactly
as they are.

### 1. Move every action onto a Node.js 24 runtime

Node.js 20 is deprecated on GitHub runners; the default flips to Node 24 on
2026-06-16. The correct fix is to use action versions that target Node 24
**natively** — not the `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` env var, which only
masks the warning while the action's own `action.yml` still declares `node20`.

| Action | From | To | Runtime at target |
|--------|------|----|-------------------|
| `actions/checkout` | `@v4` | `@v6` | node24 |
| `actions/upload-artifact` | `@v4` | `@v7` | node24 |
| `actions/download-artifact` | `@v4` | `@v8` | node24 |
| `softprops/action-gh-release` | `@v2` | `@v3` | node24 |
| `Swatinem/rust-cache` | `@v2` | `@v2` | node24 (already) |

`action-gh-release@v3.0.0` is a runtime-only major bump (Node 20 → 24) with no
input or behavior changes, so the `files:`/`GITHUB_TOKEN` usage is unchanged.
With all five actions on node24, **no `FORCE_*` env var is needed** — leave the
`env:` block as just `RUST_VERSION` + `CARGO_TERM_COLOR`.

### 3. Add the `linux-arm64` cross-compiled target

In the `build-linux` job, expand the matrix and add a cross-compile toolchain
step plus the linker env on the build step:

```yaml
  build-linux:
    runs-on: ubuntu-latest
    container: debian:bookworm
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            suffix: linux-amd64
            cross: false
          - target: aarch64-unknown-linux-gnu
            suffix: linux-arm64
            cross: true
    steps:
      - name: Install build dependencies
        run: |
          apt-get update
          apt-get install -y curl build-essential git pkg-config libssl-dev

      - name: Install cross-compilation toolchain
        if: matrix.cross
        run: apt-get install -y gcc-aarch64-linux-gnu

      # ... checkout, GIT_DESCRIBE, rust setup, cache (unchanged) ...

      - name: Build for ${{ matrix.target }}
        env:
          CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER: ${{ matrix.cross && 'aarch64-linux-gnu-gcc' || '' }}
        run: cargo build --release --target ${{ matrix.target }}
```

The `macos` matrix already covers both arches and needs no change.

## Optional: `workflow_dispatch`

`manifest` also added a manual trigger so a release build can be re-run without
cutting a new tag:

```yaml
on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:
```

## Gotcha: a brand-new workflow won't fire on the tag that introduces it

GitHub Actions only runs a tag-triggered workflow if the workflow file
**already exists on the repository's default branch** when the tag is pushed.
If the workflow file is added and the version tag is pushed in the same step,
GitHub registers the workflow from the branch update but creates **no Actions
run for that tag** — that release ships no artifacts.

This is deterministic, not a timing flake (confirmed on `manifest`: the
`v0.1.7` tagged commit had no `github-actions` check suite at all because that
commit was the one that introduced the workflow). Since aka's workflow already
exists on `main`, the version bumps above are safe to land normally and the
next tag will build as expected. The rule only bites the *first* tag of a
brand-new workflow.
