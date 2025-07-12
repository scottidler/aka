# Sudo Wrapping Bug Fix Design Plan

**Version:** 1.0
**Date:** 2025-01-20
**Status:** Design Phase

## Executive Summary

This document outlines the comprehensive fix for the sudo command wrapping bug in the aka system. The current implementation indiscriminately wraps all commands after `sudo` with `$(which <command>)` and creates recursive wrapping structures, leading to broken terminal commands and poor user experience.

### Key Problems Identified

1. **Indiscriminate Wrapping**: Every command after `sudo` gets wrapped, regardless of necessity
2. **Recursive Wrapping**: Already wrapped commands get re-wrapped, creating `$(which $(which command))` structures
3. **System Command Pollution**: Built-in and system commands unnecessarily wrapped
4. **Performance Impact**: Unnecessary subshell execution for commands that don't need it

### Solution Approach

**Smart Runtime Detection**: Use runtime checks to determine if wrapping is needed, avoiding any hardcoded lists or static databases.

## Current Architecture Problems

### Problem Analysis

The current implementation in `src/lib.rs` lines 763-765:

```rust
if sudo {
    args[0] = format!("$(which {})", args[0]);
    args.insert(0, "sudo".to_string());
}
```

This logic is applied **unconditionally** after sudo detection, causing:

1. **Double Wrapping**: `sudo $(which ls)` becomes `sudo $(which $(which) ls)`
2. **Unnecessary Wrapping**: `sudo systemctl` becomes `sudo $(which systemctl)`
3. **Broken Commands**: Complex nested subshells that fail to execute

### Current Behavior Examples

| Input | Current Output | Problem |
|-------|---------------|---------|
| `sudo ls` | `sudo $(which eza)` | May be correct if `ls` → `eza` alias |
| `sudo $(which ls)` | `sudo $(which $(which) ls)` | Recursive wrapping |
| `sudo systemctl` | `sudo $(which systemctl)` | Unnecessary wrapping |
| `sudo prompt` | `sudo $(which prompt)` | Wraps non-existent commands |

## Target Architecture

### Design Principles

1. **Runtime Intelligence**: Determine wrapping necessity at execution time
2. **No Static Lists**: Avoid hardcoded command databases
3. **Idempotent Operations**: Wrapping should be safe to apply multiple times
4. **Performance Conscious**: Minimize overhead for common operations

### Smart Detection Strategy

#### 1. Already Wrapped Detection
```rust
fn is_already_wrapped(command: &str) -> bool {
    // Detect $(which ...) patterns with proper parsing
    command.trim_start().starts_with("$(which ") && command.trim_end().ends_with(")")
}
```

#### 2. Runtime Availability Check
```rust
fn is_command_available_to_root(command: &str) -> bool {
    // Use `sudo which <command>` to check root's PATH
    // This is the ONLY reliable way to determine root availability
    std::process::Command::new("sudo")
        .args(&["-n", "which", command])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
```

#### 3. User-Only Command Detection
```rust
fn is_user_only_command(command: &str) -> bool {
    // Check if command exists in user PATH but not root PATH
    let user_has_it = std::process::Command::new("which")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !user_has_it {
        return false; // User doesn't have it, no point wrapping
    }

    // Check if root has it (with timeout for safety)
    !is_command_available_to_root(command)
}
```

### Core Logic Redesign

#### Intelligent Wrapping Function
```rust
fn needs_sudo_wrapping(command: &str) -> bool {
    // Skip if already wrapped
    if is_already_wrapped(command) {
        return false;
    }

    // Skip if it's a complex command (contains spaces, pipes, etc.)
    if command.contains(' ') || command.contains('|') || command.contains('&') {
        return false;
    }

    // Only wrap if it's a user-only command
    is_user_only_command(command)
}
```

#### Updated Sudo Processing
```rust
if sudo && needs_sudo_wrapping(&args[0]) {
    args[0] = format!("$(which {})", args[0]);
    args.insert(0, "sudo".to_string());
} else if sudo {
    // Just add sudo without wrapping
    args.insert(0, "sudo".to_string());
}
```

## Implementation Plan

### Phase 1: Core Detection Functions

#### 1.1 Implement Already-Wrapped Detection
**Location:** `src/lib.rs` (new utility functions)

```rust
/// Detect if a command is already wrapped with $(which ...)
fn is_already_wrapped(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed.starts_with("$(which ") && trimmed.ends_with(")")
}

/// Extract the actual command from a wrapped command
fn extract_wrapped_command(command: &str) -> Option<&str> {
    let trimmed = command.trim();
    if trimmed.starts_with("$(which ") && trimmed.ends_with(")") {
        let inner = &trimmed[8..trimmed.len()-1]; // Remove "$(which " and ")"
        Some(inner.trim())
    } else {
        None
    }
}
```

#### 1.2 Implement Runtime Command Availability Check
**Location:** `src/lib.rs` (new utility functions)

```rust
/// Check if a command is available to the root user
fn is_command_available_to_root(command: &str) -> bool {
    // Use sudo -n (non-interactive) to check root's PATH
    // This is the only reliable way to determine root availability
    std::process::Command::new("sudo")
        .args(&["-n", "which", command])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Check if a command exists in user PATH but not root PATH
fn is_user_only_command(command: &str) -> bool {
    // First check if user has the command
    let user_has_command = std::process::Command::new("which")
        .arg(command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !user_has_command {
        return false; // User doesn't have it, no point wrapping
    }

    // Check if root also has it
    !is_command_available_to_root(command)
}
```

#### 1.3 Implement Smart Wrapping Logic
**Location:** `src/lib.rs` (new utility function)

```rust
/// Determine if a command needs sudo wrapping
fn needs_sudo_wrapping(command: &str) -> bool {
    // Skip if already wrapped (idempotent)
    if is_already_wrapped(command) {
        debug!("Command already wrapped: {}", command);
        return false;
    }

    // Skip complex commands (contain spaces, pipes, redirects, etc.)
    if command.contains(' ') || command.contains('|') || command.contains('&')
       || command.contains('>') || command.contains('<') {
        debug!("Skipping complex command: {}", command);
        return false;
    }

    // Only wrap if it's available to user but not root
    let needs_wrapping = is_user_only_command(command);
    debug!("Command '{}' needs wrapping: {}", command, needs_wrapping);
    needs_wrapping
}
```

### Phase 2: Update Main Processing Logic

#### 2.1 Replace Unconditional Wrapping
**Location:** `src/lib.rs` lines 763-765

```rust
// BEFORE:
if sudo {
    args[0] = format!("$(which {})", args[0]);
    args.insert(0, "sudo".to_string());
}

// AFTER:
if sudo {
    if needs_sudo_wrapping(&args[0]) {
        args[0] = format!("$(which {})", args[0]);
        debug!("Wrapped sudo command: {}", args[0]);
    } else {
        debug!("Sudo command does not need wrapping: {}", args[0]);
    }
    args.insert(0, "sudo".to_string());
}
```

### Phase 3: Comprehensive Testing

#### 3.1 Unit Tests for Detection Functions
**Location:** `src/lib.rs` (in existing test module)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_already_wrapped_detection() {
        assert!(is_already_wrapped("$(which ls)"));
        assert!(is_already_wrapped("  $(which ls)  "));
        assert!(!is_already_wrapped("ls"));
        assert!(!is_already_wrapped("which ls"));
        assert!(!is_already_wrapped("$(ls)"));
    }

    #[test]
    fn test_extract_wrapped_command() {
        assert_eq!(extract_wrapped_command("$(which ls)"), Some("ls"));
        assert_eq!(extract_wrapped_command("  $(which ls)  "), Some("ls"));
        assert_eq!(extract_wrapped_command("$(which ls -la)"), Some("ls -la"));
        assert_eq!(extract_wrapped_command("ls"), None);
    }

    #[test]
    fn test_needs_sudo_wrapping() {
        // Should not wrap already wrapped commands
        assert!(!needs_sudo_wrapping("$(which ls)"));

        // Should not wrap complex commands
        assert!(!needs_sudo_wrapping("ls -la"));
        assert!(!needs_sudo_wrapping("cat file.txt"));
        assert!(!needs_sudo_wrapping("grep pattern | less"));

        // Should wrap user-only commands (if they exist)
        // Note: These tests depend on system state, so we'll mock them
    }

    #[test]
    fn test_sudo_wrapping_idempotent() {
        let mut aliases = HashMap::new();
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);

        // First application
        let result1 = aka.replace("sudo ls").unwrap();

        // Second application should be idempotent
        let result2 = aka.replace(&result1.trim()).unwrap();

        // Should not double-wrap
        assert!(!result2.contains("$(which $(which"));
    }
}
```

#### 3.2 Integration Tests
**Location:** `tests/sudo_wrapping_tests.rs` (new file)

```rust
use aka_lib::*;
use std::collections::HashMap;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_sudo_wrapping_scenarios() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza"
    space: true
    global: false
  sys:
    value: "systemctl"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test cases
    let test_cases = vec![
        // (input, expected_pattern, should_contain_which)
        ("sudo ls", "sudo", true),  // May wrap if eza is user-only
        ("sudo systemctl", "sudo systemctl", false),  // Should not wrap system commands
        ("sudo $(which ls)", "sudo $(which ls)", false),  // Should not double-wrap
        ("sudo ls -la", "sudo", false),  // Complex commands should not wrap the base
    ];

    for (input, expected_start, should_contain_which) in test_cases {
        let result = aka.replace(input).expect("Should process command");
        assert!(result.starts_with(expected_start),
               "Input '{}' should start with '{}', got '{}'", input, expected_start, result);

        if should_contain_which {
            // May or may not contain $(which) depending on system state
            println!("Input '{}' -> '{}'", input, result);
        } else {
            assert!(!result.contains("$(which $(which"),
                   "Input '{}' should not double-wrap: '{}'", input, result);
        }
    }
}

#[test]
fn test_sudo_wrapping_edge_cases() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create minimal config
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    fs::write(&config_file, "aliases: {}").expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Edge cases
    let edge_cases = vec![
        "sudo",  // Just sudo
        "sudo ",  // Sudo with space
        "sudo ''",  // Sudo with empty string
        "sudo $(which $(which ls))",  // Already double-wrapped
    ];

    for input in edge_cases {
        let result = aka.replace(input).expect("Should handle edge case");
        // Should not crash and should not create malformed commands
        assert!(!result.contains("$(which $(which $(which"),
               "Should not triple-wrap: '{}'", result);
    }
}
```

### Phase 4: Performance Optimization

#### 4.1 Caching Strategy
For performance, we can cache command availability checks:

```rust
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static! {
    static ref COMMAND_CACHE: Mutex<HashMap<String, bool>> = Mutex::new(HashMap::new());
}

fn is_user_only_command_cached(command: &str) -> bool {
    // Check cache first
    if let Ok(cache) = COMMAND_CACHE.lock() {
        if let Some(&cached_result) = cache.get(command) {
            return cached_result;
        }
    }

    // Compute result
    let result = is_user_only_command(command);

    // Cache result
    if let Ok(mut cache) = COMMAND_CACHE.lock() {
        cache.insert(command.to_string(), result);
    }

    result
}
```

#### 4.2 Timeout Protection
Add timeouts to prevent hanging on slow systems:

```rust
use std::time::Duration;

fn is_command_available_to_root_with_timeout(command: &str) -> bool {
    std::process::Command::new("timeout")
        .args(&["1", "sudo", "-n", "which", command])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
```

## Migration Strategy

### Backward Compatibility

The changes are designed to be backward compatible:

1. **Existing Behavior**: Commands that currently work will continue to work
2. **Improved Behavior**: Commands that currently break will be fixed
3. **No Breaking Changes**: No changes to public API or configuration format

### Rollout Plan

1. **Phase 1**: Implement detection functions with comprehensive tests
2. **Phase 2**: Update main processing logic with feature flag
3. **Phase 3**: Enable by default after thorough testing
4. **Phase 4**: Add performance optimizations

### Risk Mitigation

1. **Feature Flag**: Allow disabling new behavior via environment variable
2. **Extensive Testing**: Cover all edge cases and system variations
3. **Gradual Rollout**: Test on development systems first
4. **Monitoring**: Add logging to track wrapping decisions

## Success Metrics

### Functional Metrics
- [ ] No recursive wrapping (`$(which $(which ...))`)
- [ ] System commands not unnecessarily wrapped
- [ ] User-only commands properly wrapped when needed
- [ ] Complex commands handled correctly

### Performance Metrics
- [ ] No significant performance degradation
- [ ] Caching reduces redundant system calls
- [ ] Timeout protection prevents hanging

### User Experience Metrics
- [ ] Reduced terminal command failures
- [ ] Cleaner command output
- [ ] Maintained functionality for legitimate use cases

## Implementation Timeline

| Phase | Duration | Deliverables |
|-------|----------|-------------|
| Phase 1 | 2 days | Detection functions + unit tests |
| Phase 2 | 1 day | Main logic update + integration tests |
| Phase 3 | 1 day | Performance optimizations |
| Phase 4 | 1 day | Documentation + final testing |

**Total Estimated Time**: 5 days

## Conclusion

This design provides a robust, intelligent solution to the sudo wrapping problem without relying on any hardcoded lists or static databases. The runtime detection approach ensures the system adapts to any environment while maintaining high performance through caching and timeout protection.

The solution is backward compatible, thoroughly tested, and designed for easy maintenance and future enhancement.
