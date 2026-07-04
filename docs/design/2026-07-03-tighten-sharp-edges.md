# Design Document: Tighten aka's Sharp Edges

**Author:** Scott A. Idler (drafted by Claude from the 2026-07-03 deep-dive review)
**Date:** 2026-07-03
**Status:** Implemented
**Review Passes Completed:** 5/5 + cross-model panel (Architect/Gemini + Staff Engineer/Codex, 2026-07-03); panel findings incorporated below

## Summary

A full-source review of aka v0.6.13 found the architecture sound (single expansion engine, layered failure recovery, version-based daemon self-restart) but surfaced one real hot-path defect and a cluster of consolidation debt. This doc covers five fixes: remove the per-invocation daemon probe hidden in clap's `after_help` (a hang risk on every keystroke), consolidate the three parallel daemon-client/health-probe implementations onto the one testable client, delete the `store_hash` no-op ritual, finish the XDG path migration that commit `41cb1f2` started, and reduce cache write amplification on the query path.

## Problem Statement

### Background

aka rewrites the zsh command line live inside ZLE: every Space and Enter shells out to `aka query "$BUFFER"`. The entire design contract is that this path is fast and can never wedge the shell - hence the daemon's 300ms total IPC budget, the retry policy, the circuit breaker, and the killswitch. The 2026-07-03 deep-dive review (published at `marquee.internal.tatari.dev/p/~scott-idler/aka-deep-dive-inner-workings`) walked the full source and found the core engine and recovery ladder in good shape, but flagged several places where the implementation undercuts its own contract or maintains the same logic in multiple places.

### Problem

Five concrete issues, in severity order:

1. **Every `aka` invocation probes the daemon just to build `--help` text.**
   `src/bin/aka.rs:367` declares `#[command(after_help = get_after_help())]`. Clap derive evaluates that expression inside the generated `command()` builder, which `AkaOpts::parse()` runs on **every invocation** - including every Space-key `aka query`. `get_daemon_status_emoji()` (`src/bin/aka.rs:305`) then:
   - spawns a `pgrep aka-daemon` subprocess (`check_daemon_process_simple`, line 351), and
   - if socket + process exist, opens a `UnixStream`, sends a Health request, and calls `read_line` **with no read timeout set** (lines 325-341).

   Cost case: several ms of subprocess spawn + IPC added to every keystroke expansion, paid before the real query even starts. Failure case: a wedged daemon (accepts connections, never replies) hangs `read_line` forever - and because the ZLE widget runs `output=$(aka query ...)` synchronously, the **shell hangs**. The circuit breaker never fires because it counts non-zero exits, and a hung process never exits. This defeats the one guarantee the whole 300ms-budget client machinery exists to provide.

2. **Three parallel daemon-client/health implementations.**
   - `src/bin/aka.rs:31-295`: ad-hoc `DaemonError` + `DaemonClient` with hand-rolled connect/retry/timeout logic and its own constants.
   - `src/daemon-client.rs`: the DI-friendly `DaemonClient<C: SocketConnector>` with identical constants, identical error enum, identical retry policy - fully unit-tested against `MockSocketConnector`, **used only by tests**.
   - Health probing exists three more times: `check_daemon_health` in `src/lib.rs:180` (raw socket, 500ms timeouts), `get_daemon_status_emoji` in `aka.rs:305` (raw socket, no timeouts), and the `DaemonRequest::Health` path through either client.

   Every timeout or protocol change must be made 2-3 times; the copies have already drifted (the emoji probe forgot its timeouts - that is issue 1's hang).

3. **`store_hash` is a documented no-op still called at four production sites.**
   `src/lib.rs:173-178` logs a debug line and returns `Ok(())` ("Hash is now stored in the cache file itself"). It is still ritually invoked in `validate_fresh_config_and_store_hash` (`lib.rs:271`) and three times in the daemon (`aka-daemon.rs:72,172,579`), each wrapped in dead error handling. Same class: `cfg/loader.rs:49-54` contains a literal no-op loop (`if alias.count == 0 { alias.count = 0; }`).

4. **The XDG path migration is half-finished.**
   Commit `41cb1f2` added `xdg_config_dir()`/`xdg_data_dir()` (which honor `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME` on every platform) and migrated the systemd unit dir, `timing.rs`, and `get_alias_cache_path_with_base`. Still hand-building `~/.config` / `~/.local/share` from a `home_dir` argument, ignoring the env overrides:
   - `get_config_path` (`lib.rs:77`)
   - `setup_logging` (`lib.rs:137`)
   - `determine_socket_path` fallback (`lib.rs:1081`)
   - `get_alias_cache_path` (`lib.rs:1090`)
   - `get_last_valid_config_path` (`lib.rs:1101`)

   Consequence: with `$XDG_DATA_HOME` set, the daemon binary and CLI resolve the *cache and logs* differently from `timing.rs` and the service files - two halves of the program disagree about where state lives. There are also three overlapping test-override env vars (`AKA_CACHE_DIR`, `AKA_TEST_CACHE_DIR`, `AKA_LOG_FILE`) doing what the XDG vars already do. Related honesty bug: `after_help` hardcodes `Logs are written to: ~/.local/share/aka/logs/aka.log` (`aka.rs:300`), which is wrong whenever `AKA_LOG_FILE` or (post-fix) `$XDG_DATA_HOME` is set.

5. **Every expanding command rewrites the whole cache file.**
   `replace_with_mode` (`lib.rs:864-871`) serializes and atomically rewrites `aka.json` (full alias map, pretty-printed) every time any alias fires, in both direct and daemon modes. The data being persisted is advisory usage counts for `aka freq`. On the daemon path this is a per-request disk write serialized under the `AKA` write lock.

### Goals

- No code path in the CLI (or the daemon's request handling) can block without a timeout. Note: `aka query` still performs the routing health probe (`handle_regular_command` -> `execute_health_check`, `aka.rs:1395` -> `lib.rs:310`) *by design* - that probe is bounded (500ms today; standard client budget after Phase 2). This work removes the accidental *second* probe hidden in `after_help`, and the unbounded reads; it does not eliminate the health-check round trip, which is the router.
- One daemon client implementation, one health-probe implementation, both unit-testable.
- No dead rituals: `store_hash` and the loader no-op are gone.
- Every config/data/log/socket path in the program resolves through the two XDG helpers; `--help` reports the real log path.
- Usage-count persistence is amortized off the per-query hot path in the daemon.
- `otto ci` green throughout; daemon/direct equivalence tests still pass.

### Non-Goals

- **Decomposing `lib.rs` (4166 lines) / `aka.rs` (2233 lines) or extracting inline `mod tests` blocks into `tests.rs` files.** Real drift from the repo conventions, but it is a tree-wide mechanical pass that must not be mixed into behavior fixes (per `dealing-with-large-files.md`). Separate effort.
- **Multithreading the daemon accept loop.** Single-threaded is adequate for a per-user tool; requests are sub-ms.
- **Changing the `sudo -n which` probes** in the sudo-wrapping path. They are live subprocess calls, but only on `!`-triggered lines where the user is already invoking sudo; the behavior (detecting user-only binaries) requires the probe.
- Adding bash/fish shell support, changing the IPC protocol, or any alias-semantics change.

## Proposed Solution

### Overview

Five independent fixes, ordered so the highest-risk defect lands first and each phase leaves the tree shippable. No protocol changes, no config format changes; users see identical behavior except that `--help` may render its status emoji slightly differently and keystroke latency loses a constant few ms.

### Architecture

The consolidation target already exists: `src/daemon-client.rs` (`DaemonClient<C: SocketConnector>` over the `src/system.rs` trait layer). After this work:

```
src/bin/aka.rs          -- thin: parse, route, print; uses aka_lib::daemon_client
src/daemon-client.rs    -- THE client: timeouts, retries, error taxonomy
src/lib.rs              -- health checks route through daemon_client::DaemonClient
                        -- all paths route through xdg_config_dir()/xdg_data_dir()
src/bin/aka-daemon.rs   -- count-flush timer; no store_hash calls
```

### Data Model

Unchanged on disk. `aka.json` keeps its `{hash, aliases}` shape; the only change is *when* the daemon writes it (debounced instead of per-query).

**Decision (panel's converged "hardest question"): usage counts are advisory telemetry, not part of the daemon's correctness contract.** They feed `aka freq` sorting and nothing else. Bounded loss (up to one flush interval on SIGKILL, or a lost race with a direct-mode write during a daemon deadlock) is explicitly acceptable. This decision is what licenses Phase 5's timer design; if counts ever become correctness-visible, Phase 5 must be redesigned as a real persistence protocol.

### API Design

New/changed internal signatures (no CLI surface change):

```rust
// lib.rs - home_dir-parameterized XDG resolution, preserving the test seam:
// env override first (absolute paths only), then the injected home fallback.
pub fn xdg_config_dir_from(home_dir: &Path) -> PathBuf;  // $XDG_CONFIG_HOME || home/.config
pub fn xdg_data_dir_from(home_dir: &Path) -> PathBuf;    // $XDG_DATA_HOME  || home/.local/share

// lib.rs - single health probe, replacing check_daemon_health and the raw
// socket code in get_daemon_status_emoji. Uses DaemonClient (all timeouts apply).
pub enum DaemonHealth { Synced { aliases: u32 }, Stale { aliases: u32 }, Unhealthy, Unreachable }
pub fn probe_daemon_health(socket_path: &Path) -> DaemonHealth;
```

`aka daemon --status` keeps `pgrep` (diagnosing a socketless process is its job); the hot path never calls it.

### Implementation Plan

#### Phase 1: Remove the per-invocation daemon probe from `after_help`
**Model:** opus

- Gate help-text construction on help actually being requested - via clap itself, not arg-scanning (panel finding: a manual `std::env::args()` scan misses `aka help`, subcommand help, and future clap help spellings, and creates a dual source of truth). Mechanism: build the command with a *static* `after_help` (log path only), use `try_get_matches`; on `ErrorKind::DisplayHelp` / the `arg_required_else_help` case, rebuild with the dynamic status line appended and print that. Every non-help invocation pays zero probe cost; help resolution stays clap's.
- Add a read timeout to the daemon's own request read: `handle_client` (`aka-daemon.rs:269`) reads the request line with no timeout on a serial accept loop (`aka-daemon.rs:643`) - one client that connects and never sends a newline stalls the entire daemon. Set read/write timeouts on the accepted stream (same 500ms class). This is independent of the multithreading non-goal and closes the server half of the hang-safety theme.
- Replace the raw-socket probe inside `get_daemon_status_emoji` with `probe_daemon_health` (Phase 2 provides it; in this phase, minimally add `set_read_timeout`/`set_write_timeout` (500ms) to the existing probe so the hang risk dies immediately even if phases ship separately).
- Render the log path in `after_help` from the same resolver `setup_logging` uses (one source of truth), so it stays honest under `AKA_LOG_FILE` and XDG overrides.
- Regression test: an integration test that runs `aka query foo` with a socket file pointing at a listener that never responds, asserting the command completes under 2s (this currently hangs forever).

#### Phase 2: Consolidate daemon clients and health probes
**Model:** opus

- Delete the ad-hoc `DaemonError`, `DaemonClient`, `should_retry_daemon_error`, `categorize_daemon_error`, `validate_socket_path`, and `connect_with_timeout` from `src/bin/aka.rs` (lines 31-295); route all requests through `aka_lib::daemon_client::DaemonClient`, mapping `daemon_client::DaemonError` into the binary's eyre flow. Preserve `send_request_timed`'s `TimingCollector` bracketing as a thin wrapper.
- **Connect/retry contract (decided here, not discovered at runtime - panel finding):** the consolidated client must preserve the ad-hoc client's exact taxonomy, because the daemon self-restart mechanism depends on it. During the recycle window the socket file is briefly unlinked; the correct behavior is what `aka.rs:70` does today:
  - `SocketNotFound` -> fail immediately, **no retry** -> caller falls back to direct mode (this is what makes the version-recycle seamless);
  - `ConnectionRefused` / `ConnectionTimeout` -> 1 retry after 50ms;
  - read/write timeouts and protocol errors -> fail immediately.
  Port the bounded connect loop from `aka.rs:257` into `RealSocketConnector::connect` (100ms cap) as the canonical implementation, so `MockSocketConnector` tests cover the same semantics production runs.
- Implement `probe_daemon_health` in `lib.rs` on top of `DaemonClient` + a strict parse of `healthy:<count>:synced|stale`; rewrite `check_daemon_health` (`lib.rs:180`) and `get_daemon_status_emoji` as callers.
- Move the client-side test suite currently in `aka.rs`'s `mod tests` onto the lib client where duplicated; delete the copies.
- `daemon_direct_equivalence_test.rs` and `daemon_integration_tests.rs` must pass unchanged - they are the contract that consolidation broke nothing.

#### Phase 3: Delete dead rituals
**Model:** sonnet

- Remove `store_hash` (`lib.rs:173`) and its four call sites (`lib.rs:271`, `aka-daemon.rs:72,172,579`) including the dead `if let Err` handling; remove/repoint the two `store_hash` no-op tests.
- Remove the no-op count loop in `cfg/loader.rs:49-54`.
- Rename `validate_fresh_config_and_store_hash` to `validate_fresh_config` (its hash-storing half is already vestigial - the hash is persisted by `sync_cache_with_config_path` via the cache file).

#### Phase 4: Finish the XDG path migration
**Model:** opus

- Add `xdg_config_dir_from(home_dir)` / `xdg_data_dir_from(home_dir)` (env override first, absolute-only; injected home fallback) and reimplement the existing zero-arg helpers on top of them.
- Route through them: `get_config_path` (probe `xdg_config_dir_from(home)/aka/` then `home/` for the four filenames - preserving the documented home-dir fallback), `setup_logging`, `determine_socket_path` (XDG_RUNTIME_DIR first as today, then `xdg_data_dir_from(home)/aka/daemon.sock`), `get_alias_cache_path`, `get_last_valid_config_path`.
- Collapse `AKA_TEST_CACHE_DIR` into `AKA_CACHE_DIR` (grep says only tests use the former); keep `AKA_CACHE_DIR` and `AKA_LOG_FILE` as explicit overrides that beat XDG.
- **Systemd/shell env split-brain (panel must-fix):** `systemd --user` does NOT inherit `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME` exported in `.zshrc` - naively honoring the env in the CLI while the systemd-launched daemon starts with default env would make the two halves resolve *different* cache/log/socket paths, recreating the exact desync this phase exists to cure. Decisions:
  - `aka daemon --install` snapshots any set `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME` into `Environment=` lines in the generated unit (it already writes the unit; this is one more formatted field). `--reinstall` refreshes the snapshot.
  - The socket is largely immune already: `determine_socket_path` prefers `$XDG_RUNTIME_DIR`, which systemd *does* set for user sessions - both halves agree there. The `Environment=` snapshot covers cache/logs.
  - Drift detection: the existing per-request `ensure_cache_fresh` hash check plus `aka daemon --status` gain nothing new structurally, but `--status` should print the daemon's resolved cache/log paths (one debug field in the Health payload is out of scope; printing the CLI-resolved paths and noting "restart daemon after changing XDG env" in the README is in scope). Changing XDG env after install requires `aka daemon --reinstall` - documented, not auto-detected.
- Tests: per the repo's platform-path rule, add env-honoring + fallback tests behind a shared `ENV_LOCK: Mutex<()>` (env mutation is unsafe under parallel tests; existing tests that rely on temp `home_dir` injection must not be run with ambient `$XDG_*` leaking in - the `_from` variants make the fallback assertable by clearing the env under the lock).

#### Phase 5: Debounce daemon-side cache writes
**Model:** opus

- Keep direct-mode behavior as-is (a direct-mode process is short-lived; write-on-exit *is* per-command).
- Mechanism: the write currently happens unconditionally inside `replace_with_mode` (`lib.rs:864-871`), which already receives `ProcessingMode`. Key the skip off `ProcessingMode::Daemon` and surface a `cache_dirty` flag (e.g. `AtomicBool` on the server, set after each daemon-mode replacement) instead of writing. The equivalence tests compare *output*, not disk-write timing, so they stay valid.
- In the daemon: on query, update counts in memory only. Flush `aka.json` when dirty on a timer piggybacked on the existing file-watcher loop's 100ms tick (flush if dirty and >=5s since last flush - `CACHE_FLUSH_INTERVAL_SECS` const), plus an unconditional flush-if-dirty on shutdown (signal handler already owns cleanup) and before serving `Freq`. (Daemon-served `freq` reads counts from memory and is always fresh; the pre-`Freq` flush protects the *file*, which a later direct-mode fallback would read.)
- **Flush protocol (panel must-fix - ownership, lock order, failure policy):**
  - *Ownership:* `DaemonServer` owns a single `flush_counts_if_dirty()` helper; nothing else writes the cache from daemon code.
  - *Lock order:* take the `AKA` **read** lock, serialize the cache snapshot, drop the lock, then write the file (atomic tmp+rename as today). Clear the `cache_dirty` flag only after a successful rename; a query that fires mid-flush re-sets it (worst case: one redundant flush, never a lost count).
  - *Flush-before-reconstruction, everywhere:* `AKA::new()` re-reads the cache from disk (`lib.rs:532`), so every path that builds a new `AKA` must call `flush_counts_if_dirty()` first - manual `reload_config`, the watcher's `handle_config_file_change`, the version-mismatch shutdown, and the signal/shutdown path (the signal handler only sets the flag and removes the socket at `aka-daemon.rs:777`; the flush belongs in `run()`'s post-loop cleanup, which runs on the main thread).
  - *Failure policy:* counts are advisory (Data Model decision), so a failed flush is `warn!` + keep the dirty flag + retry on the next tick - never propagate, never stop serving. After N consecutive failures (const, e.g. 5) the Health response appends `:degraded-persistence` so `aka daemon --status` surfaces it.
  - *Observability:* debug-log each flush with alias-count and elapsed ms; track last-successful-flush timestamp and consecutive-failure count in the server (log-visible; not added to the wire protocol beyond the Health suffix above).
- Known accepted race (panel finding, licensed by the advisory decision): a deadlocked-but-alive daemon trips the shell circuit breaker into direct mode; direct mode reads a cache file missing up to 5s of buffered counts and rewrites it; a later daemon flush clobbers that write. Last-writer-wins, bounded loss, accepted - recorded in Risks.
- Tests: extend `usage_tracking_tests.rs` / `freq_integration_test.rs` to cover daemon-mode count persistence across reload and shutdown.

## Alternatives Considered

### Alternative 1: Lazy `OnceLock` for `after_help` instead of arg-scanning
- **Description:** Wrap the status string in `OnceLock` so it computes at most once per process.
- **Pros:** No arg-scanning; tiny diff.
- **Cons:** Clap still *calls* the expression on every parse - `OnceLock` computes it on every invocation anyway (each invocation is a fresh process). Does not fix anything.
- **Why not chosen:** Per-process memoization is worthless for a per-keystroke CLI; the probe must be gated on help being requested.

### Alternative 2: Keep the ad-hoc client, delete `daemon-client.rs`
- **Description:** Treat the binary's client as canonical and drop the DI version.
- **Pros:** Production code path untouched.
- **Cons:** Loses the only mockable, unit-tested client; keeps timeout/retry logic untestable without a live daemon; inverts the repo's DI convention (traits in `system.rs` exist precisely for this).
- **Why not chosen:** The tested implementation should be the shipped one, not the shadow.

### Alternative 3: Move counts to a separate append-only file (or SQLite)
- **Description:** Persist usage events instead of rewriting the alias map.
- **Pros:** No write amplification at all; richer history.
- **Cons:** New file format, new compaction problem, breaks the cache's second job (config-hash + fallback source) or forces a split; heavy for advisory counters.
- **Why not chosen:** Debouncing in the daemon gets ~all the benefit inside the existing format.

### Alternative 4: Fold the lib.rs/aka.rs decomposition into this work
- **Description:** Do the 1500-line-rule module split at the same time, since we are touching these files.
- **Pros:** One review cycle.
- **Cons:** Mixes a mechanical tree-wide move with behavior changes, making both diffs unreviewable and `git blame` useless across the fix.
- **Why not chosen:** Explicit repo rule: never mix decomposition into a feature/fix pass.

## Technical Considerations

### Dependencies
No new crates. Phase 5 uses the existing watcher thread's tick; no timer crate.

### Performance
- Phase 1 removes a `pgrep` fork + socket round trip from every invocation: measurable, constant win on the keystroke path (and removes an unbounded worst case).
- Phase 2 is behavior-neutral by design; the timing framework (`AKA_BENCHMARK`, `aka daemon --timing-summary`) is the before/after harness.
- Phase 5 turns N disk writes per N daemon queries into at most one per 5s window.

### Security
No new surface. Socket stays user-owned under `XDG_RUNTIME_DIR`; path changes only affect resolution order, and the env overrides accept absolute paths only (matching the existing helpers' guard against relative `$XDG_*`).

### Testing Strategy
- `otto ci` green after every phase; each phase is a separate conventional commit.
- In-tree tests help but are weaker than they look (panel finding): `daemon_direct_equivalence_test.rs` is a real behavioral contract; `protocol_consistency_test.rs` / `daemon_integration_tests.rs` are largely serialization smoke tests, and several fixtures still assert the *old* `healthy:5:aliases` health shape (`protocol_consistency_test.rs:97`, `daemon_integration_tests.rs:149`, `aka.rs:1915`, `aka-daemon.rs:873`) that `health_check_tests.rs:325` rejects. Phase 2 includes updating those stale fixtures to the `healthy:N:synced|stale` shape and adding a strict-parse test for `probe_daemon_health` - do not lean on the existing suites alone for consolidation safety.
- New tests called out per phase: the never-responding-daemon timeout test (Phase 1), env-locked XDG resolution tests (Phase 4), daemon count-flush tests (Phase 5).
- Manual verify per the repo's `/verify` flow: run the real shell integration (Space/Enter expansion, `aka freq`, config edit hot-reload, daemon kill mid-session) after Phases 1, 2, and 5.

### Rollout Plan
Ships as a normal patch release via `bump` + the existing 4-target release workflow; `cargo install --path . && systemctl --user restart aka-daemon` locally. The version-mismatch protocol recycles running daemons automatically on the first post-install query - no manual coordination needed beyond the systemd restart.

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Lib client's simpler connect (no poll loop) behaves differently under daemon restart races | Med | Med | Phase 2 explicitly tests the restart window; port the poll loop into `RealSocketConnector` if needed |
| XDG env overrides change where an existing install reads its cache/logs (user has `$XDG_DATA_HOME` set) | Low | Med | Behavior-neutral when unset (the common case); release notes call it out; `AKA_CACHE_DIR`/`AKA_LOG_FILE` still win |
| Debounced counts lost on daemon SIGKILL or a SIGTERM/SIGINT received while idle-blocked in `accept()` | Low | Low | Counts are advisory; clean `Shutdown` requests and version-mismatch restarts flush deterministically. Correction (2026-07-04 audit, `docs/design/2026-07-04-deferred-edges-audit-note.md`): a signal delivered while blocked in `accept()` does NOT merely defer the flush - unlinking the socket does not wake a blocked `accept()` and no further connection arrives, so the loop never returns and the post-loop flush never runs; systemd hard-kills at `TimeoutStopSec`. Buffered counts are lost (advisory, so acceptable), and signal-driven shutdown/restart is always a hard-kill. Cheap fix if it ever matters: self-connect / `shutdown(2)`, or a non-blocking accept polling `shutdown` - see implementation notes Phase 5 |
| Env-mutating XDG tests race under parallel `cargo test` | Med | Low | Shared `ENV_LOCK` mutex per the repo's platform-path test pattern |
| Help interception via `try_get_matches`/`DisplayHelp` misses an exotic help path | Low | Low | Worst case: static after_help without the emoji; `aka daemon --status` remains the full diagnostic |
| Systemd daemon and interactive CLI resolve different XDG paths (env not inherited) | Med | High | `--install`/`--reinstall` snapshot XDG env into `Environment=` unit lines; socket unaffected (`$XDG_RUNTIME_DIR` is systemd-set); README documents reinstall-after-env-change |
| Direct-mode fallback writes a stale cache during a daemon deadlock; daemon flush later clobbers it | Low | Low | Accepted: counts are advisory (Data Model decision); flush-before-reconstruction bounds the window to one flush interval |

## Open Questions

- [x] ~~Should the status emoji leave `--help` entirely (it duplicates `aka daemon --status`), making `after_help` fully static?~~ Resolved: kept. The `DisplayHelp`-interception mechanism (Phase 1) makes it cheap - the probe runs only on the help path, never on `aka query` - so the discoverable at-a-glance signal stays at zero hot-path cost.
- [x] ~~Phase 4: is anything in Scott's dotfiles/scripts pointing at `AKA_TEST_CACHE_DIR`?~~ Resolved: in-repo grep confirms tests-only (now migrated to `AKA_CACHE_DIR`); the panel's Codex reviewer verified no external references. Collapsed.
- [x] ~~How do CLI and systemd daemon stay XDG-synchronized?~~ Resolved by panel review: `Environment=` snapshot at install time + documented `--reinstall` on env change (Phase 4).
- [x] ~~Are usage counts advisory or correctness-visible?~~ Resolved: advisory telemetry, bounded loss accepted (Data Model).

## References

- Deep-dive review: https://marquee.internal.tatari.dev/p/~scott-idler/aka-deep-dive-inner-workings
- XDG helper introduction: commit `41cb1f2` (v0.6.13)
- Prior art in-repo: `docs/daemon-architecture.md`, `docs/version-based-daemon-restart.md`, `docs/design/2026-04-11-robustness-and-recovery.md`
- Canonical client to consolidate onto: `src/daemon-client.rs`; trait layer: `src/system.rs`
