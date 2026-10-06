# Design Document: Skeleton Expansion (learned abbreviation engine)

**Author:** Scott A. Idler (with Claude)
**Date:** 2026-07-04
**Status:** In Review
**Review Passes Completed:** 5/5

## Summary

Add a learning layer to aka: type a compressed "skeleton" of a command (`gcm`),
and at Enter the engine expands it to the full command it abbreviates
(`git commit -m`), sourced from the user's own shell history and ranked by
frecency. Ambiguity is resolved via sk/fzf. Expansions accepted repeatedly are
promoted into a persisted learned-alias tier that behaves exactly like a
hand-written alias; unused entries atrophy via zoxide-style aging. The hard
safety rule: a guessed expansion never executes - only promoted (deterministic)
entries execute through.

## Problem Statement

### Background

aka today expands only aliases the user has hand-written into `aka.yml`. The
infrastructure around expansion is mature: a daemon holding parsed state hot,
usage counts with debounced persistence, hash-gated reload, ZLE Space/Enter
widgets, an sk/fzf picker (Ctrl-T), a killswitch and a circuit breaker. But the
alias set itself only grows when the user notices a repeated pattern and edits
config. The observation cost is the bottleneck: the user IS the pattern
detector.

### Problem

Frequently-typed command shapes (`git commit -m`, `cargo build --release`,
`kubectl get pods -n`) go un-aliased because nobody sits down to write the
alias. The user wants to type a character series - one character per word -
and have the engine figure out the command, confirm it, and eventually learn
it permanently.

Requirements as stated by Scott (this session, 2026-07-04):

1. Input is a series of single characters, delimited or not
2. First character resolves to a command; subsequent characters to
   subcommands / flags / positional-arg words
3. Multiple matches are ranked and presented via fzf
4. Expansions accepted several times are written to disk and hotpathed
5. Entries unused for a long time atrophy and fall off

Accepted reframe (Scott: "I like it", this session): candidates come from the
user's own shell history via word-initial skeleton matching, NOT from
enumerating PATH and inferring per-binary grammars. History-first collapses
the knowledge-acquisition problem (binaries don't publish their grammars) and
the ambiguity problem (candidates are only commands the user actually runs,
pre-ranked by real usage) into one index lookup.

### Goals

- Skeleton lookup: `gcm` at Enter -> `git commit -m` when history supports it
- Head-token semantics: only the skeleton token expands; trailing words are
  preserved (`gcm fix the bug` -> `git commit -m fix the bug`)
- Ranked candidates; sk/fzf picker on ambiguity
- Reinforcement: accepted expansions counted; after K accepts, promoted to a
  learned tier that behaves identically to a hand-written alias
- Atrophy: zoxide-style aging demotes and eventually removes unused entries
- Zero cost and zero behavior change on every line that doesn't trigger it

### Non-Goals

Excluded (not planned):

- Bash/fish support - aka's shell integration is zsh-only today; this feature
  rides the existing widgets
- Single-character skeletons (`g` alone) - ambiguity is unbounded and a
  1-char alias is cheap to hand-write
- Auto-writing anything into `aka.yml` - user config is user-owned, the
  engine never touches it

Parked (with revisit conditions, see Addendum):

- Spec-based candidate sources (carapace-bin, zsh completion capture) -
  revisit if Phase 0/1 shows history-only hit rate is insufficient
- `.` / `:` delimited skeletons - revisit if undelimited chords prove too
  short in practice
- Pipeline-crossing skeletons (`lg` -> `ls | grep`) - revisit after v1 lands
- Context-aware ranking (cwd type, command-chain Markov) - revisit after
  promotion mechanics prove out
- atuin as history source - not installed; revisit if adopted

## Proposed Solution

### Overview

A new `skel` module in `aka_lib` builds an index from `$HISTFILE`: for every
history entry, the word-initial skeleton of each command prefix maps to that
prefix as a candidate expansion, scored by frecency. The daemon holds the
index hot (rebuilt incrementally on history-file changes via the existing
notify watcher) and persists it debounced (reusing the Phase 5 flush pattern)
so direct mode can load it from disk. At Enter (eol only), when the head token
matches no alias, no learned entry, and no binary, the widget asks
`aka __skel` for candidates: one confident candidate rewrites the buffer
WITHOUT executing; multiple candidates go through sk/fzf; the user's next
Enter both executes and (via `aka __skel_accept`) reinforces. After
`promote-after` accepts, the skeleton becomes a learned alias - merged into
the in-memory alias map below user aliases - and from then on expands and
executes exactly like any alias. Aging decays scores; long-unused learned
entries demote back to candidates and eventually drop.

### The trust boundary

This is the load-bearing design rule:

| Tier | Source | On Enter | On Space |
|------|--------|----------|----------|
| User alias (`aka.yml`) | hand-written | expands + executes | expands |
| Learned alias (promoted) | engine, after K accepts | expands + executes | expands |
| Skeleton candidate | inference | rewrites buffer, does NOT execute | never fires |

Promotion is not just a performance hotpath - it is the point where a guess
becomes deterministic and earns execute-through. Below the line, the user
always sees the expansion before running it. A promoted skeleton is pinned:
inference never re-ranks it; changing it requires explicit demotion.

### Architecture

```
 ~/.zsh_history  --(notify watcher, incremental tail parse)--> daemon
                                                                 |
                                            skel index (in-memory, frecency-scored)
                                                                 |
                              debounced persist --> ~/.local/share/aka/skel.json
                                                                 |
 zsh accept-line widget                                          |
   1. aka --eol query "$BUFFER"      (existing path, unchanged)  |
   2. if no output AND the widget pre-gate passes:               |
      aka __skel "$BUFFER"  --(daemon Skel request; direct mode reads skel.json)
        0 candidates -> plain .accept-line
        1 candidate  -> rewrite BUFFER, reset-prompt, STOP (no execute)
        2+           -> sk/fzf picker -> rewrite BUFFER, STOP
   3. next Enter: normal flow executes; widget fires
      aka __skel_accept (reinforcement) if the pending expansion survived
```

The widget pre-gate is load-bearing for latency AND correctness: without it,
EVERY unaliased line at Enter (most lines) would spawn an extra `aka __skel`
subprocess. The widget checks, in pure zsh with zero subprocesses, that the
head token is 2..=6 ASCII alphanumerics AND is not resolvable in ANY shell
namespace: `whence -w -- "$head" >/dev/null` (a builtin, no fork) covers
PATH commands, shell builtins (`cd`, `jobs`, `exec`), shell functions, shell
aliases, AND reserved words (`if`, `for`) in one call - `$+commands` alone
misses everything but PATH binaries (review panel 2026-07-04, finding 2;
verified against zsh 5.9). Only lines surviving that spawn `__skel`, which
re-checks what it CAN see authoritatively (aka aliases, learned entries,
PATH, config-driven bounds); shell-local namespaces are invisible to the
engine, so the widget check is the authority for those, not an
optimization.

Components:

- `src/skel.rs` + `src/skel/` (Rust 2018 module style): `skel/history.rs`
  (zsh history parser), `skel/index.rs` (skeleton index + lookup),
  `skel/score.rs` (frecency + aging), `skel/store.rs` (learned tier +
  accept counters, JSON persistence)
- Daemon: holds `SkelIndex` behind the existing lock pattern; second notify
  watch on `$HISTFILE`; two new protocol variants
- CLI: hidden `__skel` / `__skel_accept`; user-facing `aka skel` with flags
  (`--list`, `--demote <skeleton>`) per house CLI conventions
- `init.zsh`: extended `_aka_accept_line`, new `_aka_skel_pick` (mirrors
  `_aka_search`'s sk/fzf pattern)

### Skeleton computation

For each history command line:

- Split on whitespace (quote-awareness not required for skeleton purposes;
  the chain stops at non-word tokens anyway)
- A word is "skeletonizable" if, after stripping leading `-`s, it starts
  with an ASCII alphanumeric; its skeleton char is that char, lowercased
  (`commit` -> `c`, `-m` -> `m`, `--release` -> `r`)
- The prefix chain extends word by word and STOPS at the first word that
  violates any of the layered SECRET-GUARD rules (review panel 2026-07-04,
  finding 1 - the original `=`-only rule leaked bare space-delimited
  values):
  1. Non-skeletonizable word: quoted strings, `$vars`, paths with `/`
  2. Any word containing `=` (assignments, `--flag=value`)
  3. Any shell operator (`|`, `&&`, `;`, `>`, `<`) - skeletons never cross
     pipelines
  4. FLAG-VALUE TERMINATION: a non-flag word following a flag word ends the
     chain - every flag is assumed to possibly take a value. `docker login
     -u user -p mysecret` chains only to `docker login -u`; `kubectl get
     pods -n prod` chains to `kubectl get pods -n` (skeleton `kgpn`, `prod`
     excluded); `curl -H Bearer...` chains to `curl -H`. Known gap: a flag
     VALUE that itself begins with `-` reads as another flag and continues
     the chain (`curl --user -Secret`) - accepted as a disclosed residual
     (see Security and the Addendum) rather than closed, because the
     schema-free alternative (terminate after EVERY flag) would gut
     legitimate multi-flag skeletons like `-d --build`
  5. ALPHA-ONLY CONTINUATION: beyond the head word, a chain word must match
     letters-and-dashes only (`^-*[A-Za-z][A-Za-z-]*$`) - kills tokens,
     digit-bearing secrets, IPs, ports (`vault write secret123` chains to
     `vault write`). The head is exempt (`python3`, `base64`). Hyphenated
     subcommands survive (`git cherry-pick` -> `gc`); `git checkout -b`
     -> `gcb` with `feature-x` excluded by rule 4
- For each prefix of length k in 2..=`max-chars` (default 6), emit
  `skeleton(prefix) -> join(prefix words)` as a candidate
- FREQUENCY FLOOR: a candidate is offered only once its occurrence count
  reaches `min-occurrences` (config, default 2) - a value that slipped the
  structural rules must also recur identically before it can ever appear in
  the picker. Phase 0 reports hit-rate at floors 1/2/3 so the default is
  evidence-based
- Per skeleton, only the top `MAX_CANDIDATES_PER_SKELETON` (named const,
  10) expansions by score are retained - bounds index memory and file size
  against the 500k-line HISTSIZE ceiling (nobody scrolls past 10 in a
  picker anyway); promote to config only if tuning ever proves necessary

Example: `git commit -m "fix the thing"` emits `gc -> git commit` and
`gcm -> git commit -m`; the quoted message terminates the chain, which is
exactly right - the message is what you type after the skeleton.

### Trigger gate (when inference fires)

All conditions must hold, evaluated at eol only. The widget enforces the
cheap ones as a pre-filter (zero subprocesses); the engine re-checks
everything authoritatively inside `__skel`, because the widget's view is
approximate (e.g. an empty query result does not prove the head is not an
alias - an alias whose positionals didn't match also returns empty):

1. `eol == true` (Enter, never Space) - widget, by construction
2. The normal expansion pass produced nothing - widget flow (query output
   was empty)
3. Head token length in `min-chars..=max-chars` (default 2..=6), all ASCII
   alphanumeric - widget (regex) AND engine (config-driven bounds)
4. Head token resolves in no shell namespace - widget only, via
   `whence -w` (PATH commands, builtins, functions, shell aliases, reserved
   words; the engine cannot see shell-local namespaces). Engine
   independently re-checks PATH and rejects paths (`contains('/')` or
   starts with `.`)
5. Head token is not a user alias and not a learned alias - engine only
   (the widget cannot see aka's alias map)

The PATH check uses a command-name set the daemon prebuilds (readdir of PATH
entries, refreshed on daemon start and on skel-index rebuild); direct mode
shells out to `which` once - acceptable because this branch only runs on
lines that already expanded to nothing.

### Frecency and aging (zoxide-adapted)

- Each candidate occurrence contributes `weight(age)`: 4.0 within a day, 2.0
  within a week, 1.0 within a month, 0.5 older (extended-history timestamps;
  plain-format entries all weigh 1.0)
- Acceptance reinforcement adds a fixed bump (default 10.0) to that exact
  skeleton->expansion pair
- Aging: when the sum of all scores exceeds `score-cap` (default 10000),
  every score is multiplied by 0.9 and entries below 1.0 are dropped -
  rank-decay first, deletion later, so seasonal commands degrade instead of
  vanishing
- Promoted entries atrophy in two stages: prolonged disuse first demotes to
  candidate (back below the trust boundary - next use goes through the
  confirm flow again), and only then can aging drop it

### Data Model

`~/.local/share/aka/skel.json` (engine-owned, atomic tmp+rename writes like
`aka.json`):

```json
{
  "version": 1,
  "history-offset": 631234,
  "history-identity": "dev=2049 ino=9182",
  "candidates": {
    "gcm": [ { "expansion": "git commit -m", "score": 84.5, "last-used": 1751600000 } ]
  },
  "accepts": {
    "gcm|git commit -m": { "count": 2, "last-accept": 1751600000 }
  },
  "learned": {
    "gcm": { "expansion": "git commit -m", "score": 120.0,
             "promoted-at": 1751600000, "last-used": 1751600000 }
  }
}
```

- The `accepts` key separator `|` is safe by construction: skeletons are
  alnum-only and expansions cannot contain `|` (the prefix chain stops at
  shell operators)
- `history-offset` + `history-identity` (dev+ino only - size changes on
  every append and must NOT be part of identity): incremental tail parsing;
  offset > current size (shrink) or identity change (zsh rewrites the file
  on trim) triggers a full rebuild
- Learned entries merge into `spec.aliases` at AKA construction time as
  `Alias { name: skeleton, value: expansion, space: true, global: false }`,
  AFTER the user config: on name collision the user alias wins and the
  learned entry is dropped with a `warn!`. `space: true` always - the
  learned tier cannot infer `space: false` semantics; a user who wants them
  graduates the entry into `aka.yml` by hand (which then wins by
  precedence)
- Learned entries carry an in-memory marker (`#[serde(skip)] learned: bool`
  on `Alias`). The marker does NOT do the filtering - `serde(skip)` omits
  the field, not the entry (review panel 2026-07-04, finding 3). Two
  explicit defenses:
  - WRITE SIDE: both `aka.json` persistence sites (the direct-mode
    write-on-use in `replace_with_mode` and the daemon's `flush_counts`
    snapshot) build their cache map via an explicit
    `.retain(|_, a| !a.learned)`-style filter on the cloned alias map
  - READ SIDE (self-healing): `AKA::new` already parses the YAML before
    adopting cache aliases; at that point any cache entry whose name is
    absent from the parsed spec is dropped with a `warn!` - a leaked entry
    cannot survive a restart even if a write-side filter ever regresses.
    (`AKA::from_cache` has no YAML to intersect against; that path is
    config-broken fallback and rides the write-side guarantee alone)
- Learned usage therefore persists to `skel.json`, via the same split as
  alias counts: direct mode writes through on use; the daemon marks the skel
  store dirty and the debounced flush carries it

### Config

New optional `skel:` section in `~/.config/aka/aka.yml` (hyphenated keys,
serde kebab-case; absent section = defaults with `enabled: false`):

```yaml
skel:
  enabled: false        # opt-in (new mechanisms default off)
  min-chars: 2
  max-chars: 6
  promote-after: 3      # accepts before promotion
  min-occurrences: 2    # frequency floor before a candidate is offered
  score-cap: 10000      # aging trigger
  history-file: ~/.zsh_history   # literal default; $HISTFILE is a shell
                                 # variable, usually unexported, and the
                                 # daemon runs under systemd - it can never
                                 # see it. AKA_HISTFILE env overrides for
                                 # tests (CLI > env > config > default).
```

`enabled` gates whether the daemon builds the index and whether `__skel`
returns candidates; the widget short-circuits identically. This is the
methodology-selection carve-out (there is no per-invocation CLI surface for
a ZLE-triggered engine), and opt-in matches the house rule that new
mechanisms never silently change fleet behavior.

Wiring (review panel 2026-07-04, finding 7 - config plumbing is the
most-skipped area and gets specified explicitly):

- `Spec` gains `#[serde(default)] skel: SkelConfig` (kebab-case renames;
  `deny_unknown_fields` on `SkelConfig` so a typoed key is a loud error,
  per house serde rules); an absent section deserializes to defaults with
  `enabled: false`
- `Loader::validate_config` grows range checks: `min-chars <= max-chars`,
  `promote-after >= 1`, `min-occurrences >= 1`, `score-cap > 0`; violations
  join the existing accumulated-errors report
- `skel.json` resolves through the SAME path helper chain as `aka.json`
  (`AKA_CACHE_DIR` beats `$XDG_DATA_HOME` beats `~/.local/share`), so it
  automatically inherits the systemd `Environment=` XDG snapshot the unit
  installer already writes - no new env-parity surface
- `skel.json` carries a `version` field: on load, any version other than
  current is treated exactly like corruption - quarantine to `.corrupt`
  and rebuild (the index is derived from history; only advisory accept
  counters are lost). This covers binary downgrade as well as upgrade
  (finding 9)

### API Design

Protocol (`protocol.rs`, additive). Mixed-version honesty: an OLD daemon
receiving a `Skel` request fails deserialization BEFORE any version check
(the version field lives inside the variant), so the client sees a read
timeout, which the widget treats as "no candidates" - graceful. The window is
bounded anyway: the first versioned request of any kind triggers the existing
version-mismatch self-restart.

```rust
DaemonRequest::Skel        { version, cmdline: String }
DaemonRequest::SkelAccept  { version, skeleton: String, expansion: String }
DaemonRequest::SkelDemote  { version, skeleton: String }
DaemonResponse::SkelCandidates { candidates: Vec<SkelCandidate> }  // ranked
// SkelCandidate { skeleton, expansion, score, learned: bool }
```

`SkelDemote` exists because the daemon OWNS `skel.json` (review panel
2026-07-04, finding 6): a CLI-side file edit would leave the daemon's
in-memory learned tier stale (still expanding the demoted skeleton) and the
next debounced flush would resurrect the entry. `aka skel --demote` routes
through the daemon when reachable (in-memory demotion + store marked dirty);
only when the daemon is down does it edit `skel.json` directly (the next
daemon start reloads it).

CLI:

- `aka __skel <cmdline>` (hidden): prints ranked candidates, one per line,
  `<head-expansion>\t<score>`; empty output = no candidates. The expansion
  covers the head token ONLY (it can never contain a tab: expansions are
  whitespace-joined skeletonizable words). The widget composes the new
  buffer as `${expansion}${BUFFER#$head}` - two lines of zsh; all gate and
  ranking logic stays in the engine.
- `aka __skel_accept <skeleton> <expansion>` (hidden): bumps the accept
  counter; performs promotion when the threshold crosses. Daemon-routed;
  when the daemon is unreachable the accept is dropped (advisory data, same
  stance as usage counts).
- `aka skel --list` | `--demote <skeleton>`: inspect and manage the learned
  tier. Traceability: `--demote` is the operator control named by two
  mitigations in this doc (binary-collision recovery, pinned-promotion
  re-learning) and required by requirement 5 (atrophy implies a manual
  demotion path); `--list` is the inspection surface without which the
  learned tier (requirement 4, "written to disk") is unauditable.

Widget (`init.zsh`):

- `_aka_accept_line` gains the fallback branch. Skel failures are silent
  fallthrough to `.accept-line` and do NOT feed `_aka_on_failure` - the
  circuit breaker is for config errors, not inference misses.
- The sk/fzf pick path (2+ candidates) joins the SAME pending flow: the
  picked candidate rewrites the buffer, sets `_AKA_SKEL_PENDING`, and the
  reinforcement fires on the subsequent Enter exactly as in the
  single-candidate case. Picking is not itself an accept - executing is.
- Picker cancel (ESC / empty selection): the buffer stays untouched and
  NOTHING executes - never fall through to `.accept-line` with the raw
  skeleton (that would run a guaranteed command-not-found).
- Pending-state lifecycle (review panel 2026-07-04, finding 4 - every
  transition explicit, no stale state):
  - SET: on skeleton rewrite (single-candidate or picker path),
    `_AKA_SKEL_PENDING="<skeleton>\t<expansion>"`
  - CONSUME: on the very next accept-line, checked FIRST against `$BUFFER`
    as typed (before any query transformation); verbatim expansion prefix
    -> fire `__skel_accept` in a detached subshell (`( ... & )`, output
    discarded - no job-control noise in ZLE), then clear
  - CLEAR: unconditionally in `zle-line-init` (a fresh prompt after
    Ctrl-C/send-break, an aborted line, or a completed command all start a
    new line) and on any accept-line where the verbatim check fails
    (edited-away = not accepted). Because the var cannot survive past the
    next line-init, a later unrelated command sharing the prefix can never
    falsely reinforce; a shell restart drops it trivially (session-local
    var). The detached accept racing the next prompt is harmless: accepts
    are advisory counters, ordering-independent
- Killswitch and session-disable already gate the whole widget; no new
  escape hatch needed.

### Implementation Plan

#### Phase 0: Spike - prove the history corpus supports this
**Model:** sonnet
- Throwaway parser (an `--ignored` test or scratch bin, deleted after) run
  against the real `~/.zsh_history`: handle zsh metafication (0x83 Meta
  prefix, byte XOR 0x20), EXTENDED_HISTORY `: <ts>:<dur>;cmd` lines, and
  multiline entries; report parsed/rejected counts
- Compute the skeleton index and report: total candidates, distribution of
  candidates-per-skeleton, top-20 most ambiguous skeletons, and hit-rate
  spot-check of 10 skeletons Scott would plausibly type - reported at
  `min-occurrences` floors of 1, 2, and 3 so the shipped default is
  evidence-based
- Scan the computed expansions to prove the layered secret guard holds on
  the real corpus: zero expansions containing `=`, quotes, digits beyond
  the head word, or any non-flag word that followed a flag word; manually
  eyeball the 50 longest expansions for anything value-shaped
- Specifically scan for the two DISCLOSED residual shapes (alpha-only bare
  words in subcommand position; dash-prefixed alpha-only words in
  flag-value position) - if the real corpus shows actual leakage of either
  shape, the Addendum's revisit condition for stricter chain termination
  fires BEFORE Phase 1
- **Success criteria:** >= 95% of history lines parse without corruption
  (spot-check non-ASCII entries round-trip); median candidates-per-skeleton
  <= 3; the spot-check skeletons return the intended command in the top
  rank for at least 8 of 10

#### Phase 1: skel module in aka_lib (parser, index, scoring)
**Model:** opus
- `skel/history.rs`: byte-level reader (no UTF-8 assumptions, unmetafy, then
  lossy-convert per entry), extended + plain formats, multiline entries,
  incremental tail parse with offset/identity tracking. TORN-WRITE RULE
  (review panel 2026-07-04, finding 5): the stored offset only ever
  advances past the last COMPLETE entry (terminated by an unescaped
  newline / followed by a valid next-entry header); a partial trailing
  entry - SHARE_HISTORY can fire the watcher mid-write() - is buffered and
  re-read next pass, never split into phantom commands, never skipped
- `skel/index.rs`: skeleton computation per the rules above, lookup with
  head-token gate
- `skel/score.rs`: frecency weights, acceptance bump, aging pass
- `skel/store.rs`: `skel.json` load/save (atomic), accept counters, learned
  map, version-mismatch quarantine
- `SkelConfig` structs, defaults, and `Loader::validate_config` range
  checks (the config wiring specified in the Config section)
- Unit tests per house test-file placement (`skel/tests.rs` etc.), including
  metafied bytes, truncated files, torn trailing entries (offset must not
  advance), secret-guard chain rules against adversarial fixtures, and
  aging edge cases
- **Success criteria:** fixture-history test asserts `gcm` -> `git commit -m`
  top-ranked; aging test asserts x0.9 scale + sub-1.0 drop when cap
  exceeded; a metafied fixture parses without U+FFFD in the indexed
  expansion; `otto ci` green

#### Phase 2: daemon + protocol + direct-mode fallback
**Model:** opus
- Daemon holds `SkelIndex`; second notify watch on the history file feeding
  the existing watcher loop; incremental rebuild on append, full rebuild on
  shrink/identity change; debounced persist of `skel.json` riding the
  `FlushHandle` cadence pattern
- `Skel` / `SkelAccept` protocol variants + handlers (version-checked like
  Query); PATH command-name set built at index-build time
- Direct mode: `__skel` loads persisted `skel.json` (staleness acceptable -
  inference is advisory); accepts are daemon-only
- **Success criteria:** integration test: append a line to a fixture
  history file, daemon serves the new skeleton after the watcher event is
  processed (the debounce governs persistence, not index freshness);
  equivalence-style test asserts daemon and direct `__skel` rank a fixture
  identically; `otto ci` green

#### Phase 3: CLI surface + zsh widget
**Model:** opus
- `__skel`, `__skel_accept` (hidden), `aka skel --list/--demote`
- `init.zsh`: fallback branch in `_aka_accept_line` (rewrite-no-execute),
  `_aka_skel_pick` sk/fzf picker, `_AKA_SKEL_PENDING` acceptance tracking,
  silent-fallthrough error handling
- Shell-test harness against REAL ZLE: the existing
  `tests/test_zsh_integration.sh` mocks zle and only exercises Space
  expansion, so the no-execute assertion would not bite there (review
  panel 2026-07-04, finding 8). Drive an interactive zsh via zpty (zsh's
  own pseudo-terminal module, used by zsh's test suite) and assert against
  observable effects: no history entry, no command side effect, buffer
  content
- **Success criteria:** real-ZLE test asserts a skeleton Enter rewrites the
  buffer and does NOT execute (no history entry created, a sentinel
  side-effect file is NOT created); a second Enter executes; with an aka
  shim counting invocations, an Enter on a builtin/function/alias head
  spawns ZERO `aka` subprocesses; `aka query` output contract is
  byte-identical for all existing test inputs

#### Phase 4: reinforcement, promotion, atrophy wiring
**Model:** opus
- Accept counter -> promotion at threshold -> learned map; learned merge
  into `spec.aliases` below user config with collision `warn!`
- Two-stage atrophy (learned -> candidate -> dropped) inside the aging
  pass, with an explicit clock-free threshold (review panel 2026-07-04,
  finding 8): "prolonged disuse" means the entry's score has decayed below
  `DEMOTE_SCORE_FLOOR` (named const, 1.0) at an aging pass - aging only
  fires on activity (score-cap breach), so the measure is
  activity-relative like zoxide's, deterministic, and testable without
  mocking wall-clock time
- `aka skel --list` shows tiers and scores; `--demote` moves learned ->
  candidate
- **Success criteria:** test: 3 accepts promote; promoted skeleton expands
  through the NORMAL alias path (asserted via `daemon_direct_equivalence`
  extension); a user alias named identically to a learned skeleton wins and
  the learned entry is dropped with a warning; demotion test round-trips

#### Phase 5: hardening and docs
**Model:** sonnet
- Negative-path tests: corrupt `skel.json` (quarantine like `aka.json`),
  missing HISTFILE, `enabled: false` short-circuits everywhere, daemon-down
  accept dropped without error
- README section; `aka --help` mentions `skel`; CLAUDE.md note
- **Success criteria:** corrupt-store and version-mismatch tests pass
  (rename to `.corrupt`, rebuild fresh); with `enabled: false` the `__skel`
  subcommand exits 0 with empty output, `aka skel --list` reports the tier
  empty, and no `skel.json` is created (state assertions, not log absence);
  `otto ci` green

## Acceptance Criteria

- [ ] With a fixture history containing `git commit -m ...` lines and
      `skel.enabled: true`, `aka __skel "gcm fix it"` prints
      `git commit -m` as the top-ranked head expansion, in both daemon and
      direct modes
- [ ] The zsh integration test proves a skeleton Enter never executes: the
      buffer is rewritten, no command runs, no history entry is created;
      only the subsequent Enter executes
- [ ] Three accepted expansions of the same skeleton promote it: it appears
      in `aka skel --list` as learned, expands via the normal alias path on
      both Space and Enter, and executes through
- [ ] Aging is provable: a unit test drives total score past `score-cap` and
      asserts the x0.9 rescale and sub-1.0 eviction; a disuse test asserts
      learned -> candidate demotion precedes deletion
- [ ] All pre-existing tests pass unmodified; with `skel.enabled: false`
      (the default) every new code path is inert and `aka query` behavior is
      byte-identical to v0.6.15

Invariant assertions (review panel 2026-07-04, finding 8):

- [ ] `aka.json` NEVER contains a learned entry: a test drives learned
      expansion through both write sites (direct write-on-use, daemon
      flush) and asserts the cache file holds only `aka.yml` names; a
      seeded leaked entry is dropped with a warning on the next `AKA::new`
- [ ] No skeleton trigger for shell-resolvable heads: in the real-ZLE
      harness, Enter on a builtin (`cd`), a function, and a shell alias
      spawns zero `aka` subprocesses (counted via shim)
- [ ] No bare-value secret leak: the Phase 1 adversarial fixture (flag
      values, digit tokens, `k=v`, quoted strings) produces zero expansions
      containing any of them; Phase 0's real-corpus scan is clean
- [ ] Pending state cannot go stale: real-ZLE tests assert `__skel_accept`
      does NOT fire after Ctrl-C-then-different-command and after editing
      the expanded prefix
- [ ] The history offset never advances past an incomplete trailing entry:
      a torn-write fixture parses, re-reads the partial on the next pass,
      and yields no phantom commands

## Resolved Decisions

- 2026-07-04 (Scott, this session): candidates come from shell history
  skeleton matching, not PATH/grammar inference ("I like it")
- 2026-07-04 (Scott, prior discussion): guessed expansions must never
  auto-execute; promotion is the trust boundary
- 2026-07-04 (draft): undelimited single-token skeletons only in v1; space
  cannot be the delimiter (it is the existing expansion trigger) and `.`/`:`
  sugar is parked
- 2026-07-04 (draft): `enabled: false` default - opt-in, per the house rule
  that new mechanisms don't silently change behavior
- 2026-07-04 (draft): learned entries merge into the alias map (identical
  behavior to hand-written aliases, including Space expansion) rather than
  living in a separate eol-only expansion stage - siblings behave identically
- 2026-07-04 (pass 5): acceptance requires the executed buffer to start with
  the expansion VERBATIM; any edit to the expanded prefix means not
  accepted. Rationale: tolerating edits risks reinforcing an expansion the
  user actively corrected; verbatim is conservative and the cost of a missed
  accept is one extra confirmation cycle
- 2026-07-04 (pass 5): `promote-after: 3` ships as the default - it is
  config, so tuning it is a YAML edit, not a release; no reason to block on
  bikeshedding a tunable
- 2026-07-04 (review panel, Architect + Staff Engineer): all 10 reconciled
  findings accepted and folded in - layered secret guard replacing the
  `=`-only rule (1), `whence -w` pre-gate covering all shell namespaces
  (2), explicit write-side retain + read-side intersection for the learned
  tier (3), full pending-state lifecycle with line-init clearing (4),
  torn-write offset rule (5), `SkelDemote` protocol variant (6), config/
  store wiring section (7), invariant-level acceptance criteria + real-ZLE
  harness + clock-free demotion threshold (8), version-mismatch quarantine
  (9). On (10), the reviewers diverged (Staff: `--list`/`--demote`
  unrequested scope; Architect: no unrequested scope found): resolved by
  the panel's own recommendation - both kept, with the traceability note
  now inline in API Design tying them to requirements 4 and 5 and to two
  named mitigations
- 2026-07-04 (review panel round 2): item 2 (traceability) confirmed closed
  by both reviewers. Item 1: both reviewers confirmed the three-layer guard
  kills every round-1 example; Staff Engineer surfaced one further shape -
  a flag value that itself begins with `-` and is alpha-only
  (`curl --user -Secret`) survives, because it reads as another flag - and
  pre-blessed two resolutions. Adopted resolution (ii): DISCLOSE the
  flag-value residual alongside the subcommand residual. Rationale: the
  shape requires a dash-prefixed, digit-free secret typed identically
  `min-occurrences`+ times (triple-conditioned rare); the content is
  already verbatim in `.zsh_history`; and the alternative (i) pays a
  permanent expressiveness cost on core value (multi-flag skeletons like
  `-d --build` and `--release --quiet` would be truncated). (i) is recorded
  in the Addendum with a Phase 0-gated revisit condition. Staff Engineer
  stated either resolution satisfies the objection, so this closes without
  a further round

## Alternatives Considered

### Alternative 1: PATH enumeration + per-binary grammar inference
- **Description:** resolve the first char against binaries in PATH, then
  infer subcommands/flags from `--help` output or man pages
- **Pros:** works for commands never yet typed; matches the original framing
- **Cons:** binaries don't publish grammars; `--help` formats are chaos;
  first-char ambiguity is hundreds of binaries wide with no personal ranking
  signal; enormous knowledge-acquisition surface
- **Why not chosen:** history-first gets the personally-relevant 90% as a
  lookup instead of an inference problem; Scott accepted the reframe

### Alternative 2: carapace-bin / completion-spec candidate source
- **Description:** use carapace's machine-readable specs (or drive zsh's
  completion system through a capture harness) as the subcommand/flag corpus
- **Pros:** structured, covers unseen commands, hundreds of tools
- **Cons:** new external dependency; spec coverage is uneven; still needs
  ranking, which only history provides
- **Why not chosen:** parked, not rejected - it is the natural second
  candidate source IF history-only hit rate disappoints (Phase 0 measures
  exactly this)

### Alternative 3: manual abbreviations (fish abbr / zsh-abbr style)
- **Description:** user defines `gcm` -> `git commit -m` by hand
- **Pros:** trivial; deterministic from day one
- **Cons:** identical to what aka aliases already do; no learning, which is
  the entire point of this feature
- **Why not chosen:** already possible today; solves nothing new

### Alternative 4: atuin as the history source
- **Description:** read atuin's SQLite DB (cwd, exit code, duration per
  command) instead of parsing `.zsh_history`
- **Pros:** richer context columns; no metafication parsing
- **Cons:** not installed on Scott's machines; adds a dependency on a tool
  aka doesn't otherwise need
- **Why not chosen:** parked; the parser is behind a trait-shaped seam
  (`skel/history.rs`) so an atuin source can slot in later

## Technical Considerations

### Dependencies

- No new crates anticipated: history parsing is byte-level std; JSON via
  existing serde_json; notify, xxhash, sk/fzf already in use. If a crate
  becomes genuinely necessary it enters via `cargo add` only.
- Cross-repo blast radius: none - single repo, no consumers of aka_lib
  outside this workspace. Ship order: this repo alone.

### Performance

- Space path: one HashMap lookup added only if promoted entries exist (they
  ride the existing alias map) - zero new subprocess or IO
- Enter path: inference branch runs only when normal expansion produced
  nothing AND the head token is unknown; daemon lookup is in-memory; the
  candidate index for 12k history lines is trivially small (Phase 0
  measures; 500k-line ceiling is why the index is prebuilt and persisted,
  never computed per-query)
- Index rebuild: incremental tail parse on append; full rebuild only on
  HISTFILE rewrite (SAVEHIST trim), bounded by daemon-side background work -
  never on the query path

### Security

- The skeleton index and `skel.json` contain command-shape PREFIXES from
  shell history, structurally stripped of values by the layered guard
  (`=`-termination, quote/free-text termination, flag-value termination,
  alpha-only continuation, frequency floor). Honest residual risks - TWO
  shapes, both requiring identical repetition `min-occurrences`+ times:
  1. An alpha-only bare word in subcommand position (`vault write
     supersecret` repeated)
  2. A flag value that itself begins with `-` and is alpha-only with no
     digits (`curl --user -Secret` repeated) - it reads as a flag and
     survives the chain (review panel round 2, Staff Engineer; resolution
     (ii) adopted, see Resolved Decisions and Addendum)
  In both cases the same content already sits verbatim in `.zsh_history`,
  so the delta is display surface (the picker) and one more
  same-permissions file, not new content exposure. Stated rather than
  hidden; the floor and the Phase 0 corpus scan bound both
- No network, no subprocess execution by the engine: candidates are TEXT
  returned to the widget; execution only ever happens via the user's own
  Enter through zsh
- The engine never writes `aka.yml`; learned state is confined to
  `skel.json`

### Testing Strategy

- Unit: parser (metafication, extended/plain, multiline, truncation), index
  rules (chain termination at operators/free text), scoring/aging,
  store round-trips, promotion/demotion state machine
- Integration: daemon Skel/SkelAccept over the socket, watcher-driven
  incremental rebuild, daemon/direct ranking equivalence, promoted-entry
  equivalence through the normal expansion path
- Shell: `test_zsh_integration.sh` extended - rewrite-without-execute is the
  single most important assertion in the feature and gets a dedicated test
- All fixtures hermetic via the existing env seams (`HOME`,
  `AKA_CACHE_DIR`); a new `AKA_HISTFILE` env override is the test seam for
  the history path

### Rollout Plan

- Ships default-off (`skel.enabled: false`); Scott flips it on his own
  config first
- `/cli-shakedown` after implementation per the standard funnel
- Promotion thresholds and aging constants are config, so tuning requires no
  release

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| History parsing corrupts on metafied/multiline entries | Med | Med | Phase 0 spike against the real file before any product code; byte-level parser, never UTF-8-first |
| Ambiguity too high - fzf picker fatigue kills the UX | Med | High | Phase 0 measures candidates-per-skeleton on real history; parked spec-sources and context ranking are the escape hatches |
| Guessed expansion executes (the catastrophic case) | Low | High | Structural: candidates are rewrite-only by design; the dedicated integration test asserts no-execute; promotion is the only path to execute-through |
| Nondeterminism vs muscle memory (same keys, different result) | Med | Med | Pinned promotion: learned entries never re-rank; below the line the user always sees the expansion before running |
| HISTFILE churn under SHARE_HISTORY (multi-session appends/rewrites) | Med | Low | Offset + identity tracking; shrink or identity change = full rebuild; index is advisory so a stale window is harmless |
| Learned entry collides with a later-installed binary | Low | Med | Deterministic learned entry wins (like any alias shadowing a binary); `aka skel --demote` is the operator control; collision logged |
| skel.json corruption | Low | Low | Same quarantine-and-rebuild pattern as `aka.json` (`.corrupt` rename) |
| Secret values from history land in the index / picker UI | Med | High | Layered structural guard: `=`-termination, flag-value termination (a non-flag word after a flag ends the chain), alpha-only continuation beyond the head, plus the `min-occurrences` frequency floor; Phase 0 scans the real corpus; residual risk (an alpha-only bare word in subcommand position typed repeatedly) stated in Security |
| Typos and failed commands pollute candidates (`gti status`) | High | Low | Accepted limitation: `.zsh_history` has no exit codes; frecency dilutes one-offs to bottom rank; atuin (parked) would add exit-code filtering |
| Concurrent skel.json writers (direct-mode shells + daemon flush) | Low | Low | Atomic tmp+rename prevents torn files; last-writer-wins count loss is the same accepted stance as `aka.json` today (daemon-routed in practice; data is advisory) |

## Open Questions

None - both draft questions were closed in pass 5 (see Resolved Decisions).

## References

- This repo, full source read 2026-07-04 (v0.6.15, `c5b5c06`) - daemon flush
  machinery (`aka-daemon.rs`), widget patterns (`src/shell/init.zsh`),
  cache/quarantine pattern (`lib.rs`)
- zoxide aging algorithm (frecency cap + proportional rescale)
- fish abbreviations / zsh-abbr (expansion UX prior art)
- carapace-bin (parked spec corpus)
- marquee post: `~scott-idler/aka-deep-dive-inner-workings` (architecture
  context)

## Addendum: parked options (do not re-litigate without the revisit condition)

- **Spec-based sources (carapace / completion capture):** revisit if Phase 0
  hit rate < 8/10 or real-world usage shows frequent "no candidates" misses
- **Delimited skeletons (`g.c.m`):** revisit if chords beyond 4-5 chars are
  common enough that undelimited typing errors hurt
- **Pipeline skeletons:** revisit post-v1; requires chain rules across `|`
  and a think about where free-text termination applies per-segment
- **Context ranking (cwd type detection, first-order Markov over command
  chains):** revisit once promotion mechanics have real usage data - it is a
  score adjustment layered on the same index, not a new architecture
- **atuin source:** revisit if atuin gets installed; `skel/history.rs` is
  the seam
- **Strict flag-value termination (review panel round 2, resolution (i)):**
  terminate the chain after EVERY flag word regardless of what follows -
  fully closes the dash-prefixed flag-value residual, at the price of
  gutting multi-flag skeletons (`docker compose up -d --build` loses
  `--build`, `cargo build --release --quiet` loses `--quiet`, `ls -l -a`
  loses `-a`), because without per-tool schemas the engine cannot tell
  "flag taking a value" from "flag followed by another flag". Revisit
  condition: Phase 0's real-corpus scan finds ACTUAL dash-prefixed
  flag-value leakage (not just the theoretical shape), or real usage ever
  surfaces a leaked secret of this shape in the picker
