# Test Warnings and Logging Fix Plan

## Overview

This document outlines the plan to address two critical issues identified during `cargo test`:
1. **14 unused variable warnings** in test files
2. **Test logging contaminating production log location** (`~/.local/share/aka/logs/aka.log`)

## Current Issues

### Issue 1: Unused Variable Warnings

**Problem:** The `daemon_direct_equivalence_test.rs` file generates 14 warnings for unused variables:
- `direct_stderr` (7 instances)
- `daemon_stderr` (7 instances)

**Root Cause:** Tests capture stderr output from `run_aka_command()` but never use it in assertions. The tests only verify stdout content and exit codes for equivalence checking between daemon and direct modes.

**Warning Example:**
```
warning: unused variable: `direct_stderr`
   --> tests/daemon_direct_equivalence_test.rs:110:29
    |
110 |         let (direct_stdout, direct_stderr, direct_code) = run_aka_command(config_path, &["query", test_case]);
    |                             ^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_direct_stderr`
```

### Issue 2: Test Logging to Production Location

**Problem:** During `cargo test`, the actual `aka` binary is executed, which calls `setup_logging()` and writes logs to the production location: `~/.local/share/aka/logs/aka.log`

**Root Cause:** The `setup_logging()` function in `src/lib.rs` always uses the production log path regardless of test context.

**Current Logging Logic:**
```rust
pub fn setup_logging(home_dir: &PathBuf) -> Result<()> {
    if is_benchmark_mode() {
        // Log to stdout for visibility
        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Stdout)
            .init();
    } else {
        // Always logs to production location
        let log_dir = home_dir.join(".local").join("share").join("aka").join("logs");
        // ...
    }
}
```

## Solution Plan

### Phase 1: Make Log File Location Configurable

**Approach:** Use an environment variable to specify the log file location, allowing tests to redirect logs to `/tmp`.

**Implementation:**
```rust
// Update setup_logging() in src/lib.rs
pub fn setup_logging(home_dir: &PathBuf) -> Result<()> {
    if is_benchmark_mode() {
        // In benchmark mode, log to stdout for visibility
        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Stdout)
            .init();
    } else {
        // Check if custom log file location is specified via environment variable
        let log_file_path = if let Ok(custom_log_path) = std::env::var("AKA_LOG_FILE") {
            PathBuf::from(custom_log_path)
        } else {
            // Default to production location
            let log_dir = home_dir.join(".local").join("share").join("aka").join("logs");
            std::fs::create_dir_all(&log_dir)?;
            log_dir.join("aka.log")
        };

        // Ensure the parent directory exists
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

**Rationale:**
- `AKA_LOG_FILE` environment variable allows explicit control of log file location
- Tests set `AKA_LOG_FILE=/tmp/aka-test-logs/aka.log` to isolate test logs
- Much cleaner than trying to auto-detect test mode
- Production behavior unchanged when environment variable not set
- Works with all test runners and scenarios

### Phase 2: Update Test Functions to Set Log Location

**Modify test helper functions to set `AKA_LOG_FILE` environment variable:**
```rust
// In tests/daemon_direct_equivalence_test.rs
fn start_daemon_with_config(config_path: &str) -> std::process::Child {
    // ... build daemon binary ...

    Command::new("target/debug/aka-daemon")
        .args(&["--config", config_path, "--foreground"])
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")  // Direct test logs to /tmp
        .spawn()
        .expect("Failed to start daemon")
}

fn run_aka_command(config_path: &str, args: &[&str]) -> (String, String, i32) {
    let aka_binary = get_aka_binary_path();
    let mut cmd = Command::new(&aka_binary);
    cmd.args(&["--config", config_path]);
    cmd.args(args);
    cmd.env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log");  // Direct test logs to /tmp

    let output = cmd.output().expect("Failed to run aka command");
    // ... return output ...
}
```

**Benefits:**
- ✅ Test logs isolated to `/tmp/aka-test-logs/aka.log`
- ✅ Production logs remain untouched during testing
- ✅ Easy cleanup (system clears `/tmp` automatically)
- ✅ No impact on production or benchmark modes
- ✅ Simple and explicit approach

### Phase 3: Fix Unused Variable Warnings

**Strategy:** Remove unused stderr variables entirely rather than prefixing with underscores.

**Before:**
```rust
let (direct_stdout, direct_stderr, direct_code) = run_aka_command(config_path, &["query", test_case]);
let (daemon_stdout, daemon_stderr, daemon_code) = run_aka_command(config_path, &["query", test_case]);
```

**After:**
```rust
let (direct_stdout, _, direct_code) = run_aka_command(config_path, &["query", test_case]);
let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, &["query", test_case]);
```

**Rationale:**
- Tests don't need stderr for equivalence checking
- Using `_` is cleaner than `_prefixed_names`
- Follows Rust best practices for intentionally unused values
- No functional change to test behavior

**Files to Update:**
- `tests/daemon_direct_equivalence_test.rs` - 14 instances across 7 test functions

### Phase 4: Verification

**Tasks:**
1. Run `cargo test` and verify 0 warnings
2. Confirm test logs appear in `/tmp/aka-test-logs/aka.log`
3. Verify production logging still works normally
4. Check for any other unused variable warnings in codebase

## Implementation Details

### Test Mode Detection Logic

The `is_test_mode()` function will detect:
- `CARGO_TEST` environment variable (set automatically by `cargo test`)
- This provides automatic test detection without manual setup

### Log Path Strategy

**Test Mode:** `/tmp/aka-test-logs/aka.log`
- Isolated from production
- Automatically cleaned by system
- Easy to inspect if needed

**Production Mode:** `~/.local/share/aka/logs/aka.log` (unchanged)
- Existing behavior preserved
- No impact on normal usage

**Benchmark Mode:** `stdout` (unchanged)
- Existing behavior preserved
- Visibility for performance analysis

### Unused Variable Cleanup

**Pattern Applied to 14 instances:**
```rust
// BEFORE (generates warning)
let (stdout, stderr, code) = run_aka_command(...);

// AFTER (no warning)
let (stdout, _, code) = run_aka_command(...);
```

## Benefits

### Code Quality
- ✅ Zero unused variable warnings
- ✅ Clean, intentional code patterns
- ✅ No `#[allow(dead_code)]` workarounds needed

### Test Infrastructure
- ✅ Isolated test logging
- ✅ No contamination of production logs
- ✅ Automatic cleanup via `/tmp`

### Maintainability
- ✅ Consistent with existing patterns (`is_benchmark_mode()`)
- ✅ Clear separation of concerns
- ✅ Easy to understand and modify

## Files Modified

### Core Changes
- `src/lib.rs` - Add `is_test_mode()` and update `setup_logging()`

### Test Fixes
- `tests/daemon_direct_equivalence_test.rs` - Fix 14 unused variable warnings

### Documentation
- `docs/test-warnings-and-logging-fix.md` - This document

## Testing Strategy

### Verification Steps
1. **Warning Elimination:** `cargo test` produces 0 warnings
2. **Log Isolation:** Test logs appear in `/tmp/aka-test-logs/` only
3. **Production Safety:** Normal usage still logs to `~/.local/share/aka/logs/`
4. **Functionality:** All 200+ tests continue to pass

### Test Commands
```bash
# Verify no warnings
cargo test 2>&1 | grep warning

# Check test log location
cargo test && ls -la /tmp/aka-test-logs/

# Verify production logging unchanged
./target/debug/aka --help && ls -la ~/.local/share/aka/logs/
```

## Conclusion

This plan addresses both issues comprehensively:

1. **Eliminates all 14 unused variable warnings** through proper cleanup
2. **Isolates test logging to `/tmp`** preventing production log contamination
3. **Maintains existing functionality** with zero behavioral changes
4. **Follows established patterns** consistent with the codebase architecture

The solution is minimal, targeted, and preserves all existing functionality while improving code quality and test isolation.