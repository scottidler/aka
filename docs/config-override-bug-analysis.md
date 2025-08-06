# Config Override Bug Analysis

## Executive Summary

During testing of the `aka` command-line tool, a critical bug was discovered where the `--config` parameter is **completely non-functional in both direct and daemon modes**. This represents a fundamental architectural failure where a core feature has never worked properly.

## Problem Statement

The `count_output_test.rs` integration tests were failing with the following pattern:
- **Expected**: 3-5 lines of output from test configs with 2-3 aliases
- **Actual**: 406-420 lines of output from the system config with 400+ aliases

This indicated that the `--config` parameter was being ignored entirely, and the system was always loading the default configuration file.

## Core Architectural Principle Violation

**CRITICAL**: The `--config` parameter should have **ZERO dependency** on whether the daemon is running or not. This is a fundamental command-line interface principle:

- **User passes `--config /path/to/custom.yml`** → **System MUST use that config**
- **Daemon running or not running** → **IRRELEVANT to config selection**
- **Any mode (direct/daemon)** → **MUST honor the user's explicit config choice**

The daemon is a **performance optimization**, not a functional requirement. Users should get identical behavior regardless of daemon state when using `--config`. The current implementation violates this basic expectation by making config loading dependent on internal architectural decisions that should be transparent to the user.

## Initial Misdiagnosis: The Daemon Red Herring

### False Lead
Initially, the bug appeared to be daemon-related because:
- **With daemon running**: 420 lines of output
- **Without daemon running**: 406 lines of output

This difference suggested daemon interference, leading to investigation of the IPC protocol and daemon request handling.

### The Real Issue Revealed
After thorough investigation, it became clear that **both modes were loading the wrong configuration**:
- **420 lines** = system config loaded by daemon + additional formatting
- **406 lines** = system config loaded by direct mode

The `--config` parameter was being ignored in **both cases**.

## Technical Analysis

### Test Case Analysis

The failing tests in `tests/count_output_test.rs`:

```rust
#[test]
fn test_ls_command_shows_count() {
    // Creates temp config with 3 aliases
    let config_content = r#"
aliases:
  test1:
    value: "echo test1"
    global: false
  test2:
    value: "echo test2"
    global: false
  test3:
    value: "echo test3"
    global: false
"#;

    // Expects 5 lines: 3 aliases + empty line + count line
    let output = Command::new(&aka_binary)
        .args(&["--config", config_file.to_str().unwrap(), "ls"])
        .output()
        .expect("Failed to run aka ls");

    assert_eq!(lines.len(), 5, "Should have 3 aliases + empty line + count line");
}
```

### Actual vs Expected Behavior

| Mode | Expected Behavior | Actual Behavior | Root Cause |
|------|------------------|-----------------|------------|
| Direct | Load `/tmp/test.yml` (3 aliases) | Load `~/.config/aka/aka.yml` (406 aliases) | `--config` parameter ignored |
| Daemon | Send config path to daemon, load custom config | Daemon uses its default config (420 aliases) | Client doesn't send config path |

### Evidence Collection

**Test Configuration File:**
```yaml
aliases:
  test1:
    value: "echo test1"
    global: false
  test2:
    value: "echo test2"
    global: false
  test3:
    value: "echo test3"
    global: false
```

**Direct Mode Test:**
```bash
target/debug/aka --config /tmp/debug_test.yml ls | wc -l
# Expected: 5
# Actual: 406
```

**Daemon Mode Test:**
```bash
# With daemon running
target/debug/aka --config /tmp/debug_test.yml ls | wc -l
# Expected: 5
# Actual: 420
```

## Root Cause Analysis

### 1. Client Argument Parsing Failure
The `--config` parameter is parsed but not properly utilized in either execution path.

### 2. Direct Mode Config Loading Bug
Direct mode execution ignores the custom config path and defaults to system config resolution.

### 3. Daemon Communication Gap
The client fails to include the custom config path in daemon requests (this was partially fixed).

### 4. Silent Failure Pattern
The system fails silently when custom configs cannot be loaded, falling back to default behavior without error reporting.

## Impact Assessment

### Severity: CRITICAL
- **Core Feature Broken**: The `--config` parameter is advertised but non-functional
- **Test Suite Compromised**: Integration tests cannot reliably test custom configurations
- **User Experience**: Users cannot override configurations as documented
- **Silent Failure**: No error reporting when custom configs fail to load

### Affected Components
- `src/bin/aka.rs` - Client argument handling
- Direct mode execution path
- Daemon communication protocol (partially fixed)
- Integration test reliability
- User documentation accuracy

## Technical Details

### The Daemon Fix (Completed)
One issue was discovered and fixed in the daemon's request handling:

**Before:**
```rust
// In daemon request handler
match temp_aka.replace_with_mode(&cmdline, ProcessingMode::Daemon) {
    // This created infinite recursion - daemon calling daemon
}
```

**After:**
```rust
// Fixed to use direct processing for custom configs
match temp_aka.replace_with_mode(&cmdline, ProcessingMode::Direct) {
    // Temporary AKA instance processes directly
}
```

### The Real Bug (Still Exists)
The fundamental issue remains in the client code where `--config` parameter handling is broken.

## Debugging Instructions

### Safe Testing Protocol
When debugging this issue, follow these critical guidelines:

1. **Use full paths to avoid alias interference:**
   ```bash
   /bin/cat /path/to/config.yml  # NOT: cat /path/to/config.yml
   ```

2. **Never run processes in foreground during testing:**
   ```bash
   # WRONG - blocks terminal and interferes with testing
   cargo run --bin aka-daemon -- --foreground

   # CORRECT - background or separate terminal
   cargo run --bin aka-daemon &
   ```

3. **Kill daemon processes between tests:**
   ```bash
   pkill -f aka-daemon
   ps aux | grep aka-daemon | grep -v grep  # Verify killed
   ```

### Reproduction Steps

1. **Create test config:**
   ```bash
   echo 'aliases:
     test1:
       value: "echo test1"
       global: false' > /tmp/test_config.yml
   ```

2. **Test direct mode:**
   ```bash
   pkill -f aka-daemon  # Ensure no daemon
   target/debug/aka --config /tmp/test_config.yml ls
   # Should show 1 alias, actually shows 400+
   ```

3. **Test daemon mode:**
   ```bash
   cargo run --bin aka-daemon &
   sleep 2
   target/debug/aka --config /tmp/test_config.yml ls
   # Should show 1 alias, actually shows 400+
   pkill -f aka-daemon
   ```

## Required Fixes

### Priority 1: Honor `--config` Flag Universally
- **ABSOLUTE REQUIREMENT**: When `--config` is specified, that config MUST be used regardless of daemon state
- Fix argument parsing in `src/bin/aka.rs` to properly handle config overrides
- Ensure `--config` path is properly passed to both execution modes
- Add validation for config file existence
- **NO EXCEPTIONS**: Daemon running/not running should have ZERO impact on config selection

### Priority 2: Error Handling
- Fail loudly when custom config cannot be loaded
- Provide clear error messages for invalid config paths
- Add logging for config resolution process
- Never silently fall back to system config when `--config` is specified

### Priority 3: Test Infrastructure
- Update integration tests to properly isolate daemon processes
- Add explicit config override validation tests
- Ensure test configs are actually being used
- Test both daemon and direct modes with identical `--config` behavior

### Priority 4: Documentation
- Update CLI help to accurately reflect `--config` behavior
- Add examples of custom config usage
- Document that `--config` behavior is identical regardless of daemon state
- Emphasize that daemon is performance optimization only

## Lessons Learned

### Why This Bug Persisted
1. **False Positive Tests**: Tests appeared to work due to system config fallback
2. **Silent Failures**: No error reporting masked the underlying issue
3. **Complex Execution Paths**: Multiple modes obscured the root cause
4. **Insufficient Validation**: No verification that custom configs were actually loaded

### Development Best Practices
1. **Explicit Validation**: Always verify that custom configurations are loaded
2. **Fail Fast**: Error loudly when core functionality doesn't work
3. **Isolated Testing**: Ensure tests don't depend on system state
4. **Clear Error Messages**: Make debugging easier with descriptive errors

## Conclusion

The `--config` parameter bug represents a critical failure in core functionality that has existed since the feature was implemented. While the daemon communication protocol has been fixed, the fundamental client-side argument handling remains broken.

**The fundamental principle is simple**: When a user specifies `--config /path/to/file.yml`, that file MUST be used, period. The daemon's existence or non-existence should have absolutely no bearing on this behavior. The daemon is an internal performance optimization that should be completely transparent to the user.

This bug demonstrates the importance of thorough integration testing and the dangers of silent fallback behavior in command-line tools. The fix requires careful attention to argument parsing, config resolution, and error handling across both execution modes, with the absolute requirement that `--config` behavior is identical regardless of internal architectural state.

**Status**: The daemon communication aspect has been resolved, but the core `--config` parameter functionality remains completely broken and requires immediate attention. The fix must ensure that daemon state has zero impact on config file selection.