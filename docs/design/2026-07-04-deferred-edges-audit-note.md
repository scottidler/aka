# Audit note: are the two deferred sharp edges deferred for the right reasons?

**Subject design doc:** `docs/design/2026-07-03-tighten-sharp-edges.md` (status: Implemented)
**Status of this note:** adjudicated by the review panel (Architect/Gemini + Staff
Engineer/Codex, 2026-07-04). Two of my original claims were overturned; this note has been
corrected to record the truth. See "Panel adjudication" below.

**Context:** a full-source-read deep-dive of `scottidler/aka` at `7b68a99` (v0.6.13) listed
five "sharp edges." The tighten-sharp-edges pass fixed three of them (duplicated daemon
client -> Phase 2; hand-built XDG paths -> Phase 4; write-per-command usage counts ->
Phase 5) and explicitly declared two as **Non-Goals**:

- **#1** `sudo -n which` subprocess probes in the sudo-wrapping path
  (`lib.rs is_command_available_to_root`, `is_user_only_command`, `is_user_installed_tool`).
  Non-goal text: *"They are live subprocess calls, but only on `!`-triggered lines where
  the user is already invoking sudo; the behavior (detecting user-only binaries) requires
  the probe."*
- **#5** Single-threaded daemon accept loop (`aka-daemon.rs handle_incoming_connections`).
  Non-goal text: *"Single-threaded is adequate for a per-user tool; requests are sub-ms."*

---

## Bottom line (post-adjudication)

Both edges remain **correct to defer** - no correctness or safety bug exists on either. But
the framing "pure latency on rare paths" was wrong on both counts:

- **#1 fires on the Space path**, not only on Enter/`!` lines. `sudo` is set purely from
  `args[0] == "sudo"` with no `self.eol` guard, so `sudo <cmd>` + Space spawns the probes.
  It is still cheap and rare-ish (once per submission, sudo lines only), so the deferral
  stands - but my central frequency argument rested on a false statement about which path
  it runs on.
- **#5 is a shutdown-operability edge, not latency.** A signal delivered while blocked in
  `accept()` never triggers the post-loop count flush (unlinking the socket does not wake a
  blocked `accept()`), so shutdown/restart always hard-kills at systemd `TimeoutStopSec`.
  This is disclosed and accepted in the parent doc (advisory counts), so it is not a defect -
  but "rationale complete" was too strong; I had missed the failure mode entirely.

## What we changed as a result

- **Code (the one genuinely free, safe win):** collapsed the redundant double-`which`.
  `resolve_user_command_path` now runs `which` once per command and feeds both
  `needs_sudo_wrapping` (via a precomputed `user_has`/`user_path`) and `is_user_installed_tool`
  (via the resolved path). Behavior-preserving; strictly fewer subprocess spawns in the
  common case. No other code changed - there were no bugs to fix.
- **Code comments:** the sudo-probe call site now documents that the probes run on Space
  (not just eol) and why the cost is accepted; the daemon accept loop and `ctrlc` handler
  now document the signal-in-`accept()` flush-skip and that it is accepted (advisory counts).
- **Parent doc:** `2026-07-03-tighten-sharp-edges.md` risk row corrected - the flush does not
  merely "defer until the next connection," it never runs on signal-driven shutdown; restart
  is always a hard-kill at `TimeoutStopSec`.

## Panel adjudication (per claim)

| Claim | Verdict | Note |
|---|---|---|
| Both correct to defer | upheld | Decision stands on both edges |
| "pure latency on rare paths" | **partially refuted** | Wrong on both: #1 runs on Space; #5 is a shutdown edge |
| #5 rationale is *complete* | **refuted (convergent)** | Missed the signal-in-`accept()` flush-skip |
| "requires the probe" misleading | divergent -> wording nit | My "amortizable via memoization" was *also* misleading (the memo is the unsafe part) |
| double-`which` redundant | **confirmed (convergent)** | Fixed; kept the shared single lookup feeding both checks |
| session memoization safe | **refuted (convergent)** | Dropped - unsafe without TTL/invalidation; it gates sudo wrapping |

### #5 - single-threaded daemon (corrected)

The serial blocking accept loop is fine for a per-user tool, and the one real hazard (a peer
that connects but never sends a line, wedging subsequent keystrokes) is bounded by the
Phase 1 read/write timeouts on the accepted stream. The failure mode I missed: a signal
(SIGINT/SIGTERM) delivered while blocked in `incoming()` sets `shutdown` and unlinks the
socket, but unlinking does not wake a blocked `accept()` and no further connection arrives,
so the loop never returns and the post-loop `flush_counts_if_dirty()` never runs. Systemd
hard-kills at `TimeoutStopSec`. Buffered counts are advisory, so the bounded loss is
acceptable (this is what the parent doc already licensed). Cheap fix if it ever matters:
self-connect / `shutdown(2)`, or a non-blocking accept polling `shutdown`. Deferral stands;
the *operability* cost (always-SIGKILL restart of the `systemctl --user` service) is the
real under-documented part, now corrected in the parent doc.

### #1 - `sudo -n which` probe (corrected)

The probes fire whenever `sudo` is the first token, on the Space path too - not only on
`!`-triggered eol lines. Still cheap and infrequent (once per submission, sudo lines only,
and the user is invoking sudo interactively anyway), so deferral is right. On the wording
dispute: "requires the probe" is loose (the behavior needs the *answer*, not a fresh
subprocess), but my proposed replacement - "amortizable via memoization" - is loose in the
opposite direction, because safe caching needs TTL / path-invalidation (a stale answer
mis-gates sudo wrapping). So the memoization recommendation is **dropped**. The only free,
safe win was collapsing the double-`which`, which is done.

### Bonus smell (Codex, not a bug)

The shipped client forces `--config` to direct mode, but the daemon protocol/server still
carry live custom-config handling that the client never routes to. Dead path, contradicts
its own comment. Noted, not actioned here.

---

## Original questions (now resolved)

1. #5 complete? **No** - missed the signal-in-`accept()` flush-skip (shutdown-operability,
   not latency). Deferral still correct; parent doc wording corrected.
2. "requires the probe" misleading? **Partly** - but so was "amortizable via memoization";
   demoted to a wording nit, memo recommendation dropped.
3. double-`which` redundant? **Yes** - collapsed into one shared `resolve_user_command_path`
   lookup feeding both checks.
4. session memoization safe? **No** - unsafe without TTL/invalidation; dropped.
