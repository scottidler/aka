# Test Logging and Daemon Isolation Fix

## Problem Statement

When running `cargo test`, the test suite was causing two major issues:

1. **Production Log Contamination**: Test processes were writing logs to the production log location (`~/.local/share/aka/logs/aka.log`) instead of being isolated
2. **Rust Compiler Warnings**: Multiple unused variable warnings in test code
3. **Daemon Socket Interference**: Tests were connecting to the production daemon socket, causing system daemon processes to be triggered

## Root Cause Analysis

### Log Contamination
- Tests spawn `aka` and `aka-daemon` binaries as child processes using `std::process::Command`
- These child processes used the default logging configuration, writing to production logs
- Even when `AKA_LOG_FILE` environment variable was set, some processes still connected to the production daemon

### Daemon Socket Interference
- The daemon socket path is determined by:
  1. `XDG_RUNTIME_DIR` environment variable → `$XDG_RUNTIME_DIR/aka/daemon.sock`
  2. Fallback: `~/.local/share/aka/daemon.sock`
- Tests were connecting to the production daemon socket at `/run/user/1000/aka/daemon.sock`
- This triggered production daemon processes that logged to production location
- System-installed daemon service (`systemctl --user`) was also interfering

### Compiler Warnings
- Unused variables in test assertions (14 instances of `direct_stderr`, `daemon_stderr`, etc.)
- Variables were captured but not used in test validation logic

## Solution Implementation

### 1. Logging Isolation (`AKA_LOG_FILE` Environment Variable)

**Modified `src/lib.rs` - `setup_logging()` function:**
```rust
pub fn setup_logging(home_dir: &PathBuf) -> Result<()> {
    if is_benchmark_mode() {
        // Benchmark mode logs to stdout
        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Stdout)
            .init();
    } else {
        // Check for custom log file location via environment variable
        let log_file_path = if let Ok(custom_log_path) = std::env::var("AKA_LOG_FILE") {
            PathBuf::from(custom_log_path)
        } else {
            // Default to production location
            let log_dir = home_dir.join(".local").join("share").join("aka").join("logs");
            std::fs::create_dir_all(&log_dir)?;
            log_dir.join("aka.log")
        };

        // Ensure parent directory exists
        if let Some(parent) = log_file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)?;

        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Pipe(Box::new(log_file)))
            .init();
    }
    Ok(())
}
```

### 2. Daemon Socket Isolation (`XDG_RUNTIME_DIR` Environment Variable)

**Updated all test files to set isolated daemon socket:**

**`tests/daemon_direct_equivalence_test.rs`:**
```rust
// In start_daemon_with_config()
Command::new("target/debug/aka-daemon")
    .args(&["--config", config_path, "--foreground"])
    .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
    .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")  // Isolate daemon socket
    .spawn()

// In run_aka_command()
cmd.env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log");
cmd.env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime");  // Isolate daemon socket
```

**`tests/count_output_test.rs`:** (4 instances)
```rust
Command::new(&aka_binary)
    .args(&["--config", config_file.to_str().unwrap(), "ls"])
    .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
    .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")
```

**`tests/freq_integration_test.rs`:** (2 missing instances added)
```rust
Command::new(&aka_binary)
    .args(&["freq", "--help"])
    .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
    .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")
```

**`tests/complete_aliases_tests.rs`:** (3 instances)
```rust
// Changed from env_remove("XDG_RUNTIME_DIR") to:
Command::new("cargo")
    .args(&["run", "-q", "--", "-c", config_file.to_str().unwrap(), "__complete_aliases"])
    .env("HOME", home_dir.to_str().unwrap())
    .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
    .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")  // Isolate daemon socket
```

### 3. Compiler Warning Fixes

**`tests/daemon_direct_equivalence_test.rs`:**
- Replaced 14 instances of unused variables with `_`:
```rust
// Before:
let (stdout, direct_stderr, code) = run_aka_command(config_path, &["simple"]);

// After:
let (stdout, _, code) = run_aka_command(config_path, &["simple"]);
```

- Fixed 2 additional instances in `test_error_handling_equivalence`:
```rust
// Before:
let (direct_stdout, direct_stderr, direct_code) = ...
let (daemon_stdout, daemon_stderr, daemon_code) = ...

// After:
let (_, _, direct_code) = ...
let (_, _, daemon_code) = ...
```

### 4. System Daemon Service Management

**Identified interference from system-installed daemon:**
- System had `aka v0.5.3-20-g9a9a62f` installed via `cargo install`
- Running `systemctl --user` service: `aka-daemon.service`
- Service was auto-restarting daemon processes that logged to production

**Resolution:**
```bash
systemctl --user stop aka-daemon.service
systemctl --user disable aka-daemon.service
```

## Files Modified

### Core Library
- `src/lib.rs` - Added `AKA_LOG_FILE` environment variable support in `setup_logging()`

### Test Files
- `tests/daemon_direct_equivalence_test.rs` - Added socket isolation + fixed 16 unused variables
- `tests/count_output_test.rs` - Added socket isolation to 4 Command::new calls
- `tests/freq_integration_test.rs` - Added socket isolation to 2 missing Command::new calls
- `tests/complete_aliases_tests.rs` - Changed from removing XDG_RUNTIME_DIR to setting it

### Documentation
- `docs/test-warnings-and-logging-fix.md` - Initial planning document
- `docs/test-logging-and-daemon-isolation-fix.md` - This comprehensive summary

## Verification

### Test Results
- ✅ **No compiler warnings**: `cargo test 2>&1 | grep warning` returns empty
- ✅ **Isolated test logs**: Test processes write to `/tmp/aka-test-logs/aka.log`
- ✅ **Isolated daemon socket**: Test daemon socket created at `/tmp/aka-test-runtime/aka/daemon.sock`
- ✅ **No production log contamination**: Production logs remain unchanged during test runs

### Test Commands
```bash
# Verify no warnings
cargo test 2>&1 | grep warning || echo "✅ No warnings found"

# Verify log isolation
pkill -f aka-daemon
rm -rf /tmp/aka-test-logs /tmp/aka-test-runtime
tail -3 ~/.local/share/aka/logs/aka.log > /tmp/before.log
cargo test >/dev/null 2>&1
tail -3 ~/.local/share/aka/logs/aka.log > /tmp/after.log
diff /tmp/before.log /tmp/after.log && echo "✅ No production log contamination"
```

## Key Technical Insights

1. **Environment Variable Inheritance**: Child processes spawned by `std::process::Command` don't automatically inherit test environment - variables must be explicitly passed with `.env()`

2. **Daemon Socket Discovery**: The `determine_socket_path()` function in `src/lib.rs` uses `XDG_RUNTIME_DIR` as the primary method for socket location, making it the key isolation mechanism

3. **System Service Interference**: Installed system services can interfere with test isolation even when test code is properly configured

4. **Integration vs Unit Tests**: The `cfg!(test)` macro only works for unit tests, not integration tests that spawn separate binaries

## Future Considerations

1. **Automated Service Management**: Consider adding test setup that automatically manages system services
2. **Test Environment Validation**: Add checks to ensure test isolation is working correctly
3. **Documentation**: Update README with testing guidelines and troubleshooting steps
4. **CI/CD Integration**: Ensure build systems properly handle daemon service management