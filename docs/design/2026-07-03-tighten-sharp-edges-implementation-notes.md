# Implementation Notes: Tighten aka's Sharp Edges

Design doc: `docs/design/2026-07-03-tighten-sharp-edges.md`

## Phase 1: Remove the per-invocation daemon probe from `after_help`

### Design decisions
- Kept clap as the sole help resolver via `try_get_matches` + `from_arg_matches` — `parse_opts` (`src/bin/aka.rs`) replaces `AkaOpts::parse()`. On `ErrorKind::DisplayHelp`/`DisplayHelpOnMissingArgumentOrSubcommand` it prints clap's rendered help and, only for top-level help, appends the daemon-status line. Version and genuine parse errors fall through to `e.exit()` so clap keeps ownership of the stream and exit code. No `std::env::args()` scan (panel finding).
- Distinguished top-level help from subcommand help with `is_top_level_help` (`src/bin/aka.rs`), which checks for the `AFTER_HELP_LOG_MARKER` ("Logs are written to:") embedded in the static `after_help`. Subcommand help carries no `after_help`, so it never gets the probe-backed status line. This avoids re-rendering the command and works for both `-h` and `--help`.
- Made `after_help` static and probe-free — `get_after_help` (`src/bin/aka.rs`) now renders only the log path; the daemon-status emoji is appended lazily by `parse_opts` on the help path. Every non-help invocation (every `aka query`) pays zero probe cost.
- Single source of truth for the log path — added `log_file_path(home_dir)` (`src/lib.rs`) used by both `setup_logging` and `get_after_help`, so `--help` stays honest under `AKA_LOG_FILE` (and, post-Phase-4, XDG overrides).
- Bounded the status-emoji probe — `get_daemon_status_emoji` (`src/bin/aka.rs`) now sets 500ms read/write timeouts (`HELP_STATUS_PROBE_TIMEOUT_MS`) on the socket before writing; a wedged daemon can no longer hang `--help`. (Phase 2 replaces this whole probe with `probe_daemon_health`.)
- Bounded the daemon's own request read — `handle_client` (`src/bin/aka-daemon.rs`) sets 500ms read/write timeouts (`CLIENT_IO_TIMEOUT_MS`) on each accepted stream, so a client that connects and never sends a newline can't stall the serial accept loop.

### Deviations
- None. (The `probe_daemon_health` swap in `get_daemon_status_emoji` is deferred to Phase 2 as the doc specifies; this phase adds the interim timeouts as instructed.)

### Tradeoffs
- Top-level-help detection keys off the `after_help` marker string rather than a structural clap signal — chosen because clap's public API does not cleanly expose "is this top-level vs subcommand help" from the returned error. Per the doc's risk table, the accepted worst case for any missed help path is a static `after_help` without the emoji, so a marker miss degrades gracefully.
- Reused the existing 500ms timeout class for both the emoji probe and the daemon read, rather than introducing a new tuned value — keeps one hang-safety budget and matches `check_daemon_health`'s existing 500ms.

### Open questions
- The regression test (`tests/after_help_hang_test.rs`) runs `aka query foo` against a listener that accepts but never replies and asserts completion under 2s (child completes in ~ms). Note: the original unbounded-read hang in `get_daemon_status_emoji` is gated behind `check_daemon_process_simple` (a `pgrep aka-daemon` match), so reproducing the literal "hangs forever" in a test would require a live process named `aka-daemon`. The test therefore guards the spec's bounded-completion invariant on the hot path rather than the exact historical hang; strengthening it would mean spawning a real daemon-named process, which is fragile and out of scope. Confirm this is acceptable, or Phase 2 (which routes the emoji through the bounded `probe_daemon_health`) can carry a stronger direct probe test.

## Phase 2: Consolidate daemon clients and health probes

### Design decisions
- Deleted the ad-hoc client wholesale and made the binary a thin shim — `DaemonClient` (`src/bin/aka.rs`) now only resolves the socket path from `dirs::home_dir()` and delegates to `aka_lib::daemon_client::DaemonClient` (`LibDaemonClient::new().send_request(...)`), mapping `daemon_client::DaemonError` into eyre via `eyre::eyre!("{e}")`. The ad-hoc `DaemonError`, `should_retry_daemon_error`, `categorize_daemon_error`, `validate_socket_path`, `connect_with_timeout`, `attempt_single_request`, and the six `DAEMON_*` constants are gone. The shared client's retry policy already matches the required taxonomy exactly (retry only `ConnectionTimeout`/`ConnectionRefused`; `SocketNotFound` and read/write/protocol errors fail immediately), so the version-recycle fallback contract is preserved without new code.
- Preserved `send_request_timed`'s `TimingCollector` bracketing verbatim — `send_request_timed` (`src/bin/aka.rs`) still wraps `start_ipc()`/`end_ipc()` around the (now-delegated) `send_request` and maps its eyre error to `DaemonError::UnknownError(...)`, identical to the pre-consolidation behavior, so the four call sites in `handle_command_via_daemon_only_timed` are untouched.
- Ported the bounded connect loop into the canonical connector — `RealSocketConnector::connect` (`src/system.rs`) now runs the 100ms-capped loop (consts `CONNECT_TIMEOUT_MS`/`CONNECT_RETRY_SLEEP_MS`): retry only transient `WouldBlock`, surface `ConnectionRefused`/`NotFound` immediately, and return `TimedOut` on cap. This makes production runs use the same connect semantics the `MockSocketConnector` tests cover through `send_request`.
- Added the single health-probe implementation in the lib — `DaemonHealth` enum + `probe_daemon_health(&Path)` (`src/lib.rs`) route through `DaemonClient` and a strict `parse_health_status` (`healthy:<u32>:synced|stale`, everything else `Unhealthy`). Transport failures map to `Unreachable`; a reachable-but-malformed frame (`ProtocolError`) maps to `Unhealthy`. `check_daemon_health` (`src/lib.rs`) and `get_daemon_status_emoji` (`src/bin/aka.rs`) are now thin callers; `check_daemon_health` returns plain `bool` (no more `Result`), so `execute_health_check` dropped its `?`.
- Moved/deleted the duplicated client test suite — the ad-hoc `DaemonError` display/`should_retry`/`categorize`/`validate_socket_path` tests were removed from `aka.rs`'s `mod tests` (a breadcrumb comment marks the spot); the identical coverage already lives in `src/daemon-client.rs`'s `mod tests`. Ported the one real-fs case (regular file is not a socket) as `test_daemon_client_validate_socket_path_regular_file_not_socket` (`src/daemon-client.rs`) against `RealSocketConnector`.
- New direct-probe integration suite — `tests/health_probe_test.rs` stands up real Unix listeners to prove `probe_daemon_health`: happy synced/stale, strict-parse rejection of the legacy `healthy:N:aliases` shape (`-> Unhealthy`), bounded completion (`<2s`) + `Unreachable` against a never-responding listener, and `Unreachable` against an absent socket. This is the stronger probe test the Phase 1 handoff asked for.
- Updated the four stale health-shape fixtures to `healthy:5:synced` (`src/bin/aka.rs`, `src/bin/aka-daemon.rs`, `tests/daemon_integration_tests.rs`, `tests/protocol_consistency_test.rs`) per the doc's Testing Strategy.

### Deviations
- None.

### Tradeoffs
- `DaemonClientConfig::connection_timeout_ms` is now effectively decorative: the connect cap lives as a const inside `RealSocketConnector::connect` (per the doc's "canonical implementation" instruction) rather than being threaded from config. This already matched the pre-existing lib client, which never consumed that field; keeping it avoids churning the config struct and its `test_daemon_client_config_default` assertion. A follow-up could wire it through or drop it.
- The thin `DaemonClient::send_request_timed` double-maps the error (lib `DaemonError` -> eyre -> `DaemonError::UnknownError`), collapsing the variant. This is behavior-identical to the pre-consolidation shim and the call sites only `Display` the error, so it was preserved rather than refactored to carry the typed variant through — doing so would change the shim's public error type and ripple into the four call sites for no observable gain.

### Open questions
- Two remaining serialization smoke-test fixtures still carry legacy-shaped Health strings that the doc did not enumerate: `tests/file_watching_tests.rs:388` (`healthy:10:aliases`) and `src/protocol.rs:182` (`healthy:5:aliases:abc123:synced`). They use local test-only response types and never feed the strict parser, so they pass regardless of shape; left as-is to stay within the enumerated Phase 2 scope. Fold them into a later cleanup if a single canonical Health shape across all fixtures is wanted.

## Phase 3: Delete dead rituals

### Design decisions
- Deleted `store_hash` entirely (`src/lib.rs`) rather than leaving a stub — it was a pure no-op (`debug!` + `Ok(())`) with no remaining callers once the four production call sites were dropped, so nothing was left to keep testable.
- Removed its four call sites: the initial-hash store in `DaemonServer::new` (`src/bin/aka-daemon.rs`), the manual `reload_config` path, the `handle_config_file_change` watcher callback (both `src/bin/aka-daemon.rs`), and the config-validation path in `src/lib.rs` — each dropped along with its dead `if let Err(e) = ... { warn!/debug! }` wrapper and the now-stale "Store hash for CLI comparison" comment.
- Renamed `validate_fresh_config_and_store_hash` to `validate_fresh_config` (`src/lib.rs`) and dropped its now-unused `current_hash: &str` parameter — the parameter existed solely to feed the deleted `store_hash` call; nothing else in the function body referenced it. Updated the single call site in `execute_health_check` and the one direct unit test that called it by name.
- Removed the no-op count-initialization loop in `Loader::load` (`src/cfg/loader.rs`); `spec` no longer needs `mut` since nothing mutates it after deserialization, so dropped that binding's `mut` too (caught by `cargo fmt`/clippy, not a separate judgment call).
- Deleted the two `store_hash`-no-op tests (`test_store_hash_no_op`, `test_store_hash_no_op_returns_ok`, both `src/lib.rs`) outright rather than repointing them — they existed only to assert the deleted function's no-op `Ok(())` return, and there is no remaining behavior to assert once the function is gone.

### Deviations
- The doc names line numbers (`lib.rs:173`, `lib.rs:271`, `aka-daemon.rs:72,172,579`) that had drifted after Phases 1-2 landed; all four call sites and the definition were located by symbol name (`store_hash`, `validate_fresh_config_and_store_hash`) per the task's guidance, not by the stale line numbers. No behavioral deviation from the doc's intent.

### Tradeoffs
- None — this phase is pure deletion with no design choice beyond what the doc specifies.

### Open questions
- None.

## Phase 4: Finish the XDG path migration

> Note (orchestrator): the phase-implementer agent completed all Phase 4 code, tests, and README edits and left the tree with `otto ci` green, but terminated before committing and before appending this notes section. The orchestrator validated `otto ci` (exit 0, "✅ All CI checks passed!") on the uncommitted tree, then authored this section from the actual diff and committed the phase. Buckets below are reconstructed from the committed diff, not self-reported by the implementer.

### Design decisions
- Added `xdg_config_dir_from(home_dir)` / `xdg_data_dir_from(home_dir)` (`src/lib.rs`): env override first, absolute-only (a relative `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME` is rejected and falls through), then the injected `home/.config` and `home/.local/share` fallback. The zero-arg `xdg_config_dir`/`xdg_data_dir` are reimplemented on top via `dirs::home_dir()`, preserving the test seam.
- Routed all five resolvers through the helpers: `get_config_path` probes `xdg_config_dir_from(home)/aka/` then `home/` for the config filenames (documented home-dir fallback preserved); `setup_logging` via `log_file_path` (from Phase 1) now resolves under `xdg_data_dir_from`; `determine_socket_path` keeps `$XDG_RUNTIME_DIR` first (systemd sets it, so both halves agree), then `xdg_data_dir_from(home)/aka/daemon.sock`; `get_alias_cache_path` and `get_last_valid_config_path` likewise.
- Preserved `AKA_CACHE_DIR` and `AKA_LOG_FILE` as explicit overrides that beat XDG (`src/lib.rs` cache/log resolution).
- Collapsed `AKA_TEST_CACHE_DIR` into `AKA_CACHE_DIR`: the former is fully removed from `src/` and `tests/` (repo-wide grep returns zero hits post-change).
- Systemd/shell split-brain fix: added `xdg_environment_lines()` (`src/bin/aka.rs`) which snapshots any set `$XDG_CONFIG_HOME`/`$XDG_DATA_HOME` (absolute-only, matching the resolver guard) into `Environment=` lines baked into the generated unit at `--install`/`--reinstall`. `aka daemon --status` prints the CLI-resolved cache and log paths and warns to run `aka daemon --reinstall` after changing XDG env; the README documents the reinstall-after-env-change requirement.
- Tests use an `ENV_LOCK: Mutex<()>` with an `XdgEnvGuard` RAII guard (`src/lib.rs`, mirrored in `src/bin/aka.rs`) that snapshots and clears the path-resolution env vars under the lock so env-honoring and fallback cases are assertable without ambient `$XDG_*` leaking across parallel tests.

### Deviations
- Doc line numbers had drifted after Phases 1-3; all targets were located by symbol name per the task guidance. No behavioral deviation from the doc's intent.

### Tradeoffs
- `Environment=` snapshot is taken once at install/reinstall time rather than auto-detected on env change — matches the doc's explicit decision (documented `--reinstall`, not runtime drift detection); simpler and avoids a wire-protocol field.

### Open questions
- Doc Open Question (Phase 4): whether anything in Scott's dotfiles/scripts points at `AKA_TEST_CACHE_DIR`. The in-repo grep confirms tests-only (now migrated to `AKA_CACHE_DIR`), but the external dotfiles/scripts were not checked from here — surfaced to the user at finalization.

## Phase 5: Debounce daemon-side cache writes

### Design decisions
- Skipped the per-query cache write in daemon mode inside `replace_with_mode` (`src/lib.rs`), keyed off `ProcessingMode::Daemon`; Direct mode still writes on use, since a direct-mode process is short-lived and write-on-use IS its per-command persistence. In-memory counts still increment in both modes.
- Bundled the debounce state into a `FlushHandle` (`src/bin/aka-daemon.rs`): `cache_dirty: AtomicBool`, `failures: AtomicU32`, `timing: Mutex<FlushTiming>` (with `last_flush` and `last_successful_flush`). `DaemonServer` owns one; the watcher thread gets a `clone()` (the Arcs are shared, not duplicated). Consts `CACHE_FLUSH_INTERVAL_SECS = 5` and `FLUSH_DEGRADED_THRESHOLD = 5`.
- Single flush owner: `DaemonServer::flush_counts(aka, &FlushHandle)` is the ONLY daemon code that writes the cache. `flush_counts_if_dirty(&self)` is the convenience wrapper for the server-owned paths; the watcher tick calls `flush_counts` gated by `flush_due`.
- Lock order and dirty protocol in `flush_counts`: atomically claim the dirty state with `cache_dirty.swap(false)`, reset the cadence timer, take the AKA read lock, snapshot `{hash, aliases}` + `home_dir`, DROP the lock, then `save_alias_cache` (atomic tmp+rename). On success reset `failures` and record `last_successful_flush`; on failure re-arm `cache_dirty`, bump `failures`, and `warn!`. Never propagates, never stops serving (advisory Data Model).
- `cache_dirty` is set in the daemon Query handler (`src/bin/aka-daemon.rs`) after a default-config Daemon-mode query returns a non-empty transformation.
- Flush-before-reconstruction wired at every `AKA::new` rebuild path: manual `reload_config`, the watcher's `handle_config_file_change` (flush before the hash/reload work), `check_version_compatibility` (flush before signalling shutdown so the restarted daemon reloads a current cache), and `run()`'s post-loop cleanup on the main thread (the signal handler only sets the flag and removes the socket).
- Pre-`Freq` flush at the top of the `Freq` arm protects the cache FILE for a later direct-mode fallback; daemon-served freq still reads counts from memory.
- Timed flush piggybacks on the watcher loop's 100ms `recv_timeout` tick: on `Timeout`, flush when `cache_dirty` and `flush_due` (>= `CACHE_FLUSH_INTERVAL_SECS` since the last attempt). The loop now matches `RecvTimeoutError` explicitly and breaks on `Disconnected`.
- Health suffix: `process_health_request` appends `:degraded-persistence` once `failures >= FLUSH_DEGRADED_THRESHOLD`. `parse_health_status` (`src/lib.rs`) tolerates the optional trailing marker and still returns `Synced`/`Stale` (routing unaffected; the legacy `:aliases` shape stays `Unhealthy`). `health_status_degraded` + `DaemonHealthReport` + `probe_daemon_health_report` expose the degraded bit without touching `DaemonHealth`; `probe_daemon_health` now delegates. `aka daemon --status` probes the report and prints a degraded-persistence warning line.

### Deviations
- Dirty-flag clear ordering: the doc says "clear only after a successful rename," but clearing after the rename loses a count when a query fires between the snapshot and the clear. Implemented as an atomic read-and-clear (`swap(false)`) up front, re-arming on write failure. This delivers the doc's stated guarantees (never a lost count; failed flush keeps the flag and retries) at the correct seam ("same effect, correct seam").
- `cache_dirty` is set on any non-empty daemon-mode transform, a superset of count-bumping cases (a sudo-only transform with no alias would also set it). Worst case is one redundant flush, never a lost count; chosen over changing `replace_with_mode`'s return type to signal "counts changed," which would ripple through many callers, timing, and tests.
- Retry cadence on flush failure is the 5s flush interval (`last_flush` is reset on every attempt), not the raw 100ms tick, to avoid `warn!` spam on a persistently failing disk. The doc's "retry on the next tick" is honored in spirit (dirty flag kept and retried).
- Added `probe_daemon_health_report` + `DaemonHealthReport` rather than adding a field to `DaemonHealth::Synced`/`Stale`; Phase 2's tests match those variants by exact fields, so a new field would break them.
- The two live-daemon tests landed in `tests/freq_integration_test.rs` (spawns a real `aka-daemon` against a temp HOME) plus a lib-level no-write test in `tests/usage_tracking_tests.rs`. The shutdown-flush case is exercised via the version-mismatch path (deterministic; flushes synchronously inside the handler) rather than SIGTERM/SIGKILL, since the single-threaded blocking accept loop makes signal-driven flush timing nondeterministic to test.
- Bundled the three flush Arcs into `FlushHandle` to keep the watcher-loop signature under clippy's `too-many-arguments` threshold, rather than adding a repo-wide `clippy.toml` (none exists); this also improves cohesion.

### Tradeoffs
- Kept the debounce entirely in the daemon binary; `AKA`/`replace_with_mode` stays a pure "return data" seam that simply skips the write in daemon mode, so the equivalence tests (which compare output, not disk-write timing) remain valid.
- `FlushHandle` bundling vs a wider clippy config change: chose the self-contained struct.

### Open questions
- Signal-driven shutdown (SIGTERM/SIGINT) flush is best-effort: the `ctrlc` handler only sets the flag and removes the socket, and the `run()` cleanup flush runs on the main thread only once the blocking accept loop unblocks (next connection). Clean `Shutdown` requests and version-mismatch restarts flush deterministically; a SIGKILL loses up to one flush window (accepted per the advisory Data Model). Confirm this matches expectations, or a self-pipe / `accept` timeout could make signal shutdown flush promptly later (out of Phase 5 scope; also brushes the multithreading non-goal).

### Post-audit corrections (implementation-audit panel, 2026-07-03)
- **Supersedes Deviation #2 above.** The claim that gating `cache_dirty` on `!result.is_empty()` was a "superset of count-bumping cases ... never a lost count" is INVERTED and false: an alias whose value is `$@` with `space:false` invoked with zero args increments `alias.count` (`process_alias_replacement`, `src/lib.rs`) while `replace_with_mode` renders an empty string, so that bump never armed the dirty flag. Fixed in the follow-up commit by arming `cache_dirty` on every `Ok(_)` daemon query (`src/bin/aka-daemon.rs`); the debounce still bounds writes to one per `CACHE_FLUSH_INTERVAL_SECS`, and a flush that writes unchanged counts is a harmless redundant flush. This now genuinely delivers "never a lost count." (Panel finding #1, CONFIRMED.)
- **Phase 4 `Environment=` quoting.** `xdg_environment_lines` (`src/bin/aka.rs`) now emits `Environment="KEY=VAL"` so a snapshotted XDG value containing whitespace round-trips through the systemd unit instead of being split (systemd splits unquoted `Environment=` values on whitespace). Negligible in practice (whitespace in these paths violates the no-spaces convention) but correct. (Panel finding #4, CONFIRMED-negligible.)
- **Doc hygiene.** The Risks-table SIGKILL row now states signal-driven flush is best-effort rather than over-claiming "flush-on-shutdown covers SIGTERM/SIGINT" (finding #2), and the two stale Open-Question checkboxes in the design doc were resolved and ticked (finding #3).
