use eyre::{eyre, Result};
use log::{info, debug, warn};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::{Instant, Duration};
use xxhash_rust::xxh3::xxh3_64;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json;

// Global timing storage for analysis
use std::sync::Mutex;
use lazy_static::lazy_static;

lazy_static! {
    static ref TIMING_LOG: Mutex<Vec<TimingData>> = Mutex::new(Vec::new());
}

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

// JSON cache structure for aliases with usage counts
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AliasCache {
    pub hash: String,
    pub aliases: HashMap<String, Alias>,
}

impl Default for AliasCache {
    fn default() -> Self {
        Self {
            hash: String::new(),
            aliases: HashMap::new(),
        }
    }
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

pub fn get_stored_hash(home_dir: &PathBuf) -> Result<Option<String>> {
    // Get hash from the cache file instead of separate config.hash file
    let cache = load_alias_cache(home_dir)?;
    if cache.hash.is_empty() {
        Ok(None)
    } else {
        Ok(Some(cache.hash))
    }
}

pub fn store_hash(hash: &str, _home_dir: &PathBuf) -> Result<()> {
    // Hash is now stored in the cache file itself, so this is a no-op
    // The hash gets stored when we save the cache via sync_cache_with_config
    debug!("Hash storage is now handled by cache file (hash: {})", hash);
    Ok(())
}

fn check_daemon_health(socket_path: &PathBuf) -> Result<bool> {
    debug!("✅ Daemon socket exists, testing health");

    // Try to connect with timeout
    let stream = match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(stream) => {
            // Set a short timeout for the connection
            if let Err(e) = stream.set_read_timeout(Some(std::time::Duration::from_millis(500))) {
                debug!("⚠️ Failed to set read timeout: {}", e);
                return Ok(false);
            }
            if let Err(e) = stream.set_write_timeout(Some(std::time::Duration::from_millis(500))) {
                debug!("⚠️ Failed to set write timeout: {}", e);
                return Ok(false);
            }
            stream
        }
        Err(e) => {
            debug!("⚠️ Failed to connect to daemon socket: {}", e);
            return Ok(false);
        }
    };

    // Try to send health request
    {
        let mut stream = stream;
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
                            // Parse format: "healthy:COUNT:synced" or "healthy:COUNT:stale"
                            // Must be exactly 3 parts separated by colons
                            let parts: Vec<&str> = status.split(':').collect();
                            if parts.len() == 3 && parts[0] == "healthy" && parts[1].parse::<u32>().is_ok() && (parts[2] == "synced" || parts[2] == "stale") {
                                debug!("✅ Daemon is healthy and has config loaded: {}", status);
                                return Ok(true); // Daemon healthy
                            } else {
                                debug!("⚠️ Daemon status indicates unhealthy: {}", status);
                                return Ok(false); // Daemon unhealthy
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
    }

    debug!("❌ Daemon socket exists but health check failed - daemon is dead");
    Ok(false) // Daemon is dead
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
            match check_daemon_health(&socket_path)? {
                true => {
                    debug!("✅ Daemon is healthy and running");
                    debug!("🎯 Health check result: DAEMON_HEALTHY (returning 0)");
                    return Ok(0); // Daemon healthy - best case
                },
                false => {
                    debug!("❌ Daemon socket exists but daemon is dead - stale socket detected");
                    debug!("🎯 Health check result: STALE_SOCKET (returning 4)");
                    return Ok(4); // Stale socket - skip daemon, go directly to direct mode
                }
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

// Sudo wrapping utility functions
/// Detect if a command is already wrapped with $(which ...)
fn is_already_wrapped(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed.starts_with("$(which ") && trimmed.ends_with(")")
}

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

/// Determine if a command needs sudo wrapping
fn needs_sudo_wrapping(command: &str) -> bool {
    // Skip if already wrapped (idempotent)
    if is_already_wrapped(command) {
        debug!("Command already wrapped: {}", command);
        return false;
    }

    // Skip complex commands (contain spaces, pipes, redirects, etc.)
    if command.contains(' ') || command.contains('|') || command.contains('&')
       || command.contains('>') || command.contains('<') || command.contains(';') {
        debug!("Skipping complex command: {}", command);
        return false;
    }

    // Only wrap if it's available to user but not root
    let needs_wrapping = is_user_only_command(command);
    debug!("Command '{}' needs wrapping: {}", command, needs_wrapping);
    needs_wrapping
}

/// Check if a command is user-installed and needs environment preservation
fn is_user_installed_tool(command: &str) -> bool {
    if let Ok(output) = std::process::Command::new("which")
        .arg(command)
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let path = path.trim();

            // Check if command is in user directories
            if path.contains("/.cargo/bin/") ||
               path.contains("/.local/bin/") ||
               path.contains("/home/") ||
               path.starts_with(&std::env::var("HOME").unwrap_or_default()) {
                debug!("Command '{}' at '{}' is user-installed", command, path);
                return true;
            }
        }
    }
    false
}

// Main AKA struct and implementation
pub struct AKA {
    pub eol: bool,
    pub spec: Spec,
    pub config_hash: String,
    pub home_dir: PathBuf,
}

impl AKA {
    pub fn new(eol: bool, home_dir: PathBuf, config_path: PathBuf) -> Result<Self> {
        use std::time::Instant;

        let start_total = Instant::now();

        // Calculate config hash
        let config_hash = hash_config_file(&config_path)?;
        debug!("🔒 Config hash: {}", config_hash);

        // Time loader creation and config loading - use same loader as health check
        let start_load = Instant::now();
        let loader = Loader::new();
        let mut spec = loader.load(&config_path)?;

        // Expand keys in lookups - convert "prod|apps: us-east-1" to separate entries
        for (_, map) in spec.lookups.iter_mut() {
            let mut expanded = HashMap::new();
            for (pattern, value) in map.iter() {
                let keys: Vec<&str> = pattern.split('|').collect();
                for key in keys {
                    expanded.insert(key.to_string(), value.clone());
                }
            }
            *map = expanded;
        }

        let load_duration = start_load.elapsed();

        // Sync cache with config - but only if using default config path
        let start_cache = Instant::now();
        let default_config_path = get_config_path(&home_dir).unwrap_or_else(|_| PathBuf::new());

        debug!("🔍 Config path comparison:");
        debug!("  📄 Provided config: {:?}", config_path);
        debug!("  📄 Default config: {:?}", default_config_path);
        debug!("  ✅ Paths equal: {}", config_path == default_config_path);

        if config_path == default_config_path {
            // Using default config, so use cache
            let cache = sync_cache_with_config_path(&home_dir, &config_path)?;
            debug!("📋 Using cached aliases with usage counts ({} aliases)", cache.aliases.len());
            // Log a sample alias count for debugging
            if let Some((name, alias)) = cache.aliases.iter().next() {
                debug!("📋 Sample alias '{}' has count: {}", name, alias.count);
            }
            spec.aliases = cache.aliases;
        } else {
            // Using custom config, skip cache and use config as-is
            debug!("📋 Using custom config, skipping cache ({} aliases)", spec.aliases.len());
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
        if pos == 0 {
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
        debug!("🔍 STARTING REPLACEMENT: Input command line: '{}'", cmdline);
        let mut pos: usize = 0;
        let mut space = " ";
        let mut replaced = false;
        let mut sudo = false;
        let mut sudo_prefix = "sudo".to_string();
        let mut args = Self::split_respecting_quotes(cmdline);

        if args.is_empty() {
            debug!("🔍 EMPTY ARGS: No arguments to process");
            return Ok(String::new());
        }

        debug!("🔍 SPLIT ARGS: {:?}", args);

        // Check for sudo trigger pattern: command ends with "!" (only when eol=true)
        if self.eol && !args.is_empty() {
            if let Some(last_arg) = args.last() {
                if last_arg == "!" {
                    args.pop(); // Remove the "!"
                    sudo = true;
                    debug!("🔍 SUDO TRIGGER DETECTED: Removed '!' from args, remaining: {:?}", args);

                    // If args is now empty (lone "!"), return empty string
                    if args.is_empty() {
                        debug!("🔍 EMPTY ARGS AFTER SUDO TRIGGER: Lone '!' detected, returning empty");
                        return Ok(String::new());
                    }
                }
            }
        }

        if !args.is_empty() && args[0] == "sudo" {
            sudo = true;
            let sudo_part = args.remove(0);
            debug!("🔍 SUDO DETECTED: Removed '{}' from args, remaining: {:?}", sudo_part, args);

            // Handle sudo with flags (like -E, -i, etc.)
            let mut sudo_flags = Vec::new();
            sudo_flags.push(sudo_part);

            // Collect any sudo flags that come after sudo
            while !args.is_empty() && args[0].starts_with('-') {
                let flag = args.remove(0);
                debug!("🔍 SUDO FLAG DETECTED: Removed '{}' from args, remaining: {:?}", flag, args);

                // Handle flags that take values (like -u, -g, -C, etc.)
                let needs_value = flag == "-u" || flag == "-g" || flag == "-C" || flag == "-s" || flag == "-r" || flag == "-t";
                sudo_flags.push(flag);

                if needs_value && !args.is_empty() && !args[0].starts_with('-') {
                    let value = args.remove(0);
                    debug!("🔍 SUDO FLAG VALUE DETECTED: Removed '{}' from args, remaining: {:?}", value, args);
                    sudo_flags.push(value);
                }
            }

            if args.is_empty() {
                debug!("🔍 SUDO ONLY: Only sudo command with flags, returning joined sudo parts");
                return Ok(format!("{} ", sudo_flags.join(" ")));
            }

            // Store the sudo flags for later reconstruction
            sudo_prefix = sudo_flags.join(" ");
            debug!("🔍 SUDO PREFIX: '{}'", sudo_prefix);
        }

        while pos < args.len() {
            let current_arg = args[pos].clone();
            debug!("🔍 PROCESSING ARG[{}]: '{}'", pos, current_arg);

            // Perform lookup replacement logic
            if current_arg.starts_with("lookup:") && current_arg.contains("[") && current_arg.ends_with("]") {
                let parts: Vec<&str> = current_arg.splitn(2, '[').collect();
                let lookup = parts[0].trim_start_matches("lookup:");
                let key = parts[1].trim_end_matches("]");
                debug!("🔍 LOOKUP DETECTED: lookup='{}', key='{}'", lookup, key);
                if let Some(replacement) = self.perform_lookup(key, lookup) {
                    debug!("🔧 LOOKUP REPLACEMENT: '{}' -> '{}'", current_arg, replacement);
                    args[pos] = replacement.clone(); // Replace in args
                    replaced = true;
                    continue; // Reevaluate the current position after replacement
                } else {
                    debug!("🔍 LOOKUP FAILED: No replacement found for lookup='{}', key='{}'", lookup, key);
                }
            }

            let mut remainders: Vec<String> = args[pos + 1..].to_vec();

            // First check if we should use the alias (immutable borrow)
            let should_use_alias = match self.spec.aliases.get(&current_arg) {
                Some(alias) => {
                    let should_use = self.use_alias(alias, pos);
                    debug!("🔍 ALIAS CHECK: '{}' -> should_use={}", current_arg, should_use);
                    should_use
                }
                None => {
                    debug!("🔍 NO ALIAS: '{}' not found in aliases", current_arg);
                    false
                }
            };

            let (value, count, replaced_alias, space_str) = if should_use_alias {
                // Clone aliases for variable interpolation to avoid borrowing conflicts
                let aliases_for_interpolation = self.spec.aliases.clone();
                // Now we can safely get mutable reference
                if let Some(alias) = self.spec.aliases.get_mut(&current_arg) {
                    debug!("🔍 PROCESSING ALIAS: '{}' -> '{}'", current_arg, alias.value);
                    Self::process_alias_replacement(alias, &current_arg, cmdline, &mut remainders, pos, &aliases_for_interpolation, self.eol)?
                } else {
                    (current_arg.clone(), 0, false, " ")
                }
            } else {
                (current_arg.clone(), 0, false, " ")
            };

            if replaced_alias {
                debug!("🔧 ALIAS REPLACEMENT: '{}' -> '{}' (count={}, space='{}')", current_arg, value, count, space_str);
                replaced = true;
                // Only update space when we actually replace an alias
                space = space_str;
            } else {
                debug!("🔍 NO REPLACEMENT: '{}' unchanged", current_arg);
            }

            let beg = pos + 1;
            let end = beg + count;

            let args_before = args.clone();
            if space.is_empty() {
                args.drain(beg..end);
            } else {
                args.drain(beg..end);
            }
            args.splice(pos..=pos, Self::split_respecting_quotes(&value));
            debug!("🔍 ARGS UPDATE: {:?} -> {:?}", args_before, args);
            pos += 1;
        }

        if sudo {
            let command = args[0].clone();
            debug!("🔍 SUDO PROCESSING: command='{}'", command);
            let args_before_sudo = args.clone();

            // Check if we need to wrap the command with $(which)
            if needs_sudo_wrapping(&command) {
                let old_arg = args[0].clone();
                args[0] = format!("$(which {})", command);
                debug!("🔧 SUDO $(which) WRAPPING: '{}' -> '{}'", old_arg, args[0]);
            } else {
                debug!("🔍 SUDO NO WRAPPING: '{}' does not need $(which) wrapping", command);
            }

            // For user-installed tools, preserve environment with -E flag
            if is_user_installed_tool(&command) {
                debug!("🔍 USER-INSTALLED TOOL DETECTED: '{}'", command);
                // Check if -E flag is not already present
                if !sudo_prefix.contains("-E") {
                    sudo_prefix = format!("{} -E", sudo_prefix);
                    debug!("🔧 ADDED -E FLAG: sudo_prefix now: '{}'", sudo_prefix);
                } else {
                    debug!("🔧 -E FLAG ALREADY PRESENT: sudo_prefix: '{}'", sudo_prefix);
                }
            } else {
                debug!("🔍 NOT USER-INSTALLED: '{}' is system command", command);
            }

            args.insert(0, sudo_prefix);
            debug!("🔧 ADDED SUDO: args now: {:?}", args);

            // Interactive tools like rkvr should work properly with the environment preservation
            debug!("🔍 SUDO PROCESSING COMPLETE: command='{}', replaced={}", command, replaced);

            debug!("🔧 SUDO TRANSFORMATION: {:?} -> {:?}", args_before_sudo, args);
        }

        let result = if replaced || sudo {
            format!("{}{}", args.join(" "), space)
        } else {
            String::new()
        };

        debug!("🔍 FINAL RESULT: replaced={}, sudo={}, result='{}'", replaced, sudo, result);

        if replaced || sudo {
            let emoji = match mode {
                ProcessingMode::Daemon => "👹", // Goblin for daemon
                ProcessingMode::Direct => "📥", // Inbox for direct
            };
            info!("{} Command line transformed: {} -> {}", emoji, cmdline, result.trim());

            // Save updated usage counts to cache if any aliases were used
            if replaced {
                debug!("🔍 SAVING CACHE: Aliases were used, saving cache");
                let cache = AliasCache {
                    hash: self.config_hash.clone(),
                    aliases: self.spec.aliases.clone(),
                };
                if let Err(e) = save_alias_cache(&cache, &self.home_dir) {
                    warn!("⚠️ Failed to save alias cache: {}", e);
                }
            }
        } else {
            debug!("🔍 NO TRANSFORMATION: No changes made to command line");
        }

        Ok(result)
    }

    fn process_alias_replacement(
        alias: &mut Alias,
        current_arg: &str,
        cmdline: &str,
        remainders: &mut Vec<String>,
        pos: usize,
        alias_map: &std::collections::HashMap<String, crate::cfg::alias::Alias>,
        eol: bool,
    ) -> Result<(String, usize, bool, &'static str)> {
        debug!("🔍 ALIAS REPLACEMENT LOGIC: alias='{}', current_arg='{}', pos={}, global={}",
               alias.name, current_arg, pos, alias.global);
        debug!("🔍 ALIAS DETAILS: value='{}', space={}, cmdline='{}'", alias.value, alias.space, cmdline);
        debug!("🔍 REMAINDERS: {:?}", remainders);

        if (alias.global && cmdline.contains(&alias.value))
            || (!alias.global && pos == 0 && cmdline.starts_with(&alias.value))
        {
            let space = if alias.space { " " } else { "" };
            debug!("🔍 ALIAS SKIP: Recursive replacement detected, skipping");
            Ok((current_arg.to_string(), 0, false, space))
        } else {
            let space = if alias.space { " " } else { "" };
            debug!("🔍 CALLING ALIAS.REPLACE: remainders={:?}", remainders);
            let (v, c) = alias.replace(remainders, alias_map, eol)?;
            let replaced = v != alias.name;
            debug!("🔍 ALIAS.REPLACE RESULT: v='{}', c={}, replaced={}", v, c, replaced);
            if replaced {
                // Increment usage count when alias is actually used
                alias.count += 1;
                debug!("📊 Alias '{}' used, count now: {}", alias.name, alias.count);
            }
            Ok((v, c, replaced, space))
        }
    }
}

/// Format alias output with proper alignment and optional counts
/// This function works with iterators to avoid unnecessary allocations
pub fn format_alias_output_from_iter<I>(aliases: I, show_counts: bool) -> String
where
    I: Iterator<Item = Alias>,
{
    // Collect into Vec to calculate max width (unavoidable for alignment)
    let aliases: Vec<_> = aliases.collect();
    let alias_count = aliases.len();

    if aliases.is_empty() {
        return format!("No aliases found.\n\ncount: 0");
    }

    // Calculate the maximum alias name width for alignment
    let max_name_width = aliases
        .iter()
        .map(|alias| alias.name.len())
        .max()
        .unwrap_or(0);

    let output = aliases
        .iter()
        .map(|alias| {
            let prefix = if show_counts {
                format!("{:>4} {:>width$} -> ", alias.count, alias.name, width = max_name_width)
            } else {
                format!("{:>width$} -> ", alias.name, width = max_name_width)
            };

            let indent = " ".repeat(prefix.len());

            if alias.value.contains('\n') {
                let lines: Vec<&str> = alias.value.split('\n').collect();
                let mut result = format!("{}{}", prefix, lines[0]);
                for line in &lines[1..] {
                    result.push_str(&format!("\n{}{}", indent, line));
                }
                result
            } else {
                format!("{}{}", prefix, alias.value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{}\n\ncount: {}", output, alias_count)
}

/// Create an iterator that filters and sorts aliases based on display criteria
/// This avoids intermediate Vec allocations until sorting is required
pub fn prepare_aliases_for_display_iter<'a>(
    aliases: impl Iterator<Item = &'a Alias> + 'a,
    show_counts: bool,
    show_all: bool,
    global_only: bool,
    patterns: &'a [String],
) -> impl Iterator<Item = Alias> + 'a {
    let mut filtered_aliases: Vec<_> = aliases
        .filter(move |alias| !global_only || alias.global)
        .filter(move |alias| patterns.is_empty() ||
                patterns.iter().any(|pattern| alias.name.starts_with(pattern)))
        .filter(move |alias| !show_counts || show_all || alias.count > 0)
        .cloned()
        .collect();

    // Sort based on whether we're showing counts
    if show_counts {
        // Sort by count (descending) then by name (ascending)
        filtered_aliases.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    } else {
        // Sort alphabetically by name
        filtered_aliases.sort_by(|a, b| a.name.cmp(&b.name));
    }

    filtered_aliases.into_iter()
}

/// Efficient function that combines filtering, sorting, and formatting in one pass
/// This is the most memory-efficient way to format alias output
pub fn format_aliases_efficiently<'a>(
    aliases: impl Iterator<Item = &'a Alias> + 'a,
    show_counts: bool,
    show_all: bool,
    global_only: bool,
    patterns: &'a [String],
) -> String {
    let prepared = prepare_aliases_for_display_iter(aliases, show_counts, show_all, global_only, patterns);
    format_alias_output_from_iter(prepared, show_counts)
}

/// Get alias names for shell completion
/// Returns a sorted list of alias names
pub fn get_alias_names_for_completion(aka: &AKA) -> Vec<String> {
    let mut names: Vec<String> = aka.spec.aliases.keys().cloned().collect();
    names.sort();
    names
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

pub fn get_alias_cache_path(home_dir: &PathBuf) -> Result<PathBuf> {
    let data_dir = home_dir.join(".local").join("share").join("aka");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("aka.json"))
}

pub fn get_alias_cache_path_with_base(base_dir: Option<&PathBuf>) -> Result<PathBuf> {
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
    Ok(data_dir.join("aka.json"))
}

pub fn calculate_config_hash(home_dir: &PathBuf) -> Result<String> {
    let config_path = get_config_path(home_dir)?;
    hash_config_file(&config_path)
}

pub fn load_alias_cache(home_dir: &PathBuf) -> Result<AliasCache> {
    let cache_path = get_alias_cache_path(home_dir)?;

    if !cache_path.exists() {
        debug!("Cache file doesn't exist: {:?}, returning default", cache_path);
        return Ok(AliasCache::default());
    }

    debug!("Loading alias cache from: {:?}", cache_path);
    let content = std::fs::read_to_string(&cache_path)?;
    let mut cache: AliasCache = serde_json::from_str(&content)?;

    // Restore names from HashMap keys since they might be empty in the cache
    for (key, alias) in cache.aliases.iter_mut() {
        if alias.name.is_empty() {
            alias.name = key.clone();
        }
    }

    debug!("Loaded {} aliases from cache with hash: {}", cache.aliases.len(), cache.hash);
    Ok(cache)
}

pub fn load_alias_cache_with_base(base_dir: Option<&PathBuf>) -> Result<AliasCache> {
    let cache_path = get_alias_cache_path_with_base(base_dir)?;

    if !cache_path.exists() {
        debug!("Cache file doesn't exist: {:?}, returning default", cache_path);
        return Ok(AliasCache::default());
    }

    debug!("Loading alias cache from: {:?}", cache_path);
    let content = std::fs::read_to_string(&cache_path)?;
    let mut cache: AliasCache = serde_json::from_str(&content)?;

    // Restore names from HashMap keys since they might be empty in the cache
    for (key, alias) in cache.aliases.iter_mut() {
        if alias.name.is_empty() {
            alias.name = key.clone();
        }
    }

    debug!("Loaded {} aliases from cache with hash: {}", cache.aliases.len(), cache.hash);
    Ok(cache)
}

pub fn save_alias_cache(cache: &AliasCache, home_dir: &PathBuf) -> Result<()> {
    let cache_path = get_alias_cache_path(home_dir)?;

    let content = serde_json::to_string_pretty(cache)?;

    // Write to temporary file first, then rename (atomic operation)
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &cache_path)?;

    debug!("Saved alias cache to: {:?}", cache_path);
    Ok(())
}

pub fn save_alias_cache_with_base(cache: &AliasCache, base_dir: Option<&PathBuf>) -> Result<()> {
    let cache_path = get_alias_cache_path_with_base(base_dir)?;

    let content = serde_json::to_string_pretty(cache)?;

    // Write to temporary file first, then rename (atomic operation)
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &cache_path)?;

    debug!("Saved alias cache to: {:?}", cache_path);
    Ok(())
}

pub fn sync_cache_with_config(home_dir: &PathBuf) -> Result<AliasCache> {
    // 1. Calculate current YAML hash
    let current_hash = calculate_config_hash(home_dir)?;

    // 2. Load existing cache (or default)
    let mut cache = load_alias_cache(home_dir)?;

    // 3. Check if hash changed
    if cache.hash != current_hash {
        // 4. Merge existing counts with new config
        cache = merge_cache_with_config(cache, current_hash, home_dir)?;

        // 5. Save updated cache (hash is embedded)
        save_alias_cache(&cache, home_dir)?;
    }

    Ok(cache)
}

pub fn sync_cache_with_config_path(home_dir: &PathBuf, config_path: &PathBuf) -> Result<AliasCache> {
    // 1. Calculate current YAML hash from the specific config file
    let current_hash = hash_config_file(config_path)?;

    // 2. Load existing cache (or default)
    let mut cache = load_alias_cache(home_dir)?;

    // 3. Check if hash changed
    if cache.hash != current_hash {
        // 4. Merge existing counts with new config from the specific config file
        cache = merge_cache_with_config_path(cache, current_hash, config_path)?;

        // 5. Save updated cache (hash is embedded)
        save_alias_cache(&cache, home_dir)?;
    }

    Ok(cache)
}

pub fn merge_cache_with_config(
    old_cache: AliasCache,
    new_hash: String,
    home_dir: &PathBuf
) -> Result<AliasCache> {
    // 1. Load new config from YAML
    let config_path = get_config_path(home_dir)?;
    let loader = Loader::new();
    let new_spec = loader.load(&config_path)?;

    // 2. Create new cache with new hash
    let mut new_cache = AliasCache {
        hash: new_hash,
        aliases: HashMap::new(),
    };

    // 3. For each alias in new config:
    for (name, mut alias) in new_spec.aliases {
        // Preserve count if alias existed before
        if let Some(old_alias) = old_cache.aliases.get(&name) {
            alias.count = old_alias.count;
        }
        new_cache.aliases.insert(name, alias);
    }

    Ok(new_cache)
}

pub fn merge_cache_with_config_path(
    old_cache: AliasCache,
    new_hash: String,
    config_path: &PathBuf
) -> Result<AliasCache> {
    // 1. Load new config from the specific YAML file
    let loader = Loader::new();
    let new_spec = loader.load(config_path)?;

    // 2. Create new cache with new hash
    let mut new_cache = AliasCache {
        hash: new_hash,
        aliases: HashMap::new(),
    };

    // 3. For each alias in new config:
    for (name, mut alias) in new_spec.aliases {
        // Preserve count if alias existed before
        if let Some(old_alias) = old_cache.aliases.get(&name) {
            alias.count = old_alias.count;
        }
        new_cache.aliases.insert(name, alias);
    }

    Ok(new_cache)
}



#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use cfg::spec::Defaults;

    fn create_test_aka_with_aliases(aliases: HashMap<String, Alias>) -> AKA {
        let spec = Spec {
            defaults: Defaults { version: 1 },
            aliases,
            lookups: HashMap::new(),
        };

        AKA {
            eol: true,  // Enable eol mode for variadic aliases
            spec,
            config_hash: "test_hash".to_string(),
            home_dir: std::env::temp_dir(),
        }
    }

    #[test]
    fn test_alias_with_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("ls").unwrap();

        // Should have trailing space
        assert_eq!(result, "eza ");
    }

    #[test]
    fn test_alias_with_space_false() {
        let mut aliases = HashMap::new();
        aliases.insert("gc".to_string(), Alias {
            name: "gc".to_string(),
            value: "git commit -m\"".to_string(),
            space: false,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("gc").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "git commit -m\"");
    }

    #[test]
    fn test_alias_with_space_false_complex() {
        let mut aliases = HashMap::new();
        aliases.insert("ping10".to_string(), Alias {
            name: "ping10".to_string(),
            value: "ping 10.10.10.".to_string(),
            space: false,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("ping10").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "ping 10.10.10.");
    }

    #[test]
    fn test_multiple_aliases_space_preserved() {
        let mut aliases = HashMap::new();
        aliases.insert("gc".to_string(), Alias {
            name: "gc".to_string(),
            value: "git commit -m\"".to_string(),
            space: false,
            global: false,
            count: 0,
        });
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test space: false
        let result1 = aka.replace("gc").unwrap();
        assert_eq!(result1, "git commit -m\"");

        // Test space: true
        let result2 = aka.replace("ls").unwrap();
        assert_eq!(result2, "eza ");
    }

    #[test]
    fn test_alias_with_arguments_space_false() {
        let mut aliases = HashMap::new();
        aliases.insert("gc".to_string(), Alias {
            name: "gc".to_string(),
            value: "git commit -m\"".to_string(),
            space: false,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("gc some message").unwrap();

        // Should NOT have trailing space even with arguments
        assert_eq!(result, "git commit -m\" some message");
    }

    #[test]
    fn test_alias_with_arguments_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("ls -la").unwrap();

        // Should have trailing space
        assert_eq!(result, "eza -la ");
    }

    #[test]
    fn test_no_alias_replacement_no_space_change() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);

        let result = aka.replace("nonexistent").unwrap();

        // No alias found, should return empty string
        assert_eq!(result, "");
    }

    #[test]
    fn test_variadic_alias_space_false() {
        let mut aliases = HashMap::new();
        aliases.insert("echo_all".to_string(), Alias {
            name: "echo_all".to_string(),
            value: "echo $@".to_string(),
            space: false,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("echo_all hello world").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "echo hello world");
    }

    #[test]
    fn test_variadic_alias_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert("echo_all".to_string(), Alias {
            name: "echo_all".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("echo_all hello world").unwrap();

        // Should have trailing space
        assert_eq!(result, "echo hello world ");
    }

    #[test]
    fn test_positional_alias_space_false() {
        let mut aliases = HashMap::new();
        aliases.insert("greet".to_string(), Alias {
            name: "greet".to_string(),
            value: "echo Hello $1".to_string(),
            space: false,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("greet World").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "echo Hello World");
    }

    #[test]
    fn test_positional_alias_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert("greet".to_string(), Alias {
            name: "greet".to_string(),
            value: "echo Hello $1".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("greet World").unwrap();

        // Should have trailing space
        assert_eq!(result, "echo Hello World ");
    }

    // Sudo wrapping tests
    #[test]
    fn test_is_already_wrapped() {
        assert!(is_already_wrapped("$(which ls)"));
        assert!(is_already_wrapped("  $(which ls)  "));
        assert!(is_already_wrapped("$(which eza)"));
        assert!(!is_already_wrapped("ls"));
        assert!(!is_already_wrapped("which ls"));
        assert!(!is_already_wrapped("$(ls)"));
        assert!(!is_already_wrapped("$(which)"));
        assert!(!is_already_wrapped("$(which "));
        assert!(!is_already_wrapped(" which ls)"));
    }



    #[test]
    fn test_needs_sudo_wrapping_already_wrapped() {
        // Should not wrap already wrapped commands
        assert!(!needs_sudo_wrapping("$(which ls)"));
        assert!(!needs_sudo_wrapping("  $(which eza)  "));
        assert!(!needs_sudo_wrapping("$(which systemctl)"));
    }

    #[test]
    fn test_needs_sudo_wrapping_complex_commands() {
        // Should not wrap complex commands
        assert!(!needs_sudo_wrapping("ls -la"));
        assert!(!needs_sudo_wrapping("cat file.txt"));
        assert!(!needs_sudo_wrapping("grep pattern | less"));
        assert!(!needs_sudo_wrapping("echo hello > file.txt"));
        assert!(!needs_sudo_wrapping("command < input.txt"));
        assert!(!needs_sudo_wrapping("cmd1 && cmd2"));
        assert!(!needs_sudo_wrapping("cmd1; cmd2"));
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
        assert!(!result2.contains("sudo sudo"));
    }

    #[test]
    fn test_sudo_wrapping_system_commands() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test with commands that should exist on most systems
        // These should not be wrapped since they're available to root
        let system_commands = vec![
            "sudo echo",
            "sudo cat",
            "sudo ls", // Note: this is the base ls, not aliased
        ];

        for cmd in system_commands {
            let result = aka.replace(cmd).unwrap();
            // Should not contain $(which) wrapping for system commands
            // Note: The actual behavior depends on system state, but should not double-wrap
            assert!(!result.contains("$(which $(which"),
                   "Should not double-wrap system command: {}", result);
        }
    }

    #[test]
    fn test_sudo_wrapping_nonexistent_commands() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test with commands that definitely don't exist
        let nonexistent_commands = vec![
            "sudo nonexistent_command_12345",
            "sudo fake_binary_xyz",
        ];

        for cmd in nonexistent_commands {
            let result = aka.replace(cmd).unwrap();
            // Should not wrap commands that don't exist for the user
            assert!(!result.contains("$(which nonexistent"),
                   "Should not wrap nonexistent command: {}", result);
            assert!(!result.contains("$(which fake_binary"),
                   "Should not wrap nonexistent command: {}", result);
        }
    }

    #[test]
    fn test_sudo_wrapping_with_aliases() {
        let mut aliases = HashMap::new();
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza -la".to_string(),
            space: true,
            global: false,
            count: 0,
        });
        aliases.insert("cat".to_string(), Alias {
            name: "cat".to_string(),
            value: "bat -p".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test sudo with aliased commands
        let result1 = aka.replace("sudo ls").unwrap();
        let result2 = aka.replace("sudo cat").unwrap();

        // Should contain sudo and the expanded alias
        assert!(result1.contains("sudo"));
        assert!(result1.contains("eza") && result1.contains("-la"));
        assert!(result2.contains("sudo"));
        assert!(result2.contains("bat") && result2.contains("-p"));

        // The key test is that we don't get double wrapping
        // Note: eza and bat might be wrapped with $(which) if they're user-installed

        // Should not double-wrap
        assert!(!result1.contains("$(which $(which"));
        assert!(!result2.contains("$(which $(which"));
    }

    #[test]
    fn test_sudo_wrapping_edge_cases() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);

        // Edge cases
        let edge_cases = vec![
            ("sudo", "sudo "),  // Just sudo
            ("sudo ", "sudo "),  // Sudo with space
        ];

        for (input, expected) in edge_cases {
            let result = aka.replace(input).unwrap();
            assert_eq!(result, expected, "Edge case '{}' failed", input);
        }
    }

    #[test]
    fn test_sudo_wrapping_preserves_arguments() {
        let mut aliases = HashMap::new();
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test that arguments are preserved
        let result = aka.replace("sudo ls -la --color").unwrap();

        // Should contain sudo, the expanded alias, and the arguments
        assert!(result.contains("sudo"));
        assert!(result.contains("eza"));
        assert!(result.contains("-la"));
        assert!(result.contains("--color"));

        // Should not double-wrap
        assert!(!result.contains("$(which $(which"));
    }

    #[test]
    fn test_get_alias_names_for_completion() {
        let mut aliases = HashMap::new();
        aliases.insert("zz".to_string(), Alias {
            name: "zz".to_string(),
            value: "eza -la".to_string(),
            space: true,
            global: false,
            count: 0,
        });
        aliases.insert("cat".to_string(), Alias {
            name: "cat".to_string(),
            value: "bat -p".to_string(),
            space: true,
            global: false,
            count: 0,
        });
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let aka = create_test_aka_with_aliases(aliases);
        let names = get_alias_names_for_completion(&aka);

        // Should be sorted alphabetically
        assert_eq!(names, vec!["cat", "ls", "zz"]);
    }

    #[test]
    fn test_get_alias_names_for_completion_empty() {
        let aliases = HashMap::new();
        let aka = create_test_aka_with_aliases(aliases);
        let names = get_alias_names_for_completion(&aka);

        // Should be empty
        assert_eq!(names, Vec::<String>::new());
    }

    #[test]
    fn test_get_alias_names_for_completion_special_chars() {
        let mut aliases = HashMap::new();
        aliases.insert("!!".to_string(), Alias {
            name: "!!".to_string(),
            value: "sudo !!".to_string(),
            space: true,
            global: false,
            count: 0,
        });
        aliases.insert("|c".to_string(), Alias {
            name: "|c".to_string(),
            value: "| xclip -sel clip".to_string(),
            space: true,
            global: true,
            count: 0,
        });
        aliases.insert("...".to_string(), Alias {
            name: "...".to_string(),
            value: "cd ../..".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        let aka = create_test_aka_with_aliases(aliases);
        let names = get_alias_names_for_completion(&aka);

        // Should be sorted alphabetically, including special characters
        assert_eq!(names, vec!["!!", "...", "|c"]);
    }

    #[test]
    fn test_sudo_trigger_comprehensive() {
        let mut aliases = HashMap::new();
        aliases.insert("ls".to_string(), Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        });

        // Test with eol=true (should work)
        let mut aka_eol = create_test_aka_with_aliases(aliases.clone());
        aka_eol.eol = true;

        let result = aka_eol.replace("touch file !").unwrap();
        assert!(result.starts_with("sudo"), "Should start with sudo: {}", result);
        assert!(result.contains("touch file"), "Should contain original command: {}", result);
        assert!(!result.contains("!"), "Should not contain exclamation mark: {}", result);

        // Test with alias expansion
        let result = aka_eol.replace("ls !").unwrap();
        assert!(result.starts_with("sudo"), "Should start with sudo: {}", result);
        assert!(result.contains("eza"), "Should expand alias: {}", result);
        assert!(!result.contains("!"), "Should not contain exclamation mark: {}", result);

        // Test with eol=false (should NOT work)
        let mut aka_no_eol = create_test_aka_with_aliases(aliases);
        aka_no_eol.eol = false;

        let result = aka_no_eol.replace("touch file !").unwrap();
        assert_eq!(result, "", "Should return empty string when eol=false");
    }

    #[test]
    fn test_sudo_trigger_edge_cases() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        // Test exclamation mark in middle of command (should NOT trigger sudo)
        let result = aka.replace("echo hello ! world").unwrap();
        assert_eq!(result, "", "Should not trigger sudo for mid-command exclamation");

        // Test with quoted arguments
        let result = aka.replace("echo \"test\" !").unwrap();
        assert!(result.starts_with("sudo"), "Should work with quoted arguments: {}", result);
        assert!(result.contains("echo \"test\""), "Should preserve quotes: {}", result);
        assert!(!result.contains("!"), "Should not contain exclamation mark: {}", result);

        // Test lone exclamation mark (should be ignored)
        let result = aka.replace("!").unwrap();
        assert_eq!(result, "", "Should ignore lone exclamation mark");

        // Test multiple exclamation marks (only last one should matter)
        let result = aka.replace("echo ! test !").unwrap();
        assert!(result.starts_with("sudo"), "Should trigger sudo with trailing exclamation: {}", result);
        assert!(result.contains("echo ! test"), "Should preserve earlier exclamation marks: {}", result);
        assert!(!result.ends_with("!"), "Should not end with exclamation mark");
    }

    #[test]
    fn test_split_respecting_quotes_with_exclamation() {
        // Test basic exclamation mark splitting
        let result = AKA::split_respecting_quotes("touch file !");
        assert_eq!(result, vec!["touch", "file", "!"]);

        // Test exclamation mark in quotes (should not be split)
        let result = AKA::split_respecting_quotes("echo \"hello !\" world");
        assert_eq!(result, vec!["echo", "\"hello !\"", "world"]);

        // Test exclamation mark at end after quotes
        let result = AKA::split_respecting_quotes("echo \"test\" !");
        assert_eq!(result, vec!["echo", "\"test\"", "!"]);

        // Test multiple exclamation marks
        let result = AKA::split_respecting_quotes("echo ! test !");
        assert_eq!(result, vec!["echo", "!", "test", "!"]);

        // Test exclamation mark in middle (should not be treated specially)
        let result = AKA::split_respecting_quotes("echo hello ! world");
        assert_eq!(result, vec!["echo", "hello", "!", "world"]);

        // Test lone exclamation mark
        let result = AKA::split_respecting_quotes("!");
        assert_eq!(result, vec!["!"]);
    }


}
