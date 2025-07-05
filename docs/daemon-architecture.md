# AKA Daemon Architecture Specification

**Version:** 1.0
**Date:** 2025-01-05
**Status:** Design Phase

## Executive Summary

This document specifies the architecture for transforming the current process-per-query aka system into a high-performance daemon-based architecture. The daemon will eliminate the performance bottleneck of spawning new processes and parsing configuration files on every keystroke while maintaining full compatibility with the existing ZLE integration.

### Key Benefits

**Performance Improvements:**
- **Sub-millisecond response times**: Daemon eliminates process startup overhead
- **Persistent configuration cache**: No config parsing on every keystroke
- **Reduced CPU usage**: Eliminates repeated process creation/destruction
- **Lower memory footprint**: Single persistent process vs. multiple short-lived processes

**Enhanced User Experience:**
- **Unified CLI**: All functionality through single `aka` command
- **Cross-platform service management**: Automatic integration with systemd/launchd
- **Zero-disruption installation**: `cargo install --path .` continues to work
- **Graceful fallback**: Automatic degradation when daemon unavailable

**Operational Benefits:**
- **Proper service management**: Integration with OS service managers
- **Automatic startup**: Daemon starts on login/boot
- **Health monitoring**: Built-in daemon status and diagnostics
- **Easy maintenance**: Simple start/stop/restart commands

**Developer Benefits:**
- **Future-proof architecture**: Foundation for advanced features
- **Maintainable codebase**: Clear separation of concerns
- **Comprehensive testing**: Extensive test coverage for reliability
- **Backward compatibility**: No breaking changes to existing functionality

## Current Architecture Problems

### Performance Issues
- **Process Creation Overhead**: Each spacebar press spawns a new `aka` process (~10-50ms startup time)
- **Config Parsing Overhead**: YAML configuration parsed from disk on every query (~2-5ms)
- **Memory Allocation**: Fresh memory allocation for data structures on each invocation
- **Log Spam**: Excessive logging of config loading operations

### Architectural Limitations
- No state persistence between queries
- No opportunity for intelligent caching or optimization
- Resource waste from repeated initialization
- Difficult to implement advanced features (completion caching, usage analytics, etc.)

## Proposed Daemon Architecture

### High-Level Design

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   ZLE Widgets   │    │   aka-client     │    │   aka-daemon    │
│                 │    │   (thin client)  │    │  (persistent)   │
├─────────────────┤    ├──────────────────┤    ├─────────────────┤
│ expand-aka-space│───▶│ query request    │───▶│ config cache    │
│ expand-aka-line │    │ via Unix socket  │    │ alias engine    │
│ aka-search      │    │                  │    │ file watcher    │
└─────────────────┘    └──────────────────┘    └─────────────────┘
```

### Component Architecture

#### 1. aka-daemon (Server Process)

**Responsibilities:**
- Maintain parsed configuration in memory
- Watch configuration files for changes
- Process alias expansion requests
- Handle client connections via Unix domain sockets
- Manage daemon lifecycle (start/stop/restart)

**Key Features:**
- **In-Memory Config Cache**: Parsed `Spec` struct kept in memory
- **File System Watcher**: Automatic config reload on file changes
- **Connection Pool**: Handle multiple concurrent ZLE sessions
- **Graceful Shutdown**: Proper cleanup on termination signals
- **Health Monitoring**: Self-diagnostic capabilities

#### 2. aka CLI (Single Entry Point)

**Responsibilities:**
- Accept all command-line arguments (maintaining CLI compatibility)
- Route daemon management commands to service manager
- Route query/list commands to daemon (with fallback)
- Handle daemon unavailability gracefully

**Unified CLI Interface:**
- All functionality accessible through single `aka` command
- Daemon management: `aka daemon --install/--start/--stop/--status`
- Regular usage: `aka ls`, `aka query "command"`

**Fallback Behavior:**
- If daemon unavailable, fall back to current process-per-query model
- Ensure zero disruption to user experience

#### 3. ZLE Integration (Enhanced)

**Responsibilities:**
- Detect daemon availability
- Route requests through client when daemon available
- Maintain backward compatibility with current implementation

## Detailed Component Specifications

### 1. Daemon Process (aka-daemon)

#### 1.1 Process Management

```rust
// Main daemon structure
pub struct AkaDaemon {
    config: Arc<RwLock<Spec>>,
    config_path: PathBuf,
    socket_path: PathBuf,
    file_watcher: RecommendedWatcher,
    shutdown_signal: Arc<AtomicBool>,
    client_connections: Arc<Mutex<Vec<UnixStream>>>,
}

impl AkaDaemon {
    pub fn new(config_path: PathBuf) -> Result<Self>;
    pub fn start(&mut self) -> Result<()>;
    pub fn shutdown(&mut self) -> Result<()>;
    pub fn reload_config(&mut self) -> Result<()>;
}
```

#### 1.2 Configuration Management

```rust
// Thread-safe configuration management
impl AkaDaemon {
    fn watch_config_file(&mut self) -> Result<()> {
        // Use notify crate to watch config file changes
        // On change: reload config, update in-memory cache
        // Log config reloads at info level
    }

    fn reload_config_if_changed(&mut self) -> Result<bool> {
        // Check file modification time
        // Only reload if actually changed
        // Return true if reloaded
    }
}
```

#### 1.3 Client Communication Protocol

**Unix Domain Socket Protocol:**

```rust
// Request/Response protocol
#[derive(Serialize, Deserialize)]
pub enum DaemonRequest {
    Query { cmdline: String, eol: bool },
    List { global: bool, patterns: Vec<String> },
    Health,
    Shutdown,
}

#[derive(Serialize, Deserialize)]
pub enum DaemonResponse {
    QueryResult { result: String },
    ListResult { aliases: Vec<String> },
    HealthResult { status: String, uptime: Duration },
    Error { message: String },
}
```

**Socket Location:**
- Primary: `$XDG_RUNTIME_DIR/aka/daemon.sock`
- Fallback: `$HOME/.local/share/aka/daemon.sock`

#### 1.4 Daemon Lifecycle

**Startup Process:**
1. Parse command line arguments
2. Load and validate configuration
3. Create Unix domain socket
4. Start file watcher
5. Enter main event loop
6. Handle client connections

**Shutdown Process:**
1. Receive SIGTERM/SIGINT
2. Set shutdown flag
3. Close listening socket
4. Wait for active connections to complete
5. Clean up resources
6. Exit gracefully

#### 1.5 Error Handling & Recovery

```rust
impl AkaDaemon {
    fn handle_client_error(&mut self, error: &Error) {
        // Log error, continue serving other clients
        // Implement circuit breaker for repeated failures
    }

    fn handle_config_error(&mut self, error: &Error) {
        // Keep serving with last known good config
        // Log error prominently
        // Attempt reload on next file change
    }
}
```

### 2. Unified CLI Process (aka)

#### 2.1 CLI Command Structure

```rust
#[derive(Parser)]
struct AkaOpts {
    #[clap(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Parser)]
enum Command {
    #[clap(name = "ls", about = "list aka aliases")]
    List(ListOpts),

    #[clap(name = "query", about = "query for aka substitutions")]
    Query(QueryOpts),

    #[clap(name = "daemon", about = "manage aka daemon")]
    Daemon(DaemonOpts),

    #[clap(name = "__complete_aliases", hide = true)]
    CompleteAliases,

    #[clap(name = "__health_check", hide = true)]
    HealthCheck,
}

#[derive(Parser)]
struct DaemonOpts {
    #[clap(long, help = "Install system service")]
    install: bool,

    #[clap(long, help = "Uninstall system service")]
    uninstall: bool,

    #[clap(long, help = "Start daemon")]
    start: bool,

    #[clap(long, help = "Stop daemon")]
    stop: bool,

    #[clap(long, help = "Restart daemon")]
    restart: bool,

    #[clap(long, help = "Show daemon status")]
    status: bool,
}
```

#### 2.2 Client Implementation

```rust
pub struct AkaClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl AkaClient {
    pub fn new() -> Self;
    pub fn query(&self, cmdline: &str, eol: bool) -> Result<String>;
    pub fn list(&self, global: bool, patterns: Vec<String>) -> Result<Vec<String>>;
    pub fn health(&self) -> Result<String>;

    fn connect(&self) -> Result<UnixStream>;
    fn send_request(&self, request: DaemonRequest) -> Result<DaemonResponse>;
}
```

#### 2.3 Fallback Strategy

```rust
impl AkaClient {
    pub fn query_with_fallback(&self, cmdline: &str, eol: bool) -> Result<String> {
        match self.query(cmdline, eol) {
            Ok(result) => Ok(result),
            Err(_) => {
                // Daemon unavailable, fall back to current implementation
                let aka = AKA::new(eol, &None)?;
                aka.replace(cmdline)
            }
        }
    }
}
```

#### 2.4 System Service Management

The CLI includes comprehensive cross-platform daemon management through system service integration:

```rust
// Cross-platform service management
pub enum ServiceManager {
    Systemd(SystemdManager),
    Launchd(LaunchdManager),
    Unsupported,
}

impl ServiceManager {
    pub fn new() -> Self {
        if cfg!(target_os = "linux") && Self::has_systemd() {
            ServiceManager::Systemd(SystemdManager::new())
        } else if cfg!(target_os = "macos") {
            ServiceManager::Launchd(LaunchdManager::new())
        } else {
            ServiceManager::Unsupported
        }
    }

    pub fn install_service(&self) -> Result<()>;
    pub fn uninstall_service(&self) -> Result<()>;
    pub fn start_service(&self) -> Result<()>;
    pub fn stop_service(&self) -> Result<()>;
    pub fn restart_service(&self) -> Result<()>;
    pub fn status(&self) -> Result<()>;
}
```

**Linux (systemd) Integration:**
- Creates user service file: `~/.config/systemd/user/aka-daemon.service`
- Integrates with `systemctl --user` commands
- Automatic startup on login
- Proper service monitoring and restart

**macOS (launchd) Integration:**
- Creates LaunchAgent plist: `~/Library/LaunchAgents/com.scottidler.aka-daemon.plist`
- Integrates with `launchctl` commands
- Automatic startup on login
- Proper service monitoring and restart

**Unsupported Platforms:**
- Graceful degradation with manual daemon management
- Clear error messages with fallback instructions

#### 2.5 Binary Architecture

The system uses a single binary with multiple entry points:

**aka Binary (Main CLI):**
```rust
// src/bin/aka.rs
fn main() {
    let opts = AkaOpts::parse();

    match &opts.command {
        Some(Command::Daemon(daemon_opts)) => {
            // Handle daemon management commands
            exit(match execute_daemon_command(daemon_opts) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            });
        }
        _ => {
            // Handle regular aka commands (query, list, etc.)
            let client = AkaClient::new();
            exit(match execute_with_fallback(&client, &opts) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            });
        }
    }
}

fn execute_daemon_command(daemon_opts: &DaemonOpts) -> Result<i32> {
    let service_manager = ServiceManager::new();

    match daemon_opts {
        DaemonOpts { install: true, .. } => service_manager.install_service()?,
        DaemonOpts { uninstall: true, .. } => service_manager.uninstall_service()?,
        DaemonOpts { start: true, .. } => service_manager.start_service()?,
        DaemonOpts { stop: true, .. } => service_manager.stop_service()?,
        DaemonOpts { restart: true, .. } => service_manager.restart_service()?,
        DaemonOpts { status: true, .. } => service_manager.status()?,
        _ => {
            println!("Usage: aka daemon [--install|--uninstall|--start|--stop|--restart|--status]");
            return Ok(1);
        }
    }

    Ok(0)
}
```

**aka-daemon Binary (Daemon Process):**
```rust
// src/bin/aka-daemon.rs
#[derive(Parser)]
struct DaemonOpts {
    #[clap(long, help = "Run in foreground (don't daemonize)")]
    foreground: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,
}

fn main() {
    let opts = DaemonOpts::parse();

    // Start daemon process
    match run_daemon(&opts) {
        Ok(_) => exit(0),
        Err(e) => {
            eprintln!("Daemon error: {}", e);
            exit(1);
        }
    }
}
```

### 3. System Service Management Implementation

#### 3.1 Linux (systemd) Service Manager

```rust
// src/daemon/service_manager.rs
pub struct SystemdManager {
    service_name: String,
    service_file_path: PathBuf,
}

impl SystemdManager {
    pub fn new() -> Self {
        let service_name = "aka-daemon.service".to_string();
        let service_file_path = dirs::config_dir()
            .unwrap()
            .join("systemd/user")
            .join(&service_name);

        Self { service_name, service_file_path }
    }

    pub fn install_service(&self) -> Result<()> {
        // Create systemd user directory
        let service_dir = self.service_file_path.parent().unwrap();
        fs::create_dir_all(service_dir)?;

        // Get aka-daemon binary path
        let daemon_path = self.get_daemon_binary_path()?;

        // Create service file content
        let service_content = format!(
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
            dirs::home_dir().unwrap().join(".cargo/bin").display()
        );

        // Write service file
        fs::write(&self.service_file_path, service_content)?;

        // Reload systemd and enable service
        Command::new("systemctl").args(&["--user", "daemon-reload"]).status()?;
        Command::new("systemctl").args(&["--user", "enable", &self.service_name]).status()?;

        println!("✅ SystemD service installed and enabled");
        Ok(())
    }

    pub fn start_service(&self) -> Result<()> {
        let output = Command::new("systemctl")
            .args(&["--user", "start", &self.service_name])
            .output()?;

        if output.status.success() {
            println!("✅ Daemon started via SystemD");
        } else {
            return Err(eyre!("Failed to start daemon: {}",
                String::from_utf8_lossy(&output.stderr)));
        }
        Ok(())
    }

    // ... other methods (stop, restart, status, uninstall)
}
```

#### 3.2 macOS (launchd) Service Manager

```rust
pub struct LaunchdManager {
    service_name: String,
    plist_path: PathBuf,
}

impl LaunchdManager {
    pub fn new() -> Self {
        let service_name = "com.scottidler.aka-daemon".to_string();
        let plist_path = dirs::home_dir()
            .unwrap()
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", service_name));

        Self { service_name, plist_path }
    }

    pub fn install_service(&self) -> Result<()> {
        // Create LaunchAgents directory
        let plist_dir = self.plist_path.parent().unwrap();
        fs::create_dir_all(plist_dir)?;

        // Get aka-daemon binary path
        let daemon_path = self.get_daemon_binary_path()?;

        // Create plist content
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>{}/Library/Logs/aka-daemon.log</string>
    <key>StandardOutPath</key>
    <string>{}/Library/Logs/aka-daemon.log</string>
</dict>
</plist>
"#,
            self.service_name,
            daemon_path.display(),
            dirs::home_dir().unwrap().display(),
            dirs::home_dir().unwrap().display()
        );

        // Write plist file and load service
        fs::write(&self.plist_path, plist_content)?;
        Command::new("launchctl")
            .args(&["load", self.plist_path.to_str().unwrap()])
            .status()?;

        println!("✅ LaunchAgent installed and loaded");
        Ok(())
    }

    // ... other methods (start, stop, restart, status, uninstall)
}
```

#### 3.3 Cross-Platform Service Management

```rust
pub enum ServiceManager {
    Systemd(SystemdManager),
    Launchd(LaunchdManager),
    Unsupported,
}

impl ServiceManager {
    pub fn new() -> Self {
        if cfg!(target_os = "linux") && Self::has_systemd() {
            ServiceManager::Systemd(SystemdManager::new())
        } else if cfg!(target_os = "macos") {
            ServiceManager::Launchd(LaunchdManager::new())
        } else {
            ServiceManager::Unsupported
        }
    }

    fn has_systemd() -> bool {
        Command::new("systemctl")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    pub fn install_service(&self) -> Result<()> {
        match self {
            ServiceManager::Systemd(manager) => manager.install_service(),
            ServiceManager::Launchd(manager) => manager.install_service(),
            ServiceManager::Unsupported => {
                println!("❌ Service management not supported on this platform");
                println!("   You can still run the daemon manually:");
                println!("   aka-daemon &");
                Ok(())
            }
        }
    }

    // ... delegate other methods to appropriate manager
}
```

### 4. ZLE Integration Updates

#### 4.1 Enhanced Health Check

```zsh
# Enhanced health check with daemon awareness
aka_health_check() {
    # Check for killswitch first
    if [ -f ~/aka-killswitch ]; then
        return 1
    fi

    # Check if daemon is available
    if aka-daemon --status 2>/dev/null; then
        # Daemon available - fast path
        return 0
    else
        # Daemon unavailable - fall back to current health check
        aka __health_check 2>/dev/null
        return $?
    fi
}
```

#### 4.2 Performance-Optimized Expansion

```zsh
# Optimized expansion with daemon support
expand-aka-space() {
    aka_health_check
    if [ $? -eq 0 ]; then
        log "expand-aka-space: BUFFER=$BUFFER"

        # Use daemon-aware client
        OUTPUT=$(aka query "$BUFFER" 2>/dev/null)
        RC=$?

        log "expand-aka-space: OUTPUT=$OUTPUT RC=$RC"

        if [ $RC -eq 0 ] && [ -n "$OUTPUT" ]; then
            BUFFER="$OUTPUT"
            log "expand-aka-space: CURSOR=$CURSOR"
            CURSOR=$(expr length "$BUFFER")
            log "expand-aka-space: CURSOR(after assignment)=$CURSOR"
        else
            zle self-insert
        fi
    else
        zle self-insert
    fi
}
```

## Project Structure

### Binary Organization
```
aka/
├── Cargo.toml                   # Multiple binary configuration
├── src/
│   ├── lib.rs                   # Shared library exports
│   │
│   ├── bin/
│   │   ├── aka.rs               # Main CLI binary (unified entry point)
│   │   └── aka-daemon.rs        # Daemon process binary
│   │
│   ├── shared/                  # Code shared between binaries
│   │   ├── mod.rs
│   │   ├── protocol.rs          # IPC protocol definitions
│   │   ├── config.rs            # Configuration parsing
│   │   ├── types.rs             # Shared data structures
│   │   └── error.rs             # Error types
│   │
│   ├── daemon/                  # Daemon-specific code
│   │   ├── mod.rs
│   │   ├── server.rs            # Unix socket server
│   │   ├── lifecycle.rs         # Daemon process management
│   │   ├── watcher.rs           # File system watching
│   │   ├── handler.rs           # Request processing
│   │   ├── health.rs            # Health monitoring
│   │   └── service_manager.rs   # Cross-platform service management
│   │
│   └── client/                  # Client-specific code
│       ├── mod.rs
│       ├── connection.rs        # Socket connection management
│       ├── fallback.rs          # Fallback to current system
│       └── cli.rs               # Command line interface
│
├── bin/                         # Shell integration (unchanged)
│   ├── aka.zsh
│   └── aka-loader.zsh
│
├── docs/
│   └── daemon-architecture.md
│
└── tests/
    ├── integration/
    │   ├── daemon_lifecycle.rs
    │   ├── client_daemon_comm.rs
    │   ├── service_management.rs
    │   └── fallback_behavior.rs
    └── performance/
        └── benchmarks.rs
```

### Cargo.toml Configuration
```toml
[package]
name = "aka"
version = "0.3.21"
edition = "2021"

[[bin]]
name = "aka"
path = "src/bin/aka.rs"

[[bin]]
name = "aka-daemon"
path = "src/bin/aka-daemon.rs"

[lib]
name = "aka_lib"
path = "src/lib.rs"

[dependencies]
# ... existing dependencies
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
notify = "6.0"
```

## Implementation Plan

### Phase 1: Project Restructure & Shared Code
**Duration:** 1 week

**Deliverables:**
- [ ] Refactor current implementation for shared library use
- [ ] Create shared types in `src/shared/`
- [ ] Set up multiple binary configuration
- [ ] Ensure current functionality still works
- [ ] Create basic project structure

**Key Files:**
- `src/lib.rs` - Library exports
- `src/shared/` - Shared type definitions
- `src/main.rs` - Refactor existing implementation
- `Cargo.toml` - Multiple binary configuration

### Phase 2: Core Daemon Infrastructure
**Duration:** 2-3 weeks

**Deliverables:**
- [ ] Basic daemon process with Unix socket server
- [ ] Simple request/response protocol
- [ ] Configuration loading and caching
- [ ] Daemon lifecycle management (start/stop)
- [ ] Basic client implementation with fallback

**Key Files:**
- `src/bin/aka-daemon.rs` - Daemon binary entry point
- `src/bin/aka.rs` - Client binary entry point
- `src/daemon/server.rs` - Socket server implementation
- `src/daemon/lifecycle.rs` - Daemon lifecycle management
- `src/client/connection.rs` - Client connection logic
- `src/client/fallback.rs` - Fallback to current implementation
- `src/shared/protocol.rs` - Communication protocol

### Phase 3: File Watching & Auto-Reload
**Duration:** 1 week

**Deliverables:**
- [ ] File system watcher integration
- [ ] Automatic config reload on changes
- [ ] Error handling for config reload failures
- [ ] Logging for config changes

**Dependencies:**
- `notify` crate for file system watching

### Phase 4: ZLE Integration & Testing
**Duration:** 1-2 weeks

**Deliverables:**
- [ ] Updated ZLE scripts with daemon awareness
- [ ] Comprehensive test suite
- [ ] Performance benchmarking
- [ ] Documentation updates

**Key Files:**
- `bin/aka.zsh` - Updated ZLE integration
- `tests/daemon_integration.rs` - Integration tests
- `benchmarks/performance.rs` - Performance tests

### Phase 5: Advanced Features
**Duration:** 1-2 weeks

**Deliverables:**
- [ ] Health monitoring and diagnostics
- [ ] Usage analytics (optional)
- [ ] Advanced caching strategies
- [ ] Multi-session support improvements

## Performance Expectations

### Current Performance (Process-per-Query)
- **Cold Start**: 10-50ms per query
- **Config Parse**: 2-5ms per query
- **Memory Usage**: 2-5MB per process
- **CPU Usage**: High burst per keystroke

### Expected Daemon Performance
- **Query Response**: 0.1-0.5ms per query
- **Memory Usage**: 5-10MB persistent
- **CPU Usage**: Low, consistent
- **Startup Time**: 50-100ms (one-time)

### Performance Targets
- **Query Latency**: < 1ms (99th percentile)
- **Memory Footprint**: < 20MB
- **Config Reload**: < 10ms
- **Startup Time**: < 200ms

## Security Considerations

### Socket Security
- Unix domain sockets with appropriate permissions (600)
- Socket location in user-specific directory
- No network exposure

### Process Security
- Run as user process (not system daemon)
- No elevated privileges required
- Graceful handling of permission errors

### Configuration Security
- Validate configuration files before loading
- Sanitize user input in aliases
- Prevent arbitrary code execution

## Monitoring & Diagnostics

### Health Endpoints
```rust
// Health check responses
pub struct HealthStatus {
    pub uptime: Duration,
    pub config_last_loaded: SystemTime,
    pub total_queries: u64,
    pub active_connections: usize,
    pub memory_usage: usize,
}
```

### Logging Strategy
- **Info Level**: Daemon start/stop, config reloads
- **Debug Level**: Individual query processing
- **Error Level**: Connection failures, config errors
- **Structured Logging**: JSON format for analysis

### Metrics Collection
- Query response times
- Config reload frequency
- Error rates
- Memory usage trends

## Backward Compatibility

### CLI Compatibility
- All existing `aka` commands continue to work
- New daemon-specific commands added
- Graceful fallback when daemon unavailable

### ZLE Compatibility
- Existing ZLE integration continues to work
- Performance improvements transparent to users
- No changes to key bindings or behavior

### Configuration Compatibility
- Existing `aka.yml` files work unchanged
- No breaking changes to configuration format
- New optional daemon-specific settings

## Installation and Deployment

### Cargo Install Compatibility

The daemon architecture maintains full compatibility with `cargo install --path .`:

```bash
# Current installation (still works)
cargo install --path .
# Result: Installs both binaries:
#   ~/.cargo/bin/aka        (main CLI)
#   ~/.cargo/bin/aka-daemon (daemon process)
```

**Post-Installation Behavior:**
- **Without daemon setup**: Falls back to current process-per-query model
- **With daemon setup**: Uses high-performance daemon architecture
- **Zero breaking changes**: Existing workflows continue to work

### Service Setup Options

#### Option 1: Automatic Service Setup (Recommended)
```bash
# Install and set up in one step
cargo install --path .
aka daemon --install
aka daemon --start
```

#### Option 2: Manual Daemon Management
```bash
# Install binaries only
cargo install --path .

# Start daemon manually when needed
aka-daemon &

# Or use aka CLI for manual control
aka daemon --start  # Will fail gracefully on unsupported platforms
```

#### Option 3: No Daemon (Current Behavior)
```bash
# Install binaries only
cargo install --path .

# Use aka normally - automatically falls back to current implementation
aka ls
aka query "some command"
```

### Cross-Platform Service Integration

#### Linux (systemd)
```bash
# Service file location: ~/.config/systemd/user/aka-daemon.service
aka daemon --install    # Install service
aka daemon --start      # Start via systemctl --user start aka-daemon
aka daemon --status     # Check via systemctl --user status aka-daemon

# Service auto-starts on login
systemctl --user enable aka-daemon
```

#### macOS (launchd)
```bash
# Plist location: ~/Library/LaunchAgents/com.scottidler.aka-daemon.plist
aka daemon --install    # Install LaunchAgent
aka daemon --start      # Start via launchctl start com.scottidler.aka-daemon
aka daemon --status     # Check via launchctl list com.scottidler.aka-daemon

# Service auto-starts on login (RunAtLoad=true)
```

#### Other Platforms
```bash
# Graceful degradation
aka daemon --install    # Shows manual instructions
aka daemon --start      # Falls back to manual process management

# Manual daemon management still available
aka-daemon &
```

## Migration Strategy

### Deployment Approach
1. **Soft Launch**: Daemon optional, fallback to current system
2. **Testing Period**: Extensive testing with fallback safety net
3. **Gradual Rollout**: Users opt-in to daemon via `aka daemon --install`
4. **Legacy Support**: Maintain fallback indefinitely

### User Experience
- **Zero Disruption**: Users see only performance improvements
- **Opt-in Enhancement**: Users choose when to enable daemon
- **Automatic Fallback**: Seamless degradation when daemon unavailable
- **Unified CLI**: All functionality through single `aka` command

### CLI Usage Examples

#### Installation and Setup
```bash
# 1. Install binaries (same as current)
cargo install --path .

# 2. Set up system service (one-time setup)
aka daemon --install

# 3. Start daemon
aka daemon --start

# 4. Verify daemon is running
aka daemon --status
```

#### Daily Usage
```bash
# Daemon management
aka daemon --status          # Check daemon status
aka daemon --restart         # Restart daemon if needed
aka daemon --stop            # Stop daemon
aka daemon --uninstall       # Remove system service

# Regular aka usage (unchanged, but now fast!)
aka query "ls -la"           # Query (tries daemon first, falls back if needed)
aka ls                       # List aliases
aka ls -g                    # List global aliases
```

#### Manual Daemon Management (fallback)
```bash
# For unsupported platforms or manual control
aka-daemon                   # Run daemon in foreground
aka-daemon --foreground      # Explicit foreground mode
nohup aka-daemon &          # Background manually
```

## Testing Strategy

### Unit Tests
- Daemon lifecycle management
- Protocol serialization/deserialization
- Configuration parsing and validation
- Client connection handling

### Integration Tests
- End-to-end query processing
- Config file watching and reloading
- ZLE integration testing
- Fallback behavior validation

### Performance Tests
- Query latency benchmarks
- Memory usage profiling
- Concurrent connection handling
- Long-running stability tests

### Stress Tests
- High-frequency query bursts
- Config reload under load
- Resource exhaustion scenarios
- Error recovery testing

## Risk Assessment

### Technical Risks
- **Socket Communication Failures**: Mitigated by fallback strategy
- **Memory Leaks**: Addressed through comprehensive testing
- **Config Reload Race Conditions**: Handled with proper locking
- **Daemon Crashes**: Auto-restart and fallback mechanisms

### Operational Risks
- **User Adoption**: Mitigated by transparent performance improvements
- **Debugging Complexity**: Addressed with comprehensive logging
- **Platform Compatibility**: Tested across target platforms

## Success Metrics

### Performance Metrics
- [ ] Query latency < 1ms (99th percentile)
- [ ] Memory usage < 20MB
- [ ] Zero user-visible disruption during migration

### Quality Metrics
- [ ] Test coverage > 90%
- [ ] Zero critical bugs in production
- [ ] Successful fallback in 100% of daemon failure scenarios

### User Experience Metrics
- [ ] Improved responsiveness (subjective feedback)
- [ ] Reduced battery usage on laptops
- [ ] Elimination of log spam

## Future Enhancements

### Advanced Features
- **Completion Caching**: Cache shell completion results
- **Usage Analytics**: Track alias usage patterns
- **Smart Suggestions**: Suggest aliases based on usage
- **Multi-Config Support**: Support for project-specific configs

### Performance Optimizations
- **Lazy Loading**: Load config sections on demand
- **Compression**: Compress cached data structures
- **Batch Processing**: Handle multiple queries in single request

### Integration Improvements
- **Shell Integration**: Support for bash, fish, other shells
- **Editor Integration**: Support for vim, emacs plugins
- **System Integration**: systemd user service support

---

## Implementation Appendices

### Appendix A: IPC Protocol Specification

#### A.1 Message Framing

All messages over the Unix domain socket use length-prefixed framing:

```rust
// Message format: [4-byte length][JSON payload]
// Length is little-endian u32 indicating JSON payload size

pub struct MessageFrame {
    length: u32,      // Payload length in bytes
    payload: Vec<u8>, // JSON-serialized DaemonRequest/DaemonResponse
}

impl MessageFrame {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&self.length.to_le_bytes());
        buffer.extend_from_slice(&self.payload);
        buffer
    }

    pub fn deserialize(buffer: &[u8]) -> Result<Self> {
        if buffer.len() < 4 {
            return Err(eyre!("Insufficient data for message length"));
        }

        let length = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);

        if buffer.len() < 4 + length as usize {
            return Err(eyre!("Insufficient data for message payload"));
        }

        let payload = buffer[4..4 + length as usize].to_vec();
        Ok(MessageFrame { length, payload })
    }
}
```

#### A.2 Protocol Versioning

```rust
#[derive(Serialize, Deserialize)]
pub struct ProtocolHeader {
    version: u16,     // Protocol version (current: 1)
    request_id: u64,  // Unique request identifier
    timestamp: u64,   // Unix timestamp
}

#[derive(Serialize, Deserialize)]
pub struct DaemonMessage {
    header: ProtocolHeader,
    payload: DaemonRequest,
}

#[derive(Serialize, Deserialize)]
pub struct DaemonReply {
    header: ProtocolHeader,
    payload: DaemonResponse,
}
```

#### A.3 Extended Request/Response Types

```rust
#[derive(Serialize, Deserialize)]
pub enum DaemonRequest {
    // Query operations
    Query { cmdline: String, eol: bool },
    List { global: bool, patterns: Vec<String> },

    // Health and diagnostics
    Health,
    Ping,
    Stats,

    // Configuration operations
    ReloadConfig,
    ValidateConfig { config_path: Option<PathBuf> },

    // Administrative operations
    Shutdown { graceful: bool },
}

#[derive(Serialize, Deserialize)]
pub enum DaemonResponse {
    // Query results
    QueryResult { result: String, cached: bool },
    ListResult { aliases: Vec<AliasInfo> },

    // Health responses
    HealthResult {
        status: HealthStatus,
        uptime: Duration,
        memory_usage: u64,
        config_last_loaded: SystemTime,
    },
    PingResult { timestamp: SystemTime },
    StatsResult { stats: DaemonStats },

    // Configuration responses
    ConfigReloaded {
        success: bool,
        aliases_count: usize,
        reload_time: Duration,
    },
    ConfigValidated { valid: bool, errors: Vec<String> },

    // Administrative responses
    ShutdownAck,

    // Error responses
    Error {
        code: ErrorCode,
        message: String,
        details: Option<serde_json::Value>,
    },
}

#[derive(Serialize, Deserialize)]
pub struct AliasInfo {
    name: String,
    value: String,
    global: bool,
    usage_count: u64,
}

#[derive(Serialize, Deserialize)]
pub struct DaemonStats {
    total_queries: u64,
    cache_hits: u64,
    cache_misses: u64,
    config_reloads: u64,
    active_connections: u32,
    average_response_time_ms: f64,
}

#[derive(Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}

#[derive(Serialize, Deserialize)]
pub enum ErrorCode {
    // Protocol errors
    InvalidRequest = 1000,
    UnsupportedVersion = 1001,
    MalformedMessage = 1002,

    // Configuration errors
    ConfigNotFound = 2000,
    ConfigInvalid = 2001,
    ConfigPermissionDenied = 2002,

    // Query errors
    QueryFailed = 3000,
    AliasNotFound = 3001,

    // System errors
    InternalError = 4000,
    ResourceExhausted = 4001,
    ServiceUnavailable = 4002,
}
```

#### A.4 Connection Management

```rust
pub struct ConnectionManager {
    socket_path: PathBuf,
    timeout: Duration,
    retry_attempts: u32,
    retry_delay: Duration,
}

impl ConnectionManager {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            timeout: Duration::from_secs(5),
            retry_attempts: 3,
            retry_delay: Duration::from_millis(100),
        }
    }

    pub fn send_request(&self, request: DaemonRequest) -> Result<DaemonResponse> {
        let mut last_error = None;

        for attempt in 0..self.retry_attempts {
            match self.try_send_request(&request) {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < self.retry_attempts - 1 {
                        std::thread::sleep(self.retry_delay);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| eyre!("Connection failed")))
    }

    fn try_send_request(&self, request: &DaemonRequest) -> Result<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        // Send request
        let message = DaemonMessage {
            header: ProtocolHeader {
                version: 1,
                request_id: self.generate_request_id(),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)?
                    .as_secs(),
            },
            payload: request.clone(),
        };

        let json_data = serde_json::to_vec(&message)?;
        let frame = MessageFrame {
            length: json_data.len() as u32,
            payload: json_data,
        };

        stream.write_all(&frame.serialize())?;

        // Read response
        let mut length_buf = [0u8; 4];
        stream.read_exact(&mut length_buf)?;
        let length = u32::from_le_bytes(length_buf);

        let mut payload_buf = vec![0u8; length as usize];
        stream.read_exact(&mut payload_buf)?;

        let reply: DaemonReply = serde_json::from_slice(&payload_buf)?;
        Ok(reply.payload)
    }

    fn generate_request_id(&self) -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }
}
```

### Appendix B: Daemon Implementation Details

#### B.1 Daemon Process Management

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::signal;

pub struct AkaDaemon {
    config: Arc<RwLock<Spec>>,
    config_path: PathBuf,
    socket_path: PathBuf,
    file_watcher: Option<RecommendedWatcher>,
    shutdown_signal: Arc<AtomicBool>,
    stats: Arc<RwLock<DaemonStats>>,
    start_time: SystemTime,
}

impl AkaDaemon {
    pub fn new(config_path: PathBuf) -> Result<Self> {
        let socket_path = Self::determine_socket_path()?;

        // Ensure socket directory exists
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Remove stale socket file
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        Ok(Self {
            config: Arc::new(RwLock::new(Spec::default())),
            config_path,
            socket_path,
            file_watcher: None,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(DaemonStats::default())),
            start_time: SystemTime::now(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Load initial configuration
        self.load_config()?;

        // Set up file watcher
        self.setup_file_watcher()?;

        // Set up signal handlers
        self.setup_signal_handlers().await?;

        // Start Unix socket server
        let listener = UnixListener::bind(&self.socket_path)?;

        // Set socket permissions (readable/writable by owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&self.socket_path)?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            std::fs::set_permissions(&self.socket_path, permissions)?;
        }

        info!("Daemon started, listening on {:?}", self.socket_path);

        // Main event loop
        while !self.shutdown_signal.load(Ordering::Relaxed) {
            tokio::select! {
                // Handle incoming connections
                Ok((stream, _)) = listener.accept() => {
                    let config = Arc::clone(&self.config);
                    let stats = Arc::clone(&self.stats);
                    let shutdown = Arc::clone(&self.shutdown_signal);

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, config, stats, shutdown).await {
                            error!("Connection error: {}", e);
                        }
                    });
                }

                // Handle shutdown signal
                _ = self.wait_for_shutdown() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }

        self.cleanup().await?;
        Ok(())
    }

    fn determine_socket_path() -> Result<PathBuf> {
        // Try XDG_RUNTIME_DIR first
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            let path = PathBuf::from(runtime_dir).join("aka").join("daemon.sock");
            return Ok(path);
        }

        // Fallback to ~/.local/share/aka/
        let home_dir = dirs::home_dir()
            .ok_or_else(|| eyre!("Could not determine home directory"))?;

        Ok(home_dir.join(".local/share/aka/daemon.sock"))
    }
}
```

#### B.2 Configuration Hot Reloading

```rust
impl AkaDaemon {
    fn load_config(&self) -> Result<()> {
        let loader = Loader::new();
        let spec = loader.load(&self.config_path)?;

        // Validate configuration
        Self::validate_config(&spec)?;

        // Update in-memory config
        {
            let mut config_guard = self.config.write().unwrap();
            *config_guard = spec;
        }

        info!("Configuration loaded successfully from {:?}", self.config_path);
        Ok(())
    }

    fn validate_config(spec: &Spec) -> Result<()> {
        // Validate aliases
        for (name, alias) in &spec.aliases {
            if name.is_empty() {
                return Err(eyre!("Empty alias name not allowed"));
            }

            if alias.value.is_empty() {
                return Err(eyre!("Empty alias value not allowed for '{}'", name));
            }

            // Check for circular references
            if alias.value.contains(name) {
                warn!("Potential circular reference in alias '{}'", name);
            }
        }

        // Validate lookups
        for (table_name, lookup) in &spec.lookups {
            if table_name.is_empty() {
                return Err(eyre!("Empty lookup table name not allowed"));
            }

            if lookup.is_empty() {
                return Err(eyre!("Empty lookup table '{}' not allowed", table_name));
            }
        }

        Ok(())
    }
}
```

### Appendix C: Service Manager Implementation

#### C.1 Enhanced Binary Path Detection

```rust
impl SystemdManager {
    fn get_daemon_binary_path(&self) -> Result<PathBuf> {
        // Strategy 1: Check if aka-daemon is in PATH
        if let Ok(output) = Command::new("which").arg("aka-daemon").output() {
            if output.status.success() {
                let path_str = String::from_utf8(output.stdout)?.trim().to_string();
                let path = PathBuf::from(path_str);

                // Verify the binary exists and is executable
                if path.exists() && Self::is_executable(&path)? {
                    return Ok(path);
                }
            }
        }

        // Strategy 2: Check cargo install location
        if let Some(home_dir) = dirs::home_dir() {
            let cargo_bin = home_dir.join(".cargo/bin/aka-daemon");
            if cargo_bin.exists() && Self::is_executable(&cargo_bin)? {
                return Ok(cargo_bin);
            }
        }

        // Strategy 3: Check common system locations
        let system_paths = [
            "/usr/local/bin/aka-daemon",
            "/usr/bin/aka-daemon",
            "/opt/aka/bin/aka-daemon",
        ];

        for path_str in &system_paths {
            let path = PathBuf::from(path_str);
            if path.exists() && Self::is_executable(&path)? {
                return Ok(path);
            }
        }

        // Strategy 4: Check relative to current executable
        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(parent) = current_exe.parent() {
                let relative_daemon = parent.join("aka-daemon");
                if relative_daemon.exists() && Self::is_executable(&relative_daemon)? {
                    return Ok(relative_daemon);
                }
            }
        }

        Err(eyre!("Could not find aka-daemon binary. Please ensure it's installed and in PATH."))
    }

    fn is_executable(path: &PathBuf) -> Result<bool> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(path)?;
            let permissions = metadata.permissions();
            Ok(permissions.mode() & 0o111 != 0)
        }

        #[cfg(not(unix))]
        {
            // On non-Unix systems, just check if file exists
            Ok(path.exists())
        }
    }
}
```

### Appendix D: Error Handling Specifications

#### D.1 Error Code Taxonomy

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AkaError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<ErrorDetails>,
    pub timestamp: SystemTime,
    pub context: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorDetails {
    ConfigError {
        file_path: PathBuf,
        line_number: Option<usize>,
        column: Option<usize>,
    },
    NetworkError {
        socket_path: PathBuf,
        connection_attempt: u32,
    },
    ServiceError {
        service_name: String,
        platform: String,
        command: String,
    },
    SystemError {
        errno: Option<i32>,
        syscall: Option<String>,
    },
}

impl AkaError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            timestamp: SystemTime::now(),
            context: Vec::new(),
        }
    }

    pub fn user_message(&self) -> String {
        match self.code {
            ErrorCode::ConfigNotFound => {
                format!("Configuration file not found. Please create ~/.config/aka/aka.yml")
            }
            ErrorCode::ConfigInvalid => {
                if let Some(ErrorDetails::ConfigError { file_path, line_number, .. }) = &self.details {
                    format!("Invalid configuration in {:?} at line {}: {}",
                        file_path,
                        line_number.unwrap_or(0),
                        self.message)
                } else {
                    format!("Invalid configuration: {}", self.message)
                }
            }
            ErrorCode::ServiceUnavailable => {
                "Daemon is not running. Start it with: aka daemon --start".to_string()
            }
            _ => self.message.clone(),
        }
    }
}
```

#### D.2 Recovery Procedures

```rust
pub struct ErrorRecovery;

impl ErrorRecovery {
    pub fn handle_daemon_connection_error(error: &AkaError) -> Result<DaemonResponse> {
        match error.code {
            ErrorCode::ServiceUnavailable => {
                // Try to start daemon automatically
                info!("Daemon unavailable, attempting to start...");

                let service_manager = ServiceManager::new();
                if let Err(start_error) = service_manager.start_service() {
                    warn!("Could not start daemon: {}", start_error);
                    return Self::fallback_to_legacy_mode();
                }

                // Wait a moment for daemon to start
                std::thread::sleep(Duration::from_millis(500));

                // Retry connection
                Err(eyre!("Daemon started, please retry"))
            }

            ErrorCode::ConfigInvalid => {
                // Try to use last known good configuration
                warn!("Invalid configuration, falling back to legacy mode");
                Self::fallback_to_legacy_mode()
            }

            _ => {
                // For other errors, fall back to legacy mode
                Self::fallback_to_legacy_mode()
            }
        }
    }

    fn fallback_to_legacy_mode() -> Result<DaemonResponse> {
        info!("Falling back to legacy process-per-query mode");
        // This would integrate with the existing AKA implementation
        Err(eyre!("Fallback to legacy mode"))
    }
}
```

---

**Document Status:** Implementation Ready
**Next Steps:** Begin Phase 1 implementation with comprehensive specifications
**Review Schedule:** Weekly during implementation phases
