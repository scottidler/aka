use eyre::{eyre, Result};
use log::{info, debug, warn};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::{Instant, Duration};
use xxhash_rust::xxh3::xxh3_64;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

pub mod cfg;
pub mod protocol;
pub mod error;

use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

// Re-export for binaries
pub use cfg::alias::Alias as AliasType;
pub use cfg::loader::Loader as ConfigLoader;
pub use cfg::spec::Spec as ConfigSpec;

// Re-export protocol types for shared use
pub use protocol::{DaemonRequest, DaemonResponse};

// Re-export error types for enhanced error handling
pub use error::{AkaError, ErrorContext, ValidationError, enhance_error};

// Daemon error handling types and constants
#[derive(Debug, Clone)]
pub enum DaemonError {
    ConnectionTimeout,
    ReadTimeout,
    WriteTimeout,
    ConnectionRefused,
    SocketNotFound,
    SocketPermissionDenied,
    ProtocolError(String),
    DaemonShutdown,
    TotalOperationTimeout,
    UnknownError(String),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::ConnectionTimeout => write!(f, "Daemon connection timeout"),
            DaemonError::ReadTimeout => write!(f, "Daemon read timeout"),
            DaemonError::WriteTimeout => write!(f, "Daemon write timeout"),
            DaemonError::ConnectionRefused => write!(f, "Daemon connection refused"),
            DaemonError::SocketNotFound => write!(f, "Daemon socket not found"),
            DaemonError::SocketPermissionDenied => write!(f, "Daemon socket permission denied"),
            DaemonError::ProtocolError(msg) => write!(f, "Daemon protocol error: {}", msg),
            DaemonError::DaemonShutdown => write!(f, "Daemon is shutting down"),
            DaemonError::TotalOperationTimeout => write!(f, "Total daemon operation timeout"),
            DaemonError::UnknownError(msg) => write!(f, "Unknown daemon error: {}", msg),
        }
    }
}

impl std::error::Error for DaemonError {}

// Aggressive timeout constants for CLI performance
pub const DAEMON_CONNECTION_TIMEOUT_MS: u64 = 100;  // 100ms to connect
pub const DAEMON_READ_TIMEOUT_MS: u64 = 200;        // 200ms to read response
pub const DAEMON_WRITE_TIMEOUT_MS: u64 = 50;        // 50ms to write request
pub const DAEMON_TOTAL_TIMEOUT_MS: u64 = 300;       // 300ms total operation limit
pub const DAEMON_RETRY_DELAY_MS: u64 = 50;          // 50ms between retries
pub const DAEMON_MAX_RETRIES: u32 = 1;              // Only 1 retry attempt

// Timeout utility functions
pub fn should_retry_daemon_error(error: &DaemonError) -> bool {
    match error {
        DaemonError::ConnectionTimeout => true,
        DaemonError::ConnectionRefused => true,
        DaemonError::ReadTimeout => false,  // Don't retry read timeouts
        DaemonError::WriteTimeout => false, // Don't retry write timeouts
        DaemonError::SocketNotFound => false,
        DaemonError::SocketPermissionDenied => false,
        DaemonError::ProtocolError(_) => false,
        DaemonError::DaemonShutdown => false,
        DaemonError::TotalOperationTimeout => false,
        DaemonError::UnknownError(_) => false,
    }
}

pub fn categorize_daemon_error(error: &std::io::Error) -> DaemonError {
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

pub fn validate_socket_path(socket_path: &PathBuf) -> Result<(), DaemonError> {
    if !socket_path.exists() {
        return Err(DaemonError::SocketNotFound);
    }

    // Check if it's actually a socket (not a regular file)
    match std::fs::metadata(socket_path) {
        Ok(metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                if !metadata.file_type().is_socket() {
                    return Err(DaemonError::SocketNotFound);
                }
            }
        }
        Err(e) => {
            return Err(categorize_daemon_error(&e));
        }
    }

    Ok(())
}

// JSON cache structure for aliases with usage counts
#[derive(Serialize, Deserialize)]
struct AliasCache {
    aliases: HashMap<String, Alias>,
}

// Check if benchmark mode is enabled
fn is_benchmark_mode() -> bool {
    std::env::var("AKA_BENCHMARK").is_ok() ||
    std::env::var("AKA_TIMING").is_ok() ||
    std::env::var("AKA_DEBUG_TIMING").is_ok()
}

// Timing instrumentation framework
#[derive(Debug, Clone)]
pub struct TimingData {
    pub total_duration: Duration,
    pub config_load_duration: Option<Duration>,
    pub ipc_duration: Option<Duration>,
    pub processing_duration: Duration,
    pub mode: ProcessingMode,
    pub timestamp: std::time::SystemTime,
}

#[derive(Debug, Clone)]
pub struct TimingCollector {
    start_time: Instant,
    config_start: Option<Instant>,
    ipc_start: Option<Instant>,
    processing_start: Option<Instant>,
    mode: ProcessingMode,
}

impl TimingCollector {
    pub fn new(mode: ProcessingMode) -> Self {
        TimingCollector {
            start_time: Instant::now(),
            config_start: None,
            ipc_start: None,
            processing_start: None,
            mode,
        }
    }

    pub fn start_config_load(&mut self) {
        self.config_start = Some(Instant::now());
    }

    pub fn end_config_load(&mut self) -> Option<Duration> {
        self.config_start.map(|start| start.elapsed())
    }

    pub fn start_ipc(&mut self) {
        self.ipc_start = Some(Instant::now());
    }

    pub fn end_ipc(&mut self) -> Option<Duration> {
        self.ipc_start.map(|start| start.elapsed())
    }

    pub fn start_processing(&mut self) {
        self.processing_start = Some(Instant::now());
    }

    pub fn end_processing(&mut self) -> Duration {
        self.processing_start.map(|start| start.elapsed()).unwrap_or_default()
    }

    pub fn finalize(self) -> TimingData {
        TimingData {
            total_duration: self.start_time.elapsed(),
            config_load_duration: self.config_start.map(|start| start.elapsed()),
            ipc_duration: self.ipc_start.map(|start| start.elapsed()),
            processing_duration: self.processing_start.map(|start| start.elapsed()).unwrap_or_default(),
            mode: self.mode,
            timestamp: std::time::SystemTime::now(),
        }
    }
}

impl TimingData {
    pub fn log_detailed(&self) {
        // Only log detailed timing if benchmark mode is enabled
        if !is_benchmark_mode() {
            return;
        }

        let emoji = match self.mode {
            ProcessingMode::Daemon => "👹",
            ProcessingMode::Direct => "📥",
        };

        debug!("{} === TIMING BREAKDOWN ({:?}) ===", emoji, self.mode);
        debug!("  🎯 Total execution: {:.3}ms", self.total_duration.as_secs_f64() * 1000.0);

        if let Some(config_duration) = self.config_load_duration {
            debug!("  📋 Config loading: {:.3}ms ({:.1}%)",
                config_duration.as_secs_f64() * 1000.0,
                (config_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
            );
        }

        if let Some(ipc_duration) = self.ipc_duration {
            debug!("  🔌 IPC communication: {:.3}ms ({:.1}%)",
                ipc_duration.as_secs_f64() * 1000.0,
                (ipc_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
            );
        }

        debug!("  ⚙️  Processing: {:.3}ms ({:.1}%)",
            self.processing_duration.as_secs_f64() * 1000.0,
            (self.processing_duration.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
        );

        // Calculate overhead
        let accounted = self.config_load_duration.unwrap_or_default() +
                       self.ipc_duration.unwrap_or_default() +
                       self.processing_duration;
        let overhead = self.total_duration.saturating_sub(accounted);
        debug!("  🏗️  Overhead: {:.3}ms ({:.1}%)",
            overhead.as_secs_f64() * 1000.0,
            (overhead.as_secs_f64() / self.total_duration.as_secs_f64()) * 100.0
        );
    }

    pub fn to_csv_line(&self) -> String {
        format!("{},{:?},{:.3},{:.3},{:.3},{:.3}",
            self.timestamp.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
            self.mode,
            self.total_duration.as_secs_f64() * 1000.0,
            self.config_load_duration.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            self.ipc_duration.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            self.processing_duration.as_secs_f64() * 1000.0
        )
    }
}

fn parse_csv_line(line: &str) -> Result<TimingData> {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() != 6 {
        return Err(eyre!("Invalid CSV line format"));
    }

    let timestamp_ms: u64 = parts[0].parse().map_err(|_| eyre!("Invalid timestamp"))?;
    let mode = match parts[1] {
        "Daemon" => ProcessingMode::Daemon,
        "Direct" => ProcessingMode::Direct,
        _ => return Err(eyre!("Invalid processing mode")),
    };

    let total_ms: f64 = parts[2].parse().map_err(|_| eyre!("Invalid total duration"))?;
    let config_ms: f64 = parts[3].parse().map_err(|_| eyre!("Invalid config duration"))?;
    let ipc_ms: f64 = parts[4].parse().map_err(|_| eyre!("Invalid IPC duration"))?;
    let processing_ms: f64 = parts[5].parse().map_err(|_| eyre!("Invalid processing duration"))?;

    Ok(TimingData {
        total_duration: Duration::from_secs_f64(total_ms / 1000.0),
        config_load_duration: if config_ms > 0.0 { Some(Duration::from_secs_f64(config_ms / 1000.0)) } else { None },
        ipc_duration: if ipc_ms > 0.0 { Some(Duration::from_secs_f64(ipc_ms / 1000.0)) } else { None },
        processing_duration: Duration::from_secs_f64(processing_ms / 1000.0),
        mode,
        timestamp: std::time::UNIX_EPOCH + Duration::from_millis(timestamp_ms),
    })
}

// Global timing storage for analysis
use std::sync::Mutex;
use lazy_static::lazy_static;

lazy_static! {
    static ref TIMING_LOG: Mutex<Vec<TimingData>> = Mutex::new(Vec::new());
}

pub fn log_timing(timing: TimingData) {
    // Only log detailed breakdown if benchmark mode is enabled
    if is_benchmark_mode() {
        timing.log_detailed();
    }

    // Always store in memory for CLI commands (minimal overhead)
    if let Ok(mut log) = TIMING_LOG.lock() {
        log.push(timing.clone());

        // Keep only last 1000 entries to prevent memory bloat
        let len = log.len();
        if len > 1000 {
            log.drain(0..len - 1000);
        }
    }

    // Only write to CSV file if benchmark mode is enabled
    if is_benchmark_mode() {
        if let Ok(timing_file_path) = get_timing_file_path() {
            // Ensure directory exists
            if let Some(parent) = timing_file_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let csv_line = timing.to_csv_line();
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(timing_file_path) {
                use std::io::Write;
                let _ = writeln!(file, "{}", csv_line);
            }
        }
    }
}

pub fn export_timing_csv() -> Result<String> {
    let mut csv = String::from("timestamp,mode,total_ms,config_ms,ipc_ms,processing_ms\n");

    // Load from persistent file if it exists
    if let Ok(timing_file_path) = get_timing_file_path() {
        if let Ok(content) = std::fs::read_to_string(timing_file_path) {
            for line in content.lines() {
                if !line.trim().is_empty() {
                    csv.push_str(line);
                    csv.push('\n');
                }
            }
        }
    }

    // Also include current session data
    if let Ok(log) = TIMING_LOG.lock() {
        for timing in log.iter() {
            csv.push_str(&timing.to_csv_line());
            csv.push('\n');
        }
    }

    Ok(csv)
}

pub fn get_timing_summary() -> Result<(Duration, Duration, usize, usize)> {
    let mut all_timings = Vec::new();

    // Load from persistent file if it exists
    if let Ok(timing_file_path) = get_timing_file_path() {
        if let Ok(content) = std::fs::read_to_string(timing_file_path) {
            for line in content.lines() {
                if let Ok(timing) = parse_csv_line(line) {
                    all_timings.push(timing);
                }
            }
        }
    }

    // Also include current session data
    if let Ok(log) = TIMING_LOG.lock() {
        all_timings.extend(log.iter().cloned());
    }

    let daemon_timings: Vec<_> = all_timings.iter().filter(|t| matches!(t.mode, ProcessingMode::Daemon)).collect();
    let direct_timings: Vec<_> = all_timings.iter().filter(|t| matches!(t.mode, ProcessingMode::Direct)).collect();

    let daemon_avg = if !daemon_timings.is_empty() {
        daemon_timings.iter().map(|t| t.total_duration).sum::<Duration>() / daemon_timings.len() as u32
    } else {
        Duration::default()
    };

    let direct_avg = if !direct_timings.is_empty() {
        direct_timings.iter().map(|t| t.total_duration).sum::<Duration>() / direct_timings.len() as u32
    } else {
        Duration::default()
    };

    Ok((daemon_avg, direct_avg, daemon_timings.len(), direct_timings.len()))
}

pub fn get_timing_file_path() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("timing_data.csv"))
}

pub fn get_config_path(home_dir: &PathBuf) -> Result<PathBuf> {
    let config_dirs = [
        home_dir.join(".config").join("aka"),
        home_dir.clone(),
    ];

    let config_files = ["aka.yml", "aka.yaml", ".aka.yml", ".aka.yaml"];
    let mut attempted_paths = Vec::new();

    for config_dir in &config_dirs {
        for config_file in &config_files {
            let path = config_dir.join(config_file);
            attempted_paths.push(path.clone());
            if path.exists() {
                return Ok(path);
            }
        }
    }

    let context = ErrorContext::new("locating configuration file")
        .with_context("checking standard configuration locations");

    let aka_error = context.to_config_not_found_error(attempted_paths, home_dir.clone(), None);
    Err(eyre::eyre!(aka_error))
}

pub fn get_config_path_with_override(home_dir: &PathBuf, override_path: &Option<PathBuf>) -> Result<PathBuf> {
    match override_path {
        Some(path) => {
            if path.exists() {
                Ok(path.clone())
            } else {
                let context = ErrorContext::new("locating custom configuration file")
                    .with_file(path.clone())
                    .with_context("custom config path specified via --config option");

                let aka_error = context.to_config_not_found_error(vec![path.clone()], home_dir.clone(), Some(path.clone()));
                Err(eyre::eyre!(aka_error))
            }
        }
        None => get_config_path(home_dir),
    }
}

pub fn test_config(file: &PathBuf) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.clone());
    }
    Err(eyre!("config {:?} not found!", file))
}

pub fn setup_logging(home_dir: &PathBuf) -> Result<()> {
    if is_benchmark_mode() {
        // In benchmark mode, log to stdout for visibility
        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Stdout)
            .init();
    } else {
        // In normal mode, log to file
        let log_dir = home_dir.join(".local").join("share").join("aka").join("logs");

        std::fs::create_dir_all(&log_dir)?;
        let log_file_path = log_dir.join("aka.log");

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

pub fn hash_config_file(config_path: &PathBuf) -> Result<String> {
    let content = std::fs::read(config_path)?;
    let hash = xxh3_64(&content);
    Ok(format!("{:016x}", hash))
}

pub fn get_hash_cache_path(home_dir: &PathBuf) -> Result<PathBuf> {
    let cache_dir = home_dir.join(".local").join("share").join("aka");

    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("config.hash"))
}

pub fn get_stored_hash(home_dir: &PathBuf) -> Result<Option<String>> {
    let hash_path = get_hash_cache_path(home_dir)?;
    if hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path)?;
        Ok(Some(stored_hash.trim().to_string()))
    } else {
        Ok(None)
    }
}

pub fn store_hash(hash: &str, home_dir: &PathBuf) -> Result<()> {
    let hash_path = get_hash_cache_path(home_dir)?;
    std::fs::write(&hash_path, hash)?;
    Ok(())
}

fn check_daemon_health(socket_path: &PathBuf) -> Result<bool> {
    debug!("✅ Daemon socket exists, testing health");

    // Try to connect and send health request
    if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
        use std::io::{BufRead, BufReader, Write};

        let health_request = serde_json::json!({
            "type": "Health"
        });

        debug!("📤 Sending health request to daemon");
        if let Ok(_) = writeln!(stream, "{}", health_request) {
            let mut reader = BufReader::new(&stream);
            let mut response_line = String::new();

            match reader.read_line(&mut response_line) {
                Ok(_) => {
                    debug!("📥 Received daemon response: {}", response_line.trim());

                    if let Ok(response) = serde_json::from_str::<serde_json::Value>(&response_line.trim()) {
                        if let Some(status) = response.get("status").and_then(|s| s.as_str()) {
                            debug!("🔍 Daemon status parsed: {}", status);
                            if status.starts_with("healthy:") && status.contains(":aliases") {
                                debug!("✅ Daemon is healthy and has config loaded: {}", status);
                                debug!("🎯 Health check result: DAEMON_HEALTHY (returning 0)");
                                return Ok(true); // Daemon healthy - best case
                            } else {
                                debug!("⚠️ Daemon status indicates unhealthy: {}", status);
                            }
                        } else {
                            debug!("⚠️ Daemon response missing status field");
                        }
                    } else {
                        debug!("⚠️ Failed to parse daemon response as JSON");
                    }
                }
                Err(e) => {
                    debug!("⚠️ Failed to read daemon response: {}", e);
                }
            }
        } else {
            debug!("⚠️ Failed to send health request to daemon");
        }
    } else {
        debug!("⚠️ Failed to connect to daemon socket");
    }
    debug!("❌ Daemon socket exists but health check failed");
    Ok(false)
}

fn validate_fresh_config_and_store_hash(
    config_path: &PathBuf,
    current_hash: &str,
    home_dir: &PathBuf,
) -> Result<i32> {
    // Use the same loader as direct mode for consistency
    let loader = Loader::new();
    debug!("🔄 Loading fresh config from: {:?}", config_path);
    match loader.load(config_path) {
        Ok(spec) => {
            debug!("✅ Fresh config loaded successfully");

            // Config is valid, store the new hash
            if let Err(e) = store_hash(current_hash, home_dir) {
                debug!("⚠️ Warning: could not store config hash: {}", e);
            } else {
                debug!("✅ New config hash stored: {}", current_hash);
            }

            // Check if we have any aliases
            if spec.aliases.is_empty() {
                debug!("⚠️ Fresh config valid but no aliases defined");
                debug!("🎯 Health check result: NO_ALIASES (returning 3)");
                return Ok(3); // No aliases defined
            }

            debug!("✅ Fresh config valid with {} aliases", spec.aliases.len());
            debug!("🎯 Health check result: FRESH_CONFIG_VALID (returning 0)");
            Ok(0) // All good
        }
        Err(e) => {
            debug!("❌ Health check failed: config file invalid: {}", e);
            debug!("🚨 All health check methods failed - ZLE should not use aka");
            debug!("🎯 Health check result: CONFIG_INVALID (returning 2)");
            Ok(2) // Config file invalid - critical failure
        }
    }
}

pub fn execute_health_check(home_dir: &PathBuf) -> Result<i32> {
    debug!("🏥 === HEALTH CHECK START ===");
    debug!("📋 Health check will determine the best processing path");

    // Step 1: Check if daemon is available and healthy
    debug!("📋 Step 1: Checking daemon health");
    if let Ok(socket_path) = determine_socket_path(home_dir) {
        debug!("🔌 Daemon socket path: {:?}", socket_path);
        if socket_path.exists() {
            if check_daemon_health(&socket_path)? {
                return Ok(0); // Daemon healthy - best case
            }
        } else {
            debug!("❌ Daemon socket not found at path: {:?}", socket_path);
        }
    } else {
        debug!("❌ Cannot determine daemon socket path");
    }

    // Step 2: Daemon not available, check config file cache
    debug!("📋 Step 2: Daemon unavailable, checking config cache");

    let config_path = match get_config_path(home_dir) {
        Ok(path) => path,
        Err(e) => {
            debug!("❌ Health check failed: config file not found: {}", e);
            debug!("🎯 Health check result: CONFIG_NOT_FOUND (returning 1)");
            return Ok(1); // Config file not found
        }
    };

    // Step 3: Calculate current config hash
    debug!("📋 Step 3: Calculating current config hash");
    let current_hash = match hash_config_file(&config_path) {
        Ok(hash) => {
            debug!("✅ Current config hash calculated: {}", hash);
            hash
        }
        Err(e) => {
            debug!("❌ Health check failed: cannot read config file: {}", e);
            debug!("🎯 Health check result: CONFIG_READ_ERROR (returning 1)");
            return Ok(1); // Cannot read config file
        }
    };

    // Step 4: Compare with stored hash
    debug!("📋 Step 4: Comparing with stored hash");
    let stored_hash = get_stored_hash(home_dir).unwrap_or(None);

    match stored_hash {
        Some(stored) => {
            debug!("🔍 Found stored hash: {}", stored);
            if stored == current_hash {
                debug!("✅ Hash matches! Config cache is valid, can use direct mode");
                debug!("🎯 Health check result: CACHE_VALID (returning 0)");
                return Ok(0);
            } else {
                debug!("⚠️ Hash mismatch: stored={}, current={}", stored, current_hash);
                debug!("📋 Cache invalid, need fresh config load");
            }
        }
        None => {
            debug!("⚠️ No stored hash found, need fresh config load");
        }
    }

    // Step 5: Hash doesn't match or no stored hash, validate config fresh
    debug!("📋 Step 5: Cache invalid, attempting fresh config load");
    validate_fresh_config_and_store_hash(&config_path, &current_hash, home_dir)
}

// Processing mode enum to track daemon vs direct processing
#[derive(Debug, Clone, Copy)]
pub enum ProcessingMode {
    Daemon,  // Processing via daemon (goblin emoji 👹)
    Direct,  // Processing directly (inbox emoji 📥)
}

// Main AKA struct and implementation
pub struct AKA {
    pub eol: bool,
    pub spec: Spec,
    pub config_hash: String,
    pub home_dir: PathBuf,
}

impl AKA {
    pub fn new(eol: bool, home_dir: PathBuf) -> Result<Self> {
        use std::time::Instant;

        let start_total = Instant::now();

        // Config path is always derived from home_dir using the same function
        let config_path = get_config_path(&home_dir)?;

        // Calculate config hash
        let config_hash = hash_config_file(&config_path)?;
        debug!("🔒 Config hash: {}", config_hash);

        // Time loader creation and config loading - use same loader as health check
        let start_load = Instant::now();
        let loader = Loader::new();
        let mut spec = loader.load(&config_path)?;
        let load_duration = start_load.elapsed();

        // Try to load from cache first
        let start_cache = Instant::now();
        if let Some(cached_aliases) = load_alias_cache(&config_hash, &home_dir)? {
            debug!("📋 Using cached aliases with usage counts ({} aliases)", cached_aliases.len());
            // Log a sample alias count for debugging
            if let Some((name, alias)) = cached_aliases.iter().next() {
                debug!("📋 Sample alias '{}' has count: {}", name, alias.count);
            }
            spec.aliases = cached_aliases;
        } else {
            debug!("📋 No cache found, initializing usage counts to 0");
            // Initialize all counts to 0 (they already are due to skip_deserializing) and save to cache
            save_alias_cache(&config_hash, &spec.aliases, &home_dir)?;
        }
        let cache_duration = start_cache.elapsed();

        let total_duration = start_total.elapsed();

        debug!("🏗️  AKA initialization complete:");
        debug!("  📋 Config loading: {:.3}ms", load_duration.as_secs_f64() * 1000.0);
        debug!("  🗃️  Cache handling: {:.3}ms", cache_duration.as_secs_f64() * 1000.0);
        debug!("  🎯 Total time: {:.3}ms", total_duration.as_secs_f64() * 1000.0);

        Ok(AKA { eol, spec, config_hash, home_dir })
    }

    pub fn use_alias(&self, alias: &Alias, pos: usize) -> bool {
        if alias.is_variadic() && !self.eol {
            false
        } else if pos == 0 {
            true
        } else {
            alias.global
        }
    }

    fn split_respecting_quotes(cmdline: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut start = 0;
        let mut in_quotes = false;
        let chars: Vec<char> = cmdline.chars().collect();
        for index in 0..chars.len() {
            if chars[index] == '"' {
                in_quotes = !in_quotes;
            } else if chars[index] == ' ' && !in_quotes {
                if start != index {
                    args.push(cmdline[start..index].to_string());
                }
                start = index + 1;
            } else if chars[index] == '!' && !in_quotes && index == chars.len() - 1 {
                if start != index {
                    args.push(cmdline[start..index].to_string());
                }
                args.push(String::from("!"));
                start = index + 1;
            }
        }
        if start != chars.len() {
            args.push(cmdline[start..].to_string());
        }
        args
    }

    fn perform_lookup(&self, key: &str, lookup: &str) -> Option<String> {
        self.spec.lookups.get(lookup).and_then(|map| map.get(key).cloned())
    }

    pub fn replace(&mut self, cmdline: &str) -> Result<String> {
        self.replace_with_mode(cmdline, ProcessingMode::Direct)
    }

    pub fn replace_with_mode(&mut self, cmdline: &str, mode: ProcessingMode) -> Result<String> {
        debug!("Processing command line: {}", cmdline);
        let mut pos: usize = 0;
        let mut space = " ";
        let mut replaced = false;
        let mut sudo = false;
        let mut args = Self::split_respecting_quotes(cmdline);

        if args.is_empty() {
            return Ok(String::new());
        }

        if args[0] == "sudo" {
            sudo = true;
            args.remove(0);
            if args.is_empty() {
                return Ok(String::new());
            }
        }

        while pos < args.len() {
            let current_arg = args[pos].clone();

            // Perform lookup replacement logic
            if current_arg.starts_with("lookup:") && current_arg.contains("[") && current_arg.ends_with("]") {
                let parts: Vec<&str> = current_arg.splitn(2, '[').collect();
                let lookup = parts[0].trim_start_matches("lookup:");
                let key = parts[1].trim_end_matches("]");
                if let Some(replacement) = self.perform_lookup(key, lookup) {
                    args[pos] = replacement.clone(); // Replace in args
                    replaced = true;
                    continue; // Reevaluate the current position after replacement
                }
            }

            let mut remainders: Vec<String> = args[pos + 1..].to_vec();

            // First check if we should use the alias (immutable borrow)
            let should_use_alias = match self.spec.aliases.get(&current_arg) {
                Some(alias) => self.use_alias(alias, pos),
                None => false,
            };

            let (value, count, replaced_alias, space_str) = if should_use_alias {
                // Now we can safely get mutable reference
                if let Some(alias) = self.spec.aliases.get_mut(&current_arg) {
                    Self::process_alias_replacement(alias, &current_arg, cmdline, &mut remainders, pos)?
                } else {
                    (current_arg.clone(), 0, false, " ")
                }
            } else {
                (current_arg.clone(), 0, false, " ")
            };

            if replaced_alias {
                replaced = true;
            }
            space = space_str;

            let beg = pos + 1;
            let end = beg + count;

            if space.is_empty() {
                args.drain(beg..end);
            } else {
                args.drain(beg..end);
            }
            args.splice(pos..=pos, Self::split_respecting_quotes(&value));
            pos += 1;
        }

        if sudo {
            args[0] = format!("$(which {})", args[0]);
            args.insert(0, "sudo".to_string());
        }

        let result = if replaced || sudo {
            format!("{}{}", args.join(" "), space)
        } else {
            String::new()
        };

        if replaced || sudo {
            let emoji = match mode {
                ProcessingMode::Daemon => "👹", // Goblin for daemon
                ProcessingMode::Direct => "📥", // Inbox for direct
            };
            info!("{} Command line transformed: {} -> {}", emoji, cmdline, result.trim());

            // Save updated usage counts to cache if any aliases were used
            if replaced {
                if let Err(e) = save_alias_cache(&self.config_hash, &self.spec.aliases, &self.home_dir) {
                    warn!("⚠️ Failed to save alias cache: {}", e);
                }
            }
        }

        Ok(result)
    }

    fn process_alias_replacement(
        alias: &mut Alias,
        current_arg: &str,
        cmdline: &str,
        remainders: &mut Vec<String>,
        pos: usize,
    ) -> Result<(String, usize, bool, &'static str)> {
        if (alias.global && cmdline.contains(&alias.value))
            || (!alias.global && pos == 0 && cmdline.starts_with(&alias.value))
        {
            Ok((current_arg.to_string(), 0, false, " "))
        } else {
            let space = if alias.space { " " } else { "" };
            let (v, c) = alias.replace(remainders)?;
            let replaced = v != alias.name;
            if replaced {
                // Increment usage count when alias is actually used
                alias.count += 1;
                debug!("📊 Alias '{}' used, count now: {}", alias.name, alias.count);
            }
            Ok((v, c, replaced, space))
        }
    }
}

pub fn print_alias(alias: &Alias) {
    if alias.value.contains('\n') {
        println!("{}: |\n  {}", alias.name, alias.value.replace("\n", "\n  "));
    } else {
        println!("{}: {}", alias.name, alias.value);
    }
}

// Utility function to determine socket path for daemon
pub fn determine_socket_path(home_dir: &PathBuf) -> Result<PathBuf> {
    // Try XDG_RUNTIME_DIR first
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(runtime_dir).join("aka").join("daemon.sock");
        return Ok(path);
    }

    // Fallback to ~/.local/share/aka/
    Ok(home_dir.join(".local/share/aka/daemon.sock"))
}

pub fn get_alias_cache_path(config_hash: &str, home_dir: &PathBuf) -> Result<PathBuf> {
    let data_dir = home_dir.join(".local").join("share").join("aka");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join(format!("{}.json", config_hash)))
}

pub fn get_alias_cache_path_with_base(config_hash: &str, base_dir: Option<&PathBuf>) -> Result<PathBuf> {
    let data_dir = match base_dir {
        Some(dir) => dir.clone(),
        None => {
            // Check for test environment variable first
            if let Ok(test_dir) = std::env::var("AKA_TEST_CACHE_DIR") {
                PathBuf::from(test_dir)
            } else {
                dirs::data_local_dir()
                    .ok_or_else(|| eyre!("Could not determine local data directory"))?
                    .join("aka")
            }
        }
    };

    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join(format!("{}.json", config_hash)))
}

pub fn load_alias_cache(config_hash: &str, home_dir: &PathBuf) -> Result<Option<HashMap<String, Alias>>> {
    let cache_path = get_alias_cache_path(config_hash, home_dir)?;

    if !cache_path.exists() {
        debug!("Cache file doesn't exist: {:?}", cache_path);
        return Ok(None);
    }

    debug!("Loading alias cache from: {:?}", cache_path);
    let content = std::fs::read_to_string(&cache_path)?;
    let cache: AliasCache = serde_json::from_str(&content)?;

    // Restore names from HashMap keys since they might be empty in the cache
    let mut aliases_with_names = HashMap::new();
    for (key, mut alias) in cache.aliases {
        if alias.name.is_empty() {
            alias.name = key.clone();
        }
        aliases_with_names.insert(key, alias);
    }

    debug!("Loaded {} aliases from cache", aliases_with_names.len());
    Ok(Some(aliases_with_names))
}

pub fn load_alias_cache_with_base(config_hash: &str, base_dir: Option<&PathBuf>) -> Result<Option<HashMap<String, Alias>>> {
    let cache_path = get_alias_cache_path_with_base(config_hash, base_dir)?;

    if !cache_path.exists() {
        debug!("Cache file doesn't exist: {:?}", cache_path);
        return Ok(None);
    }

    debug!("Loading alias cache from: {:?}", cache_path);
    let content = std::fs::read_to_string(&cache_path)?;
    let cache: AliasCache = serde_json::from_str(&content)?;

    // Restore names from HashMap keys since they might be empty in the cache
    let mut aliases_with_names = HashMap::new();
    for (key, mut alias) in cache.aliases {
        if alias.name.is_empty() {
            alias.name = key.clone();
        }
        aliases_with_names.insert(key, alias);
    }

    debug!("Loaded {} aliases from cache", aliases_with_names.len());
    Ok(Some(aliases_with_names))
}

pub fn save_alias_cache(config_hash: &str, aliases: &HashMap<String, Alias>, home_dir: &PathBuf) -> Result<()> {
    let cache_path = get_alias_cache_path(config_hash, home_dir)?;

    let cache = AliasCache {
        aliases: aliases.clone(),
    };

    let content = serde_json::to_string_pretty(&cache)?;

    // Write to temporary file first, then rename (atomic operation)
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &cache_path)?;

    debug!("Saved alias cache to: {:?}", cache_path);
    Ok(())
}

pub fn save_alias_cache_with_base(config_hash: &str, aliases: &HashMap<String, Alias>, base_dir: Option<&PathBuf>) -> Result<()> {
    let cache_path = get_alias_cache_path_with_base(config_hash, base_dir)?;

    let cache = AliasCache {
        aliases: aliases.clone(),
    };

    let content = serde_json::to_string_pretty(&cache)?;

    // Write to temporary file first, then rename (atomic operation)
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &cache_path)?;

    debug!("Saved alias cache to: {:?}", cache_path);
    Ok(())
}

pub fn migrate_alias_counts(old_hash: &str, new_hash: &str, new_aliases: &mut HashMap<String, Alias>) -> Result<()> {
    migrate_alias_counts_with_base(old_hash, new_hash, new_aliases, None)
}

pub fn migrate_alias_counts_with_base(old_hash: &str, new_hash: &str, new_aliases: &mut HashMap<String, Alias>, base_dir: Option<&PathBuf>) -> Result<()> {
    if let Some(old_aliases) = load_alias_cache_with_base(old_hash, base_dir)? {
        debug!("Migrating usage counts from old config hash: {}", old_hash);

        for (key, new_alias) in new_aliases.iter_mut() {
            if let Some(old_alias) = old_aliases.get(key) {
                new_alias.count = old_alias.count;
                debug!("Migrated count {} for alias: {}", old_alias.count, key);
            }
        }

        // Save the new cache with migrated counts
        save_alias_cache_with_base(new_hash, new_aliases, base_dir)?;

        debug!("Migration complete, saved new cache with hash: {}", new_hash);
    } else {
        debug!("No old cache found for migration from hash: {}", old_hash);
    }

    Ok(())
}
