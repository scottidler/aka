# Design Document: aka Robustness and Error Recovery

**Author:** Scott A. Idler
**Date:** 2026-04-11
**Status:** Implemented
**Review Passes Completed:** 5/5

## Summary

`aka` uses a ZLE-powered ZSH integration that spawns a subprocess on every space/enter keystroke. When the user's `~/.config/aka/aka.yml` contains a syntax error or validation failure, every keypress silently fails with no feedback - the user loses all alias expansion with no indication why. This design adds: last-valid config fallback using the existing JSON cache, user-visible error notifications, safe editing tools, a circuit breaker, and a suite of operational commands (`check`, `edit`, `restore`, `disable`, `enable`) to make the system self-healing and transparent.

## Problem Statement

### Background

`aka` embeds a ZSH init script that overrides the `space` and `accept-line` ZLE widgets. Each widget invocation calls `aka query "$BUFFER"` in a subshell with `2>/dev/null`. The binary loads `~/.config/aka/aka.yml` on every invocation (or consults the daemon). A separate JSON cache at `~/.local/share/aka/aka.json` stores all aliases plus a SHA256 of the last-successfully-parsed config.

### Problem

1. **Silent failure with no recovery path.** When YAML is broken, the health check returns status `2` (CONFIG_INVALID). The code discards the JSON cache (which contains all aliases from the last valid config) and returns a non-zero exit code. The shell receives empty output and silently falls back to literal key insertion. The user discovers the problem only by noticing aliases stopped working.

2. **No diagnostics.** There is no `aka check` command. The only way to see errors is to run `aka query something` and read stderr directly - which the shell integration always suppresses.

3. **No safe editing workflow.** Users edit `aka.yml` directly with no validation step. A single typo breaks everything.

4. **Kill-switch is discovery-dependent.** The `~/aka-killswitch` mechanism works but requires advance knowledge. There is no `aka disable` / `aka enable` pair.

5. **Cache JSON corruption is unhandled.** A corrupted `aka.json` (truncated write, manual edit error) propagates a parse error that kills alias lookup entirely.

6. **Daemon reload is blind.** `aka daemon --reload` applies a new config without first validating it, potentially leaving the daemon in a broken state.

### Goals

- G1: When config is broken, serve aliases from last-valid cache with a user notification
- G2: Provide `aka check` command that validates config and prints actionable errors
- G3: Provide `aka edit` safe editor (visudo-style: validate before applying)
- G4: Provide `aka restore` that restores `~/.local/share/aka/last/aka.yml` over the current config
- G5: Auto-backup config to `~/.local/share/aka/last/aka.yml` on every successful load
- G6: Provide `aka disable` / `aka enable` for ergonomic kill-switch management
- G7: Show a one-time ZLE message when config error is detected in the shell
- G8: Circuit breaker in ZSH: disable after N consecutive failures with a clear message
- G9: Shell startup health warning if config is already broken when shell opens
- G10: Make daemon auto-reload resilient (don't crash on broken config); surface reload errors to the user when running `aka daemon --reload`
- G11: Corrupt JSON cache falls back to empty rather than propagating error
- G12: Optional `$(aka prompt-status)` command for prompt integration

### Non-Goals

- Moving the kill-switch file from `~/aka-killswitch` to XDG (existing location stays; `aka disable`/`enable` just create/remove it)
- Automatic repair of broken YAML (we can report errors, not fix them)
- Supporting config formats other than YAML
- Breaking changes to the JSON cache schema (additions only)

## Proposed Solution

### Overview

The solution spans two layers:

**Rust layer** - New commands (`check`, `edit`, `restore`, `disable`, `enable`, `prompt-status`), a new health status code (`5` = CACHE_FALLBACK), last-valid backup on successful load, corrupt-cache resilience, and daemon reload validation.

**ZSH layer** - Per-session error flag with `zle -M` notification, circuit breaker after 5 consecutive failures, and startup health check emitted from the `shell-init` output.

### Architecture

#### New Health Status: CACHE_FALLBACK (5)

Current flow when config is broken:
```
health_check → 2 (CONFIG_INVALID)
  → route_command_by_health_status(2)
    → handle_command_direct_timed()
      → AKA::new() → Loader::load() → Err  ← dies here
```

New flow:
```
health_check → 2 (CONFIG_INVALID)
  → load_alias_cache() → has aliases? → yes
    → return 5 (CACHE_FALLBACK)
  → route_command_by_health_status(5)
    → handle_command_from_cache()  ← serves aliases from JSON cache
```

`handle_command_from_cache` works by calling `AKA::from_cache(cache, home_dir)` - a new constructor that builds an `AKA` from a pre-loaded `AliasCache` without touching the YAML file. Internally it reconstructs a `Spec` from the cache's `aliases` map (using `Spec { aliases: cache.aliases, lookups: HashMap::new(), defaults: Default::default() }`) and sets `config_hash` from `cache.hash`. The resulting `AKA` is then used identically to the direct-mode path. Lookups stored in `last/aka.yml` are not available in this fallback path since the cache only stores aliases - this is acceptable.

#### Last-Valid Backup Path

```
~/.local/share/aka/
  aka.json          (existing: alias cache with usage counts + config hash)
  last/
    aka.yml         (new: copy of config at last successful load)
```

The `last/` directory lives inside the same data directory returned by `get_alias_cache_path` (which respects `AKA_CACHE_DIR` for test isolation). It is created lazily on first successful config load. The backup is a plain file copy - no JSON wrapping, just the raw YAML. This means `aka restore` can literally `fs::copy` it back over `~/.config/aka/aka.yml`.

A new public function `get_last_valid_config_path(home_dir)` returns `<data_dir>/last/aka.yml` using the same `AKA_CACHE_DIR`-aware logic.

#### New Commands

| Command | Description |
|---------|-------------|
| `aka check` | Validate config, print errors, exit non-zero on failure |
| `aka edit` | Open $EDITOR on config, validate on save, abort if invalid |
| `aka restore` | Copy `last/aka.yml` over current config |
| `aka disable` | `touch ~/aka-killswitch` |
| `aka enable` | Remove `~/aka-killswitch` |
| `aka prompt-status` | Print `""` if healthy, `"⚠aka"` if broken |

#### ZSH Circuit Breaker

Two new `typeset -g` variables in the init script:
- `_AKA_FAIL_COUNT` - integer, increments on each failed `aka query` call; resets to 0 on success
- `_AKA_SESSION_DISABLED` - integer flag (0/1); set to 1 after 5 consecutive failures; never resets within session

Thresholds:
- At failure **1** (first after a working state or shell start): `zle -M "aka: config error - run 'aka check'"` (one-time notification)
- At failure **5**: set `_AKA_SESSION_DISABLED=1`, emit `zle -M "aka: disabled this session after 5 failures - run 'aka check' or restart shell"`
- While `_AKA_SESSION_DISABLED=1`: skip all `aka` calls immediately, fall through to normal shell behavior
- On any success: `_AKA_FAIL_COUNT=0` (but session disable is permanent - new shell resets it)

#### Shell Startup Warning

The `shell-init zsh` output includes a single health check at the bottom of the emitted script. The `command -v aka` guard is belt-and-suspenders since `aka` must be on PATH for `shell-init` to have run at all:
```zsh
# Run once at shell init - not on every keypress (~2ms cost)
if command -v aka >/dev/null 2>&1; then
    aka check --quiet 2>/dev/null || \
        echo "⚠️  aka: config error - run 'aka check' to diagnose" >&2
fi
```

### Data Model

#### AliasCache (extended)

No schema change needed. The existing `AliasCache` contains `hash: String` and `aliases: HashMap<String, Alias>`. The cache already carries all the data needed for fallback serving.

The "last valid" backup is a separate file (`last/aka.yml`), not embedded in the cache.

#### New Health Status Codes

```rust
// Existing
const HEALTHY: i32 = 0;
const CONFIG_NOT_FOUND: i32 = 1;
const CONFIG_INVALID: i32 = 2;
const NO_ALIASES: i32 = 3;
const STALE_SOCKET: i32 = 4;

// New
const CACHE_FALLBACK: i32 = 5;  // config broken, serving from last-valid cache
```

### API Design

#### `aka check [--quiet] [--json]`

```
$ aka check
Config: ~/.config/aka/aka.yml
Status: INVALID

Errors:
  line 14, col 3: mapping values are not allowed here
  alias 'g st' contains spaces in name - use hyphens or underscores

$ echo $?
1

$ aka check --quiet
$ echo $?    # 0 = valid, 1 = invalid
1

$ aka check --json
{"status":"invalid","path":"~/.config/aka/aka.yml","errors":[...]}
```

#### `aka edit`

```
$ aka edit
# Opens $VISUAL or $EDITOR on ~/.config/aka/aka.yml
# After editor exits:
Config invalid: line 14, col 3: mapping values are not allowed here
Re-edit? [Y/n]:
# If n: discard changes, original config unchanged
# If y: re-open editor on same temp file
```

#### `aka restore [--diff] [--force]`

```
$ aka restore
Restoring from: ~/.local/share/aka/last/aka.yml
--- current ---
+++ last-valid ---
  ... diff output ...
Restore? [y/N]: y
Restored. Run 'aka check' to verify.

$ aka restore --diff
# Shows diff only, no restore

$ aka restore --force
# No prompt
```

#### `aka disable` / `aka enable`

```
$ aka disable
aka disabled (created ~/aka-killswitch)

$ aka enable
aka enabled (removed ~/aka-killswitch)

$ aka disable
aka already disabled
```

#### `aka prompt-status`

```
$ aka prompt-status
   # empty string when healthy

$ aka prompt-status  # config broken
⚠aka
```

### Implementation Plan

#### Phase 1: Rust - Core Robustness
1. Cache corruption resilience (G11) - error recovery on JSON parse fail
2. Last-valid backup: directory + copy on successful load (G5)
3. CACHE_FALLBACK health status + `handle_command_from_cache` (G1)
4. `aka check` command (G2)

#### Phase 2: Rust - Operational Commands
5. `aka restore` command (G4)
6. `aka edit` safe editor (G3)
7. `aka disable` / `aka enable` (G6)
8. `aka prompt-status` (G12)

#### Phase 3: Rust - Daemon
9. Fix auto-reload crash risk (`?` at line 208 in aka-daemon.rs); display reload errors from `aka daemon --reload` (G10)

#### Phase 4: ZSH
10. ZSH error notification via `zle -M` (G7)
11. ZSH circuit breaker (G8)
12. Shell startup health warning in `shell-init` output (G9)

## Alternatives Considered

### Alternative 1: Embed last-valid aliases in aka.json

Store the last-valid spec directly in the JSON cache under a `last_valid_aliases` key, rather than keeping a separate `last/aka.yml` file.

- **Pros:** Single file, no second copy of data to manage.
- **Cons:** The JSON cache uses counts + hashes for performance; mixing recovery state into it couples two concerns. A separate file is simpler, human-readable, and lets `aka restore` work with `fs::copy` without JSON parsing.
- **Why not chosen:** Separation of concerns and simplicity of restore.

### Alternative 2: Git-based config versioning

Track `aka.yml` in a git repo and use `git stash` / `git checkout` for restore.

- **Pros:** Full history, diff support.
- **Cons:** Requires git in PATH, complex setup, overkill for this use case.
- **Why not chosen:** Too heavyweight; a single backup file solves 95% of the problem.

### Alternative 3: Move kill-switch to XDG data dir

Use `~/.local/share/aka/disabled` instead of `~/aka-killswitch`.

- **Pros:** All aka state in one place.
- **Cons:** Breaking change for anyone using the current path; shell init script hardcodes `~/aka-killswitch`; not worth the churn.
- **Why not chosen:** The existing path works; `aka disable`/`enable` just make it ergonomic.

## Technical Considerations

### Dependencies

No new external crates needed. The existing `serde_json`, `serde_yaml`, `eyre`, `fs`, `std::process::Command` cover everything. The diff for `aka restore --diff` uses `std::process::Command::new("diff")` (POSIX diff, always present).

### Performance

- Cache fallback path: `load_alias_cache` is already called in health check; no extra disk I/O.
- Last-valid backup: one `fs::copy` on successful load. Called only when config hash changes (not every query). Negligible.
- `aka check --quiet`: single YAML parse. Called once at shell startup. Acceptable.
- ZSH circuit breaker: pure shell variable checks; zero subprocess overhead.

### Security

- `aka edit` writes to a temp file in `/tmp` (not `$TMPDIR` which may be world-readable on shared systems). Use `tempfile` crate to create with mode `0600`.
- `aka restore` only copies from `~/.local/share/aka/last/aka.yml` (user-owned). No privilege escalation.

### Testing Strategy

- **Rust unit tests:** Each new function (`check_config`, `backup_config`, `handle_command_from_cache`, etc.) tested in isolation with `tempfile` fixtures.
- **Integration test:** Simulate broken config → `aka query` → verify fallback aliases are served → verify `aka check` exits non-zero with error text.
- **ZSH tests:** `tests/test_zsh_integration.sh` - add cases for circuit breaker flag behavior and `zle -M` emission (can test the logic via sourcing the script in a test shell).

### Rollout Plan

Single PR. No config schema changes. No breaking changes to existing `aka.json` format. Fully backward-compatible. The `last/` directory is created lazily - existing installs work without it.

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Cache fallback serves stale/removed aliases | Med | Low | Document in `zle -M` message: "using cached aliases from last valid config" |
| `aka edit` leaves temp file on crash | Low | Low | Use `tempfile` crate; OS cleans `/tmp` on reboot |
| `aka restore` overwrites a config that was intentionally changed | Low | Med | Default to interactive prompt; require `--force` for non-interactive |
| Diff binary not available on minimal systems | Low | Low | Fall back to "diff unavailable, restore anyway? [y/N]" |
| Shell startup check adds latency to new shells | Low | Low | `aka check --quiet` is a single YAML parse (~2ms) |
| `last/aka.yml` doesn't exist on first install | Med | Low | `aka restore` must check existence and print "no backup available yet" |
| Circuit breaker fires on daemon not running (transient error) | Med | Med | Only count failures where `rc != 0` AND `output` is empty; daemon timeout returns rc=1 which would incorrectly trip the breaker - filter by checking if `aka check --quiet` also fails before incrementing |
| `AKA::from_cache` reconstructed `Spec` missing `lookups` means lookup-using aliases expand incorrectly | Med | Med | Aliases that use `lookup:` syntax will expand to the literal `lookup:name[key]` string; this is acceptable fallback behavior - document it |
| `aka disable` run inside a shell that already has ZLE hooks loaded | Low | Low | Disable takes effect on **next** keypress (killswitch is checked per-keystroke); no reload needed |

## Open Questions

All resolved:

- `aka edit` editor precedence: `$VISUAL` -> `$EDITOR` -> `vi` (standard Unix precedence)
- `aka prompt-status` checks config validity only, not daemon health (avoids daemon dependency in prompt path)
- `aka restore --diff`: try `git diff --no-index --color=always`; fall back to plain `diff -u`
- Circuit breaker resets: session-only via `typeset -g`; each new shell starts fresh with counts at 0

## References

- [Alias Caching Investigation](../alias-caching-investigation.md)
- [Daemon Architecture](../daemon-architecture.md)
- [Performance Analysis](../performance-analysis.md)
- Current init script: `src/shell/init.zsh`
- Cache loader: `src/lib.rs:load_alias_cache()`
- Health check: `src/lib.rs:execute_health_check()`
- Config loader: `src/cfg/loader.rs`
