# Refactor: Move Pure Logic from Binaries to Library

## Overview

This document outlines a plan to refactor the `aka` and `aka-daemon` binaries to move pure logic functions into the `aka-lib` library. This will improve testability and increase code coverage.

### Current State

| Module | Coverage | Notes |
|--------|----------|-------|
| Library code | ~90% | Well-tested |
| `aka.rs` | 6% | 42/698 lines |
| `aka-daemon.rs` | 0% | 0/363 lines |
| **Overall** | 46.72% | Binaries drag down average |

### Goal

Move enough pure logic to the library to achieve **80% overall coverage** while maintaining clean separation of concerns.

---

## Analysis: What Can Be Moved

### 1. DaemonError Module (High Priority)

**Location:** `src/bin/aka.rs:27-109`

**Components:**
- `DaemonError` enum (27-39)
- `Display` impl (41-56)
- `std::error::Error` impl (58)
- `should_retry_daemon_error()` (60-73)
- `categorize_daemon_error()` (75-85)
- `validate_socket_path()` (87-109)

**Why:** These are pure logic functions with no I/O dependencies. They can be fully unit tested.

**Proposed location:** `src/daemon_client.rs` (new module) or `src/error.rs`

**Estimated lines:** ~80 lines → ~80 lines covered

### 2. Daemon Client Configuration (Medium Priority)

**Location:** `src/bin/aka.rs:19-25`

**Components:**
```rust
const DAEMON_CONNECTION_TIMEOUT_MS: u64 = 100;
const DAEMON_READ_TIMEOUT_MS: u64 = 200;
const DAEMON_WRITE_TIMEOUT_MS: u64 = 50;
const DAEMON_TOTAL_TIMEOUT_MS: u64 = 300;
const DAEMON_RETRY_DELAY_MS: u64 = 50;
const DAEMON_MAX_RETRIES: u32 = 1;
```

**Why:** Configuration constants should be in the library for consistency and testing.

**Proposed location:** `src/daemon_client.rs` as `DaemonClientConfig` struct

**Estimated lines:** ~10 lines

### 3. Service Configuration Generation (Medium Priority)

**Location:** `src/bin/aka.rs:497-597`

**Components:**
- `install_systemd_service()` - generates systemd unit file content
- `install_launchd_service()` - generates launchd plist content

**Refactor approach:** Extract content generation as pure functions:

```rust
// In library
pub fn generate_systemd_unit(daemon_path: &Path, cargo_bin: &Path) -> String { ... }
pub fn generate_launchd_plist(daemon_path: &Path, log_path: &Path) -> String { ... }
```

**Why:** The string generation is pure; only the file writing is I/O.

**Estimated lines:** ~60 lines

### 4. Request/Response Processing Logic (Medium Priority)

**Location:** `src/bin/aka-daemon.rs:267-450`

**Components:**
- Request validation logic
- Response formatting
- Version compatibility checking

**Refactor approach:** Extract as trait or helper functions:

```rust
// In library
pub trait RequestProcessor {
    fn validate_version(&self, client_version: &str, daemon_version: &str) -> Result<()>;
    fn format_health_status(alias_count: usize, is_synced: bool) -> String;
}
```

**Estimated lines:** ~40 lines

### 5. Daemon Status Logic (Low Priority)

**Location:** `src/bin/aka.rs:301-356`

**Components:**
- `get_daemon_status_emoji()` - partially testable
- `check_daemon_process_simple()` - requires pgrep

**Refactor approach:** Extract status determination logic:

```rust
// In library
pub enum DaemonStatus {
    Running { synced: bool },
    StaleSocket,
    NotRunning,
    Unknown,
}

pub fn interpret_daemon_status(socket_exists: bool, process_running: bool, health_status: Option<&str>) -> DaemonStatus { ... }
pub fn status_to_emoji(status: &DaemonStatus) -> &'static str { ... }
```

**Estimated lines:** ~30 lines

---

## What CANNOT Be Moved

These components must remain in the binaries:

### Binary-Specific Code

1. **Main functions** - Entry points must stay in binaries
2. **CLI argument parsing** - Clap derives are binary-specific
3. **Actual I/O operations:**
   - Socket connections (`UnixStream::connect`)
   - File system operations (`std::fs::write`)
   - Process spawning (`std::process::Command`)
4. **Signal handling** - OS-specific signal registration

### Why Keep These in Binaries

- **Separation of concerns:** Library should be platform-agnostic logic
- **Testing complexity:** I/O operations require integration tests
- **Build dependencies:** Service management requires OS-specific features

---

## Implementation Plan

### Phase 1: Create `daemon_client` Module (Week 1)

**Tasks:**
1. Create `src/daemon_client.rs` in library
2. Move `DaemonError` enum and implementations
3. Move `should_retry_daemon_error()`
4. Move `categorize_daemon_error()`
5. Move `validate_socket_path()`
6. Add comprehensive tests
7. Update `aka.rs` to import from library

**Expected coverage gain:** +3-4%

**File structure:**
```
src/
├── lib.rs
├── daemon_client.rs  ← NEW
│   ├── DaemonError
│   ├── DaemonClientConfig
│   ├── error handling functions
│   └── tests
├── protocol.rs
└── ...
```

### Phase 2: Extract Service Configuration (Week 2)

**Tasks:**
1. Create `src/service.rs` in library
2. Move `generate_systemd_unit()` as pure function
3. Move `generate_launchd_plist()` as pure function
4. Add tests for generated content
5. Keep file writing in binary

**Expected coverage gain:** +2-3%

### Phase 3: Daemon Status Module (Week 3)

**Tasks:**
1. Create `DaemonStatus` enum in library
2. Extract `interpret_daemon_status()` logic
3. Extract `status_to_emoji()` function
4. Add tests

**Expected coverage gain:** +1-2%

### Phase 4: Request Processing Helpers (Week 4)

**Tasks:**
1. Add request validation helpers to `protocol.rs`
2. Add response formatting helpers
3. Add version compatibility utilities
4. Add tests

**Expected coverage gain:** +1-2%

---

## Code Examples

### Example: DaemonError Module

```rust
// src/daemon_client.rs

use std::path::PathBuf;
use thiserror::Error;

/// Configuration for daemon client timeouts
#[derive(Debug, Clone)]
pub struct DaemonClientConfig {
    pub connection_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub retry_delay_ms: u64,
    pub max_retries: u32,
}

impl Default for DaemonClientConfig {
    fn default() -> Self {
        Self {
            connection_timeout_ms: 100,
            read_timeout_ms: 200,
            write_timeout_ms: 50,
            total_timeout_ms: 300,
            retry_delay_ms: 50,
            max_retries: 1,
        }
    }
}

/// Errors that can occur during daemon communication
#[derive(Debug, Clone, Error)]
pub enum DaemonError {
    #[error("Daemon connection timeout")]
    ConnectionTimeout,

    #[error("Daemon read timeout")]
    ReadTimeout,

    #[error("Daemon write timeout")]
    WriteTimeout,

    #[error("Daemon connection refused")]
    ConnectionRefused,

    #[error("Daemon socket not found")]
    SocketNotFound,

    #[error("Daemon socket permission denied")]
    SocketPermissionDenied,

    #[error("Daemon protocol error: {0}")]
    ProtocolError(String),

    #[error("Daemon is shutting down")]
    DaemonShutdown,

    #[error("Total daemon operation timeout")]
    TotalOperationTimeout,

    #[error("Unknown daemon error: {0}")]
    UnknownError(String),
}

/// Determines if a daemon error should trigger a retry
pub fn should_retry(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::ConnectionTimeout | DaemonError::ConnectionRefused
    )
}

/// Categorizes an I/O error into a DaemonError
pub fn categorize_io_error(error: &std::io::Error) -> DaemonError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::TimedOut => DaemonError::ConnectionTimeout,
        ErrorKind::ConnectionRefused => DaemonError::ConnectionRefused,
        ErrorKind::NotFound => DaemonError::SocketNotFound,
        ErrorKind::PermissionDenied => DaemonError::SocketPermissionDenied,
        ErrorKind::WouldBlock => DaemonError::ReadTimeout,
        _ => DaemonError::UnknownError(error.to_string()),
    }
}

/// Validates that a socket path exists and is actually a socket
pub fn validate_socket_path(socket_path: &PathBuf) -> Result<(), DaemonError> {
    if !socket_path.exists() {
        return Err(DaemonError::SocketNotFound);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let metadata = std::fs::metadata(socket_path)
            .map_err(|e| categorize_io_error(&e))?;

        if !metadata.file_type().is_socket() {
            return Err(DaemonError::SocketNotFound);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_daemon_client_config_default() {
        let config = DaemonClientConfig::default();
        assert_eq!(config.connection_timeout_ms, 100);
        assert_eq!(config.max_retries, 1);
    }

    #[test]
    fn test_should_retry_connection_timeout() {
        assert!(should_retry(&DaemonError::ConnectionTimeout));
    }

    #[test]
    fn test_should_retry_connection_refused() {
        assert!(should_retry(&DaemonError::ConnectionRefused));
    }

    #[test]
    fn test_should_not_retry_read_timeout() {
        assert!(!should_retry(&DaemonError::ReadTimeout));
    }

    #[test]
    fn test_should_not_retry_socket_not_found() {
        assert!(!should_retry(&DaemonError::SocketNotFound));
    }

    #[test]
    fn test_categorize_io_error_timed_out() {
        let error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        assert!(matches!(categorize_io_error(&error), DaemonError::ConnectionTimeout));
    }

    #[test]
    fn test_categorize_io_error_connection_refused() {
        let error = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert!(matches!(categorize_io_error(&error), DaemonError::ConnectionRefused));
    }

    #[test]
    fn test_validate_socket_path_not_found() {
        let path = PathBuf::from("/nonexistent/socket.sock");
        assert!(matches!(validate_socket_path(&path), Err(DaemonError::SocketNotFound)));
    }

    #[test]
    fn test_validate_socket_path_regular_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("regular_file");
        std::fs::write(&file_path, "test").unwrap();

        let result = validate_socket_path(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_daemon_error_display() {
        assert_eq!(DaemonError::ConnectionTimeout.to_string(), "Daemon connection timeout");
        assert_eq!(DaemonError::SocketNotFound.to_string(), "Daemon socket not found");
        assert_eq!(
            DaemonError::ProtocolError("test".to_string()).to_string(),
            "Daemon protocol error: test"
        );
    }
}
```

### Example: Service Configuration

```rust
// src/service.rs

use std::path::Path;

/// Generates systemd user service unit file content
pub fn generate_systemd_unit(daemon_path: &Path, cargo_bin_path: &Path) -> String {
    format!(
        r#"[Unit]
Description=AKA Alias Daemon
After=graphical-session.target

[Service]
Type=simple
ExecStart={}
Restart=always
RestartSec=5
Environment=PATH={}:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
"#,
        daemon_path.display(),
        cargo_bin_path.display()
    )
}

/// Generates macOS launchd plist content
pub fn generate_launchd_plist(daemon_path: &Path, log_path: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.scottidler.aka-daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>{}</string>
    <key>StandardOutPath</key>
    <string>{}</string>
</dict>
</plist>
"#,
        daemon_path.display(),
        log_path.display(),
        log_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_generate_systemd_unit_contains_description() {
        let unit = generate_systemd_unit(
            &PathBuf::from("/usr/bin/aka-daemon"),
            &PathBuf::from("/home/user/.cargo/bin"),
        );
        assert!(unit.contains("Description=AKA Alias Daemon"));
    }

    #[test]
    fn test_generate_systemd_unit_contains_exec_start() {
        let unit = generate_systemd_unit(
            &PathBuf::from("/usr/bin/aka-daemon"),
            &PathBuf::from("/home/user/.cargo/bin"),
        );
        assert!(unit.contains("ExecStart=/usr/bin/aka-daemon"));
    }

    #[test]
    fn test_generate_launchd_plist_contains_label() {
        let plist = generate_launchd_plist(
            &PathBuf::from("/usr/bin/aka-daemon"),
            &PathBuf::from("/var/log/aka-daemon.log"),
        );
        assert!(plist.contains("com.scottidler.aka-daemon"));
    }

    #[test]
    fn test_generate_launchd_plist_is_valid_xml() {
        let plist = generate_launchd_plist(
            &PathBuf::from("/usr/bin/aka-daemon"),
            &PathBuf::from("/var/log/aka-daemon.log"),
        );
        assert!(plist.starts_with("<?xml version=\"1.0\""));
        assert!(plist.contains("</plist>"));
    }
}
```

---

## Expected Coverage After Refactor

| Module | Before | After | Change |
|--------|--------|-------|--------|
| `src/daemon_client.rs` | N/A | ~95% | +80 lines |
| `src/service.rs` | N/A | ~95% | +60 lines |
| `src/bin/aka.rs` | 6% | ~8% | Reduced code |
| `src/bin/aka-daemon.rs` | 0% | ~2% | Reduced code |
| **Overall** | 46.72% | **~55-60%** | +8-13% |

**Note:** To reach 80% overall coverage would require either:
1. Additional refactoring phases
2. Integration tests for the binaries
3. Lowering the coverage threshold

---

## Migration Guide

### Step 1: Add New Module to Library

```rust
// src/lib.rs
pub mod daemon_client;
pub mod service;
```

### Step 2: Update Binary Imports

```rust
// src/bin/aka.rs
use aka_lib::daemon_client::{DaemonError, should_retry, categorize_io_error, validate_socket_path};
use aka_lib::service::{generate_systemd_unit, generate_launchd_plist};
```

### Step 3: Remove Duplicate Code

Delete the moved code from the binary files.

### Step 4: Run Tests

```bash
cargo test
otto cov
```

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Breaking changes to binary | High | Incremental migration with feature flags |
| Version compatibility | Medium | Keep daemon protocol backward compatible |
| Performance regression | Low | Profile critical paths before/after |

---

## Success Criteria

1. ✅ All existing tests pass
2. ✅ Coverage increases by at least 5%
3. ✅ No performance regression in critical paths
4. ✅ Binary code is cleaner and focused on I/O orchestration
5. ✅ Library provides well-documented, testable components

---

## Timeline

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| Phase 1: daemon_client | 1 week | DaemonError module in library |
| Phase 2: service | 1 week | Service config generation |
| Phase 3: status | 1 week | Daemon status module |
| Phase 4: protocol | 1 week | Request/response helpers |
| **Total** | **4 weeks** | **~55-60% coverage** |

---

## References

- Current coverage report: `otto cov`
- Binary source: `src/bin/aka.rs`, `src/bin/aka-daemon.rs`
- Library source: `src/lib.rs`


