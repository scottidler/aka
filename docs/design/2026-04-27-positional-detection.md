# Design Document: Bash Positional Parameter Detection

**Author:** Scott Idler
**Date:** 2026-04-27
**Status:** In Review
**Review Passes Completed:** 5/5

## Summary

`aka` detects positional parameters (`$1`-`$9`) in alias values to decide whether
to expand an alias immediately or wait for arguments. The detection regex only
recognises bare `$N` forms. Bash parameter expansion forms like `${1:-.}`,
`${1:-default}`, `${#1}` are invisible to the detector, causing those aliases to
fire immediately and emit unexpanded shell syntax into the command line.

## Problem Statement

### Background

When `aka` evaluates an alias it calls `positionals()` on the alias value. If any
positionals are found the alias is held until the right number of arguments arrive;
if none are found the alias fires immediately (expanding the alias name to its
value). The same regex is reused in `colorize_value()` to highlight positional
tokens in terminal output.

### Problem

`positionals()` in `src/cfg/alias.rs` uses:

```rust
let re = Regex::new(r"(\$[1-9])")?;
```

This matches only bare `$1`–`$9`. Every other standard bash positional form is
invisible to it:

| Form | Example | Detected? |
|------|---------|-----------|
| Bare | `$1` | yes |
| Braced | `${1}` | no |
| Default value | `${1:-.}` | no |
| Alternate value | `${1:+foo}` | no |
| Error on unset | `${1:?msg}` | no |
| Length | `${#1}` | no |
| Substitution | `${1/x/y}` | no |

The same narrow regex appears in `colorize_value()` in `src/lib.rs`:

```rust
let re = Regex::new(r"\$[@1-9]").unwrap();
```

**Concrete failure:** the `cip` alias was written as:

```yaml
cip: |
  cargo metadata --manifest-path "${1:-.}/Cargo.toml" \
    ...
```

Because `${1:-.}` is not detected, `positionals()` returns empty, the alias fires
immediately, and `${1:-.}` lands as a literal unexpanded string in the shell
command.

### Goals

- `positionals()` detects all standard bash `${N...}` positional forms.
- `colorize_value()` highlights those same forms in terminal output.
- Replacement in `replace()` works correctly: the full token (`${1:-.}`) is
  substituted with the supplied argument.
- Deduplication is by positional number, not token string, so `$1` and `${1:-.}`
  in the same value count as one positional, not two.
- All existing tests continue to pass.

### Non-Goals

- Supporting optional positionals (aliases that accept 0 or 1 argument with a
  shell default). That requires a different semantic model and is out of scope.
- Full POSIX/bash shell parsing.
- Handling `${10}` and above (aka already stops at `$9`).
- Supporting `$@` or `$*` via this path (handled separately via `is_variadic()`).

## Proposed Solution

### Overview

Extend the regex in `positionals()` to match the full family of `${N...}` forms,
return the full matched token for use as a replacement target, and deduplicate by
extracted digit so that mixed forms referring to the same positional count as one.
Apply the same regex extension to `colorize_value()`.

### Architecture

Two files change:

- `src/cfg/alias.rs` - `positionals()` and its tests
- `src/lib.rs` - `colorize_value()`

No structural changes. No new dependencies.

### Data Model

`positionals()` currently returns `Vec<String>` of the raw matched tokens (`"$1"`,
`"$2"`, …). The return type stays the same. The content changes: tokens can now be
full bash expansions like `"${1:-.}"`.

The deduplication step currently does a sort + dedup on the string values. That
must change to dedup by extracted digit so that `["$1", "${1:-.}"]` collapses to
one entry.

Which token survives per digit? **The longest token wins.** This is required for
correctness. `str::replace` is a naive substring match: if `$1` is chosen as the
token and the value also contains `${1:-.}`, replacing `"$1"` clobbers the inner
`$1` of the braced form, producing invalid shell syntax like `${foo:-.}`. Choosing
the longest token avoids this — `replace("${1:-.}", arg)` is a precise match that
leaves the bare `$1` literal (and vice versa). The `BTreeMap<char, String>` keyed
on positional digit naturally provides ascending order and longest-wins semantics:

```rust
by_digit.entry(digit)
    .and_modify(|existing| if token.len() > existing.len() { *existing = token.clone() })
    .or_insert(token);
```

If an alias mixes both `$1` and `${1:-.}`, the shorter form survives literal in the
output (for the shell to expand). This is a degenerate alias and is not a supported
pattern; authors should use a single consistent form per positional number.

### API Design

`positionals()` signature is unchanged:

```rust
pub fn positionals(&self) -> Result<Vec<String>>
```

### Implementation Plan

#### Phase 1: Update `positionals()` regex and dedup
**Model:** sonnet

- Replace `r"(\$[1-9])"` with a regex that matches both bare and braced forms:
  ```
  \$(?:\{#?([1-9])[^}]*\}|([1-9]))
  ```
  - Group 1 `([1-9])` inside `\{...\}` - braced: optional `#` prefix (for length
    form `${#1}`), digit, then any non-`}` chars up to closing `}`
  - Group 2 `([1-9])` - bare digit
- Use a `BTreeMap<char, String>` keyed on digit; for each match, store the token
  only if it is longer than what is already stored for that digit (longest wins).
  `BTreeMap` provides ascending-digit order for free.
- Preserve the `$$N` exclusion: skip any match whose `start()` is preceded by `$`.
- Return `by_digit.into_values().collect()`.

#### Phase 2: Update `colorize_value()`
**Model:** sonnet

- Apply the same extended regex to `colorize_value()` in `src/lib.rs` so that
  braced forms are also highlighted in cyan. The full token (`${1:-.}`) is
  highlighted, not just the digit, making the expansion boundary visually clear.

#### Phase 3: Tests
**Model:** sonnet

Add to `src/cfg/alias.rs` tests:

- `${1}` detected as one positional.
- `${1:-.}` detected as one positional.
- `${1:-default}` detected as one positional.
- `${1:+foo}` detected as one positional.
- `${1:?msg}` detected as one positional.
- `${#1}` detected as one positional.
- Mixed: alias with both `$1` and `${1:-.}` counts as one positional.
- Replacement: `${1:-.}` is replaced with the supplied argument.
- `$$1` still excluded (not a positional).

## Alternatives Considered

### Alternative 1: Shell parser crate (yash-syntax)
- **Description:** Parse alias values as POSIX shell ASTs; walk the tree to find
  `ParameterExpansion` nodes.
- **Pros:** Correct for all possible bash forms by construction.
- **Cons:** GPL-3.0 license forces aka to be GPL. Many aka aliases use bash-specific
  syntax that a POSIX parser rejects. Adds a large dependency for a narrow use case.
- **Why not chosen:** License incompatibility and bash/POSIX divergence.

### Alternative 2: Extended regex (chosen)
- **Description:** Extend the existing regex to cover all `${N...}` forms.
- **Pros:** No new dependencies, no license issues, easy to audit, handles the full
  practical set of bash positional forms.
- **Cons:** A sufficiently exotic alias value could still slip through; regex can't
  handle nested braces (not a real-world concern for positional params).
- **Why chosen:** Solves the real problem with minimal surface area.

### Alternative 3: Prohibit bash expansion forms in aka aliases
- **Description:** Document that aliases must use bare `$N` and reject `${N...}`
  forms at parse time.
- **Pros:** Simple, no regex change needed.
- **Cons:** Unhelpful - the forms are valid shell; authors should not be surprised
  that they do not work.
- **Why not chosen:** Pushes the problem onto the user.

## Technical Considerations

### Dependencies

None added.

### Performance

`Regex::new` is called on every `positionals()` invocation today. This is unchanged.
The new pattern is slightly longer but compile cost is negligible for alias counts
in the hundreds.

### Security

No change to the attack surface.

### Testing Strategy

Unit tests in `src/cfg/alias.rs` (existing file). Each new bash form gets its own
test case asserting detection and, where relevant, correct substitution.

### Rollout Plan

Patch release. No config or daemon changes needed.

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Mixed-form alias (`$1` and `${1:-x}` for same positional) produces wrong substitution | Low | Medium | Dedup-by-digit picks one token; the other form remains literal. Document as unsupported. |
| Regex matches non-positional brace expansion (e.g. `${VAR}`) | Low | Low | Pattern requires a digit `[1-9]` immediately after `{` or `{#`, which excludes named variables |
| `$$` exclusion breaks with captures-based iteration | Low | Medium | The exclusion checks `match.start()` against the character before `$`; this is position-based and works regardless of capture groups. Add a regression test. |
| Nested brace default like `${1:-${2}}` confuses the regex | Very Low | Low | `[^}]*` stops at first `}`, producing a malformed match. Real aliases don't nest positionals in defaults; treat as unsupported. |
| `${10}` and above | Very Low | None | `[1-9]` is intentionally one digit; matches aka's existing `$1`-`$9` limit. |

## Open Questions

None.

## References

- `src/cfg/alias.rs` - `positionals()` implementation
- `src/lib.rs` - `colorize_value()` implementation
- Conversation that identified the bug: `cip` alias with `${1:-.}`
