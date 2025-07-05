# AKA Daemon Architecture Specification

**Version:** 1.0  
**Date:** 2025-01-05  
**Status:** Design Phase  

## Executive Summary

This document specifies the architecture for transforming the current process-per-query aka system into a high-performance daemon-based architecture. The daemon will eliminate the performance bottleneck of spawning new processes and parsing configuration files on every keystroke while maintaining full compatibility with the existing ZLE integration.

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

#### 2. aka-client (Thin Client)

**Responsibilities:**
- Accept command-line arguments (maintaining CLI compatibility)
- Establish connection to daemon
- Send requests and receive responses
- Handle daemon unavailability gracefully

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

### 2. Client Process (aka-client)

#### 2.1 Client Implementation

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

#### 2.2 Fallback Strategy

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

#### 2.3 Separate Binary Architecture

The system uses two separate binaries for clear separation of concerns:

**aka-daemon Binary:**
```rust
// src/bin/aka-daemon.rs
#[derive(Parser)]
struct DaemonOpts {
    #[clap(long, help = "Run in foreground (don't daemonize)")]
    foreground: bool,
    
    #[clap(long, help = "Stop running daemon")]
    stop: bool,
    
    #[clap(long, help = "Restart daemon")]
    restart: bool,
    
    #[clap(long, help = "Show daemon status")]
    status: bool,
    
    #[clap(short, long)]
    config: Option<PathBuf>,
}

fn main() {
    let opts = DaemonOpts::parse();
    
    match opts {
        DaemonOpts { stop: true, .. } => stop_daemon(),
        DaemonOpts { restart: true, .. } => restart_daemon(),
        DaemonOpts { status: true, .. } => show_status(),
        _ => run_daemon(&opts),
    }
}
```

**aka Client Binary:**
```rust
// src/bin/aka.rs
#[derive(Parser)]
struct AkaOpts {
    #[clap(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    command: Option<Command>,
}

fn main() {
    let opts = AkaOpts::parse();
    let client = AkaClient::new();
    
    // Always try daemon first, fallback to legacy implementation
    match execute_with_fallback(&client, &opts) {
        Ok(code) => exit(code),
        Err(e) => {
            eprintln!("Error: {}", e);
            exit(1);
        }
    }
}
```

### 3. ZLE Integration Updates

#### 3.1 Enhanced Health Check

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

#### 3.2 Performance-Optimized Expansion

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
│   │   ├── aka.rs               # Main client binary
│   │   └── aka-daemon.rs        # Daemon binary
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
│   │   ├── lifecycle.rs         # Start/stop/restart logic
│   │   ├── watcher.rs           # File system watching
│   │   ├── handler.rs           # Request processing
│   │   └── health.rs            # Health monitoring
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

## Migration Strategy

### Deployment Approach
1. **Soft Launch**: Daemon optional, fallback to current system
2. **Testing Period**: Extensive testing with fallback safety net
3. **Gradual Rollout**: Enable daemon by default
4. **Legacy Support**: Maintain fallback indefinitely

### User Experience
- **Zero Disruption**: Users see only performance improvements
- **Manual Daemon Start**: Users explicitly start `aka-daemon` when ready
- **Automatic Fallback**: Seamless degradation when daemon unavailable
- **Clear Binary Separation**: `aka` for queries, `aka-daemon` for daemon management

### Binary Usage Examples
```bash
# Start daemon
aka-daemon                    # Foreground mode
aka-daemon --foreground       # Explicit foreground
nohup aka-daemon &           # Background manually

# Daemon management
aka-daemon --stop            # Stop daemon
aka-daemon --restart         # Restart daemon
aka-daemon --status          # Check status

# Client usage (unchanged)
aka query "ls -la"           # Query (tries daemon first)
aka ls                       # List aliases
aka ls -g                    # List global aliases
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

**Document Status:** Ready for Implementation  
**Next Steps:** Begin Phase 1 implementation  
**Review Schedule:** Weekly during implementation phases 