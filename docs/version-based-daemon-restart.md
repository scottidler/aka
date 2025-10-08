# Version-Based Daemon Auto-Restart Design

## Problem Statement

When users run `cargo install --path .`, the binaries are replaced on disk but the daemon process continues running with the OLD code. This causes:

1. **New features don't work** - Daemon has old code
2. **Bug fixes don't apply** - Daemon has old bugs
3. **Silent failures** - No indication daemon is stale
4. **User confusion** - "I installed it but it doesn't work!"

**Current workaround:** Manually run `aka daemon --restart` after every install.

**User expectation:** It should "just work" after install.

## Solution: Version-Based Compatibility Check

Embed the version string (from `env!("GIT_DESCRIBE")`) in both CLI and daemon. On every IPC communication, compare versions. If they mismatch, daemon gracefully shuts down and SystemD auto-restarts it with the new binary.

### Why This Solution

1. ✅ **Already have version info** - Both binaries use `env!("GIT_DESCRIBE")`
2. ✅ **Exact detection** - Not "newer/older" but "exact version mismatch"
3. ✅ **Zero filesystem overhead** - No need to stat() binary files
4. ✅ **Protocol-level** - Built into IPC, part of the handshake
5. ✅ **Works everywhere** - Even if daemon is remote (future distributed mode)
6. ✅ **Transparent to user** - Automatic with no manual intervention
7. ✅ **Single request delay** - Only first request after update triggers restart

## Architecture

### Current State

```
CLI (v0.5.1)                    Daemon (v0.5.0)
│                               │
├─ DaemonRequest::Query         │
│  {                            │
│    cmdline: "desk"            │
│  }                            │
├──────────────────────────────>│
│                               ├─ Process with OLD code
│                               ├─ Return OLD cached values
│<───────────────────────────────┤
│                               │
└─ ❌ User gets wrong results
```

### Proposed State

```
CLI (v0.5.1)                    Daemon (v0.5.0)
│                               │
├─ DaemonRequest::Query         │
│  {                            │
│    version: "v0.5.1",         │
│    cmdline: "desk"            │
│  }                            │
├──────────────────────────────>│
│                               ├─ Check version
│                               ├─ v0.5.1 != v0.5.0
│                               ├─ ❌ VERSION MISMATCH
│                               │
│                               ├─ warn!("Version mismatch...")
│                               ├─ shutdown.store(true)
│                               └─ Exit gracefully
│
│  (SystemD detects exit)
│  (SystemD restarts daemon)
│
│                               Daemon (v0.5.1) ← New process
│                               │
├─ CLI retries automatically    │
├─ DaemonRequest::Query         │
│  {                            │
│    version: "v0.5.1",         │
│    cmdline: "desk"            │
│  }                            │
├──────────────────────────────>│
│                               ├─ Check version
│                               ├─ v0.5.1 == v0.5.1
│                               ├─ ✅ MATCH
│                               ├─ Process with NEW code
│<───────────────────────────────┤
│                               │
└─ ✅ User gets correct results
```

## Detailed Design

### 1. Protocol Changes

#### Current DaemonRequest (src/protocol.rs)

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    Query {
        cmdline: String,
        eol: bool,
        config: Option<PathBuf>,
    },
    List {
        global: bool,
        patterns: Vec<String>,
        config: Option<PathBuf>,
    },
    // ... etc
}
```

#### Proposed DaemonRequest

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    Query {
        version: String,          // ← NEW: CLI version
        cmdline: String,
        eol: bool,
        config: Option<PathBuf>,
    },
    List {
        version: String,          // ← NEW: CLI version
        global: bool,
        patterns: Vec<String>,
        config: Option<PathBuf>,
    },
    Freq {
        version: String,          // ← NEW: CLI version
        all: bool,
        config: Option<PathBuf>,
    },
    CompleteAliases {
        version: String,          // ← NEW: CLI version
        config: Option<PathBuf>,
    },
    Health,                       // ← No version check needed
    ReloadConfig,                 // ← No version check needed
    Shutdown,                     // ← No version check needed
}
```

**Alternative: Add version field to every variant**
- Pro: Consistent, every request has version
- Con: More boilerplate

**Decision: Add to user-facing requests only (Query, List, Freq, CompleteAliases)**
- Health/ReloadConfig/Shutdown are admin commands that work regardless of version

#### New DaemonResponse

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    // ... existing variants ...

    VersionMismatch {
        daemon_version: String,
        client_version: String,
        message: String,
    },
}
```

### 2. Daemon Changes

#### A. Add Version Constant

**File:** `src/bin/aka-daemon.rs`

```rust
const DAEMON_VERSION: &str = env!("GIT_DESCRIBE");
```

#### B. Version Check Function

**File:** `src/bin/aka-daemon.rs`

```rust
impl DaemonServer {
    fn check_version_compatibility(&self, client_version: &str) -> Result<()> {
        if client_version != DAEMON_VERSION {
            warn!("🔄 Version mismatch detected!");
            warn!("   Daemon version: {}", DAEMON_VERSION);
            warn!("   Client version: {}", client_version);
            warn!("   Initiating graceful shutdown for auto-restart");

            // Trigger shutdown
            self.shutdown.store(true, Ordering::Relaxed);

            // Return error to stop processing this request
            return Err(eyre!(
                "Version mismatch: daemon={}, client={}. Daemon shutting down for restart.",
                DAEMON_VERSION,
                client_version
            ));
        }
        Ok(())
    }
}
```

#### C. Check Version in handle_client()

**File:** `src/bin/aka-daemon.rs`

```rust
fn handle_client(&self, mut stream: UnixStream) -> Result<()> {
    // ... existing cache freshness check ...
    self.ensure_cache_fresh()?;

    // ... read request ...
    let request: Request = serde_json::from_str(&line.trim())?;

    // Extract version from request and check compatibility
    let client_version = match &request {
        Request::Query { version, .. } => Some(version.as_str()),
        Request::List { version, .. } => Some(version.as_str()),
        Request::Freq { version, .. } => Some(version.as_str()),
        Request::CompleteAliases { version, .. } => Some(version.as_str()),
        _ => None, // Admin commands don't require version check
    };

    if let Some(client_version) = client_version {
        if let Err(e) = self.check_version_compatibility(client_version) {
            // Send version mismatch response
            let response = Response::VersionMismatch {
                daemon_version: DAEMON_VERSION.to_string(),
                client_version: client_version.to_string(),
                message: format!("Daemon restarting to match client version"),
            };
            let response_json = serde_json::to_string(&response)?;
            writeln!(stream, "{}", response_json)?;

            // Return error to trigger shutdown
            return Err(e);
        }
    }

    // ... continue with normal request processing ...
}
```

### 3. CLI Changes

#### A. Add Version Constant

**File:** `src/bin/aka.rs`

```rust
const CLI_VERSION: &str = env!("GIT_DESCRIBE");
```

#### B. Include Version in All Requests

**File:** `src/bin/aka.rs`

When building requests in `handle_command_via_daemon_only_timed()`:

```rust
Command::Query(query_opts) => {
    let request = DaemonRequest::Query {
        version: CLI_VERSION.to_string(),  // ← NEW
        cmdline: query_opts.cmdline.clone(),
        eol: opts.eol,
        config: opts.config.clone(),
    };
    // ... send request ...
}

Command::List(list_opts) => {
    let request = DaemonRequest::List {
        version: CLI_VERSION.to_string(),  // ← NEW
        global: list_opts.global,
        patterns: list_opts.patterns.clone(),
        config: opts.config.clone(),
    };
    // ... send request ...
}

Command::Freq(freq_opts) => {
    let request = DaemonRequest::Freq {
        version: CLI_VERSION.to_string(),  // ← NEW
        all: freq_opts.all,
        config: opts.config.clone(),
    };
    // ... send request ...
}

Command::CompleteAliases => {
    let request = DaemonRequest::CompleteAliases {
        version: CLI_VERSION.to_string(),  // ← NEW
        config: opts.config.clone(),
    };
    // ... send request ...
}
```

#### C. Handle VersionMismatch Response

**File:** `src/bin/aka.rs`

In `handle_daemon_query_response()` and similar functions:

```rust
fn handle_daemon_query_response(
    response: DaemonResponse,
    timing: &mut TimingCollector,
) -> Result<i32> {
    match response {
        DaemonResponse::Success { data } => {
            println!("{}", data);
            timing.end_processing();
            Ok(0)
        },
        DaemonResponse::VersionMismatch { daemon_version, client_version, message } => {
            info!("🔄 Version mismatch: daemon={}, client={}", daemon_version, client_version);
            info!("   {}", message);
            info!("   Daemon is restarting, retrying request...");

            // Wait briefly for daemon to restart
            std::thread::sleep(std::time::Duration::from_millis(500));

            // Retry the request (daemon should be up with new version)
            // Return special code to indicate retry
            timing.end_processing();
            Err(eyre!("RETRY_REQUEST"))  // Special error for retry logic
        },
        DaemonResponse::Error { message } => {
            eprintln!("Daemon error: {}", message);
            timing.end_processing();
            Ok(1)
        },
        _ => {
            eprintln!("Unexpected daemon response");
            timing.end_processing();
            Ok(1)
        }
    }
}
```

#### D. Retry Logic

**File:** `src/bin/aka.rs`

Wrap daemon calls with retry logic:

```rust
fn handle_command_via_daemon_only_timed(opts: &AkaOpts, timing: &mut TimingCollector) -> Result<i32> {
    const MAX_RETRIES: u32 = 2;

    for attempt in 0..MAX_RETRIES {
        match try_daemon_request(opts, timing) {
            Ok(result) => return Ok(result),
            Err(e) if e.to_string() == "RETRY_REQUEST" => {
                if attempt < MAX_RETRIES - 1 {
                    debug!("🔄 Retrying request (attempt {}/{})", attempt + 2, MAX_RETRIES);
                    continue;
                } else {
                    return Err(eyre!("Max retries exceeded, daemon may be unhealthy"));
                }
            },
            Err(e) => return Err(e),
        }
    }

    Err(eyre!("Unexpected retry loop exit"))
}
```

### 4. SystemD Configuration

The daemon service must be configured for automatic restart:

**File:** `~/.config/systemd/user/aka-daemon.service`

```ini
[Service]
Type=simple
ExecStart=/home/user/.cargo/bin/aka-daemon
Restart=always          # ← Always restart on exit
RestartSec=0.5          # ← Wait 500ms before restart
```

This is already in the current implementation from `ServiceManager::install_systemd_service()`.

## Implementation Steps

### Phase 1: Protocol Changes

1. Update `DaemonRequest` enum in `src/protocol.rs`
   - Add `version: String` field to Query, List, Freq, CompleteAliases

2. Update `DaemonResponse` enum in `src/protocol.rs`
   - Add `VersionMismatch` variant

3. Update tests in `src/protocol.rs` to include version field

### Phase 2: Daemon Implementation

1. Add `DAEMON_VERSION` constant to `src/bin/aka-daemon.rs`

2. Add `check_version_compatibility()` method to `DaemonServer`

3. Update `handle_client()` to:
   - Extract version from request
   - Call `check_version_compatibility()`
   - Send `VersionMismatch` response if needed
   - Trigger shutdown on mismatch

4. Update daemon logging to show version on startup

### Phase 3: CLI Implementation

1. Add `CLI_VERSION` constant to `src/bin/aka.rs`

2. Update all request builders to include `version` field:
   - `handle_command_via_daemon_only_timed()` for Query
   - List request builder
   - Freq request builder
   - CompleteAliases request builder

3. Update response handlers:
   - `handle_daemon_query_response()`
   - `handle_daemon_list_response()`
   - Add handling for `VersionMismatch` response

4. Add retry logic:
   - Wrap daemon calls in retry loop
   - Wait 500ms after `VersionMismatch`
   - Retry up to 2 times

### Phase 4: Testing

1. Unit tests for version checking:
   - Test matching versions (should succeed)
   - Test mismatched versions (should trigger restart)

2. Integration tests:
   - Start daemon with version X
   - Send request with version Y
   - Verify daemon restarts
   - Verify retry succeeds

3. Manual testing:
   - Install version A
   - Start daemon
   - Install version B (without restarting daemon)
   - Run any aka command
   - Verify automatic restart and success

## Edge Cases

### 1. Daemon Restart Fails

**Scenario:** Daemon exits but SystemD fails to restart it

**Detection:** CLI retry times out after 2 attempts

**Handling:** Fall back to direct mode with warning:
```
⚠️  Daemon not responding after version mismatch
   Falling back to direct mode (slower)
   To fix: aka daemon --restart
```

### 2. Rapid Successive Requests During Restart

**Scenario:** User runs multiple commands quickly, all hit restarting daemon

**Detection:** Multiple requests get `VersionMismatch` simultaneously

**Handling:** Each CLI independently waits and retries. SystemD ensures only one daemon instance runs. First retry after restart succeeds, others follow.

### 3. Version Downgrade

**Scenario:** User installs older version (e.g., checkout old git commit)

**Detection:** Same as upgrade - version mismatch

**Handling:** Same behavior - daemon restarts with older binary

### 4. Development Mode

**Scenario:** Developer repeatedly builds and tests without installing

**Detection:** Binary on disk changes but daemon not restarted

**Handling:** Version check still triggers because `GIT_DESCRIBE` changes with commits. Developer workflow:
```bash
cargo build
cargo run -- daemon --restart  # Manual restart in dev mode
cargo run -- query "test"
```

**Alternative:** Add `--dev` flag that disables version check for development.

### 5. Multiple Daemons

**Scenario:** User accidentally starts multiple daemon processes

**Detection:** Not directly related to version check, but could cause confusion

**Handling:** SystemD prevents multiple instances (already handled). Socket file is exclusive.

### 6. Network/Remote Daemon (Future)

**Scenario:** Daemon runs on different machine than CLI

**Detection:** Version check still works (protocol-level)

**Handling:** Cannot auto-restart remote daemon. Return clear error:
```
❌ Version mismatch with remote daemon
   Daemon version: v0.5.0
   Client version: v0.5.1
   Remote daemon must be updated manually
```

## User Experience

### Scenario: User Updates aka

```bash
~/repos/aka $ cargo install --path .
   Compiling aka v0.5.1
   Installing aka v0.5.1
   Replaced executables: aka, aka-daemon

~/repos/aka $ aka query "desk"
# First request after install:
# - CLI v0.5.1 → Daemon v0.5.0
# - Daemon detects mismatch
# - Daemon exits gracefully
# - SystemD restarts daemon (v0.5.1)
# - CLI retries request
# - Success!
ssh desk.lan
```

**User sees:** Normal output, ~500ms delay on first command after install

**User doesn't see:** Any restart messages (unless using RUST_LOG=debug)

### Scenario: Install Fails to Restart Daemon

```bash
~/repos/aka $ cargo install --path .
   Compiling aka v0.5.1
   Installing aka v0.5.1

~/repos/aka $ aka query "desk"
⚠️  Daemon not responding after version mismatch
   Falling back to direct mode (slower)
   To fix: aka daemon --restart

ssh desk.lan
```

**User sees:** Warning and workaround

**Result:** Command still works (via direct mode)

## Backwards Compatibility

### Breaking Change

**This IS a breaking change** because we're adding a required field to the protocol.

### Migration Strategy

**Option 1: Hard Break (Recommended)**
- Old CLI cannot talk to new daemon (missing `version` field)
- New CLI cannot talk to old daemon (daemon doesn't expect `version` field)
- **Solution:** Document in release notes: "Must restart daemon after upgrading"

**Option 2: Graceful Degradation**
- Make `version` field optional: `version: Option<String>`
- Old requests without `version` → skip version check
- New requests with `version` → perform version check
- After 2-3 releases, make it required

**Decision: Option 1 - Hard Break**
- Simpler implementation
- We're pre-1.0, breaking changes acceptable
- Forces users to restart daemon properly
- Clear error message if they forget

### Release Notes

```markdown
## v0.5.1

### Breaking Changes

**Daemon restart required after upgrade**

This version adds automatic version detection. After installing,
the daemon will automatically restart on the first request.

If upgrading from v0.5.0 or earlier:
```bash
cargo install --path .
aka daemon --restart  # Required for v0.5.0 → v0.5.1
```

Future upgrades will not require manual restart.
```

## Monitoring & Debugging

### Log Messages

**Daemon startup:**
```
🚀 AKA Daemon starting...
📦 Version: v0.5.1-3-g1a2b3c4
✅ Daemon running (PID: 12345)
```

**Version mismatch detected:**
```
⚠️  Version mismatch detected!
   Daemon version: v0.5.0
   Client version: v0.5.1
   Initiating graceful shutdown for auto-restart
```

**SystemD logs:**
```bash
$ journalctl --user -u aka-daemon -f

Oct 08 15:30:42 hostname aka-daemon[12345]: Version mismatch detected
Oct 08 15:30:42 hostname systemd[1234]: aka-daemon.service: Succeeded.
Oct 08 15:30:42 hostname systemd[1234]: aka-daemon.service: Scheduled restart job
Oct 08 15:30:43 hostname aka-daemon[12346]: Daemon starting... Version: v0.5.1
```

### Health Check

Add version info to health check response:

```rust
Response::Health {
    status: format!("healthy:{}:synced:v{}", alias_count, DAEMON_VERSION)
}
```

```bash
$ aka daemon --status
🔍 AKA Daemon Status Check

📦 Daemon binary: ✅ Found at /home/user/.cargo/bin/aka-daemon
🔌 Socket file: ✅ Found at /run/user/1000/aka.sock
⚙️  Daemon process: ✅ Running (PID: 12345)
📊 Version: v0.5.1-3-g1a2b3c4
🏗️  SystemD service: ✅ Active

🚀 Overall status: ✅ Daemon is healthy and running
```

## Performance Impact

### Additional Overhead Per Request

1. **String comparison:** `client_version != DAEMON_VERSION`
   - Cost: ~10ns (pointer comparison after first interning)

2. **Version field in JSON:**
   - Additional bytes: ~20 bytes per request
   - Serialization cost: ~50ns

3. **Total overhead:** < 100ns per request (negligible)

### Restart Performance

1. **First request after install:**
   - CLI sends request: ~1ms
   - Daemon detects mismatch: ~10ns
   - Daemon shutdown: ~10ms
   - SystemD restart: ~200ms
   - CLI retry: ~1ms
   - **Total delay:** ~212ms one time

2. **Subsequent requests:**
   - No delay, versions match

## Future Enhancements

### 1. Protocol Version Negotiation

Instead of exact version match, use protocol version:

```rust
const PROTOCOL_VERSION: u32 = 1;

// In request:
protocol_version: 1,

// Daemon checks:
if client_protocol_version > DAEMON_PROTOCOL_VERSION {
    // Client is newer, daemon needs update
    restart();
} else if client_protocol_version < DAEMON_PROTOCOL_VERSION {
    // Client is older, suggest upgrade
    return VersionMismatch;
}
```

**Benefit:** Allows backwards-compatible protocol changes

### 2. Semantic Version Checking

```rust
// Only restart if major.minor differs
// Patch versions are compatible

let daemon_semver = parse_semver(DAEMON_VERSION);
let client_semver = parse_semver(client_version);

if daemon_semver.major != client_semver.major
   || daemon_semver.minor != client_semver.minor {
    restart();
}
```

**Benefit:** Reduces unnecessary restarts for patch releases

### 3. Graceful Degradation

```rust
if version_mismatch && request_is_simple() {
    // Simple queries might still work
    warn!("Version mismatch but attempting request");
    process_request();
} else {
    restart();
}
```

**Benefit:** Better UX during upgrades

## Testing Strategy

### Unit Tests

**File:** `src/protocol.rs`

```rust
#[test]
fn test_request_includes_version() {
    let request = DaemonRequest::Query {
        version: "v0.5.1".to_string(),
        cmdline: "test".to_string(),
        eol: true,
        config: None,
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("\"version\":\"v0.5.1\""));
}

#[test]
fn test_version_mismatch_response() {
    let response = DaemonResponse::VersionMismatch {
        daemon_version: "v0.5.0".to_string(),
        client_version: "v0.5.1".to_string(),
        message: "Restarting".to_string(),
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("VersionMismatch"));
}
```

### Integration Tests

**File:** `tests/version_mismatch_test.rs`

```rust
#[test]
fn test_version_mismatch_triggers_restart() {
    // Start daemon with mock version "v0.5.0"
    // Send request with version "v0.5.1"
    // Verify daemon exits
    // Verify VersionMismatch response received
}

#[test]
fn test_matching_versions_succeed() {
    // Start daemon with version "v0.5.1"
    // Send request with version "v0.5.1"
    // Verify request processes normally
}

#[test]
fn test_retry_after_mismatch() {
    // Mock scenario where daemon restarts
    // Verify CLI retries request
    // Verify second attempt succeeds
}
```

### Manual Testing

```bash
# Test 1: Normal operation
cargo build --release
cargo run --release --bin aka-daemon &
cargo run --release --bin aka -- query "test"
# Expected: Works normally

# Test 2: Simulate version mismatch
# Edit GIT_DESCRIBE in one binary
cargo run --release --bin aka -- query "test"
# Expected: Daemon restarts, command succeeds

# Test 3: Verify restart
journalctl --user -u aka-daemon -f
# Expected: See restart logs
```

## Success Criteria

1. ✅ After `cargo install`, first command auto-restarts daemon
2. ✅ User sees no errors or warnings (normal case)
3. ✅ Daemon logs version mismatch clearly
4. ✅ SystemD successfully restarts daemon
5. ✅ CLI retries and succeeds automatically
6. ✅ Subsequent commands have no delay
7. ✅ Falls back to direct mode if restart fails
8. ✅ All tests pass
9. ✅ No performance regression (< 100ns overhead)
10. ✅ Works with existing SystemD configuration

## Rollout Plan

### Phase 1: Implementation
- Implement protocol changes
- Implement daemon version check
- Implement CLI retry logic
- Write tests

### Phase 2: Testing
- Run full test suite
- Manual testing of various scenarios
- Test on fresh install
- Test on upgrade from v0.5.0

### Phase 3: Documentation
- Update CHANGELOG
- Add migration notes
- Update README with new behavior
- Document troubleshooting

### Phase 4: Release
- Tag release v0.5.1
- Push to main
- Announce breaking change
- Monitor for issues

## Related Documentation

- `docs/alias-caching-investigation.md` - The caching bug that revealed this issue
- `docs/daemon-architecture.md` - Overall daemon design
- `src/protocol.rs` - IPC protocol definitions

