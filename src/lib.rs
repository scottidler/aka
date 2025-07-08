use eyre::{eyre, Result};
use log::{info, debug};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::{Instant, Duration};
use xxhash_rust::xxh3::xxh3_64;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

pub mod cfg;
use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

// Re-export for binaries
pub use cfg::alias::Alias as AliasType;
pub use cfg::loader::Loader as ConfigLoader;
pub use cfg::spec::Spec as ConfigSpec;

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
        let total_duration = self.start_time.elapsed();
        let config_load_duration = self.config_start.map(|start| start.elapsed());
        let ipc_duration = self.ipc_start.map(|start| start.elapsed());
        let processing_duration = self.processing_start.map(|start| start.elapsed()).unwrap_or_default();

        TimingData {
            total_duration,
            config_load_duration,
            ipc_duration,
            processing_duration,
            mode: self.mode,
            timestamp: std::time::SystemTime::now(),
        }
    }
}

impl TimingData {
    pub fn log_detailed(&self) {
        let emoji = match self.mode {
            ProcessingMode::Daemon => "👹",
            ProcessingMode::Direct => "📥",
        };

        debug!("🕒 Timing Report [{}]:", emoji);
        debug!("  📊 Total: {:.3}ms", self.total_duration.as_secs_f64() * 1000.0);

        if let Some(config_duration) = self.config_load_duration {
            debug!("  📋 Config Load: {:.3}ms", config_duration.as_secs_f64() * 1000.0);
        }

        if let Some(ipc_duration) = self.ipc_duration {
            debug!("  📡 IPC: {:.3}ms", ipc_duration.as_secs_f64() * 1000.0);
        }

        debug!("  ⚙️  Processing: {:.3}ms", self.processing_duration.as_secs_f64() * 1000.0);

        // Calculate percentages
        let total_ms = self.total_duration.as_secs_f64() * 1000.0;
        if total_ms > 0.0 {
            if let Some(config_duration) = self.config_load_duration {
                let config_ms = config_duration.as_secs_f64() * 1000.0;
                let config_pct = (config_ms / total_ms) * 100.0;
                debug!("  📋 Config Load: {:.1}%", config_pct);
            }

            if let Some(ipc_duration) = self.ipc_duration {
                let ipc_ms = ipc_duration.as_secs_f64() * 1000.0;
                let ipc_pct = (ipc_ms / total_ms) * 100.0;
                debug!("  📡 IPC: {:.1}%", ipc_pct);
            }

            let processing_ms = self.processing_duration.as_secs_f64() * 1000.0;
            let processing_pct = (processing_ms / total_ms) * 100.0;
            debug!("  ⚙️  Processing: {:.1}%", processing_pct);
        }
    }

    pub fn to_csv_line(&self) -> String {
        let mode_str = match self.mode {
            ProcessingMode::Daemon => "daemon",
            ProcessingMode::Direct => "direct",
        };

        format!("{},{:.3},{:.3},{:.3},{:.3}",
            mode_str,
            self.total_duration.as_secs_f64() * 1000.0,
            self.config_load_duration.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            self.ipc_duration.map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            self.processing_duration.as_secs_f64() * 1000.0
        )
    }
}

fn parse_csv_line(line: &str) -> Result<TimingData> {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() != 5 {
        return Err(eyre!("Invalid CSV line format"));
    }

    let mode = match parts[0] {
        "daemon" => ProcessingMode::Daemon,
        "direct" => ProcessingMode::Direct,
        _ => return Err(eyre!("Invalid mode: {}", parts[0])),
    };

    let total_duration = Duration::from_secs_f64(parts[1].parse::<f64>()? / 1000.0);
    let config_load_duration = if parts[2] == "0" {
        None
    } else {
        Some(Duration::from_secs_f64(parts[2].parse::<f64>()? / 1000.0))
    };
    let ipc_duration = if parts[3] == "0" {
        None
    } else {
        Some(Duration::from_secs_f64(parts[3].parse::<f64>()? / 1000.0))
    };
    let processing_duration = Duration::from_secs_f64(parts[4].parse::<f64>()? / 1000.0);

    Ok(TimingData {
        total_duration,
        config_load_duration,
        ipc_duration,
        processing_duration,
        mode,
        timestamp: std::time::SystemTime::now(),
    })
}

pub fn log_timing(timing: TimingData) {
    if is_benchmark_mode() {
        timing.log_detailed();

        // Also append to CSV file for analysis
        if let Ok(timing_file) = get_timing_file_path() {
            let csv_line = timing.to_csv_line();
            let header = "mode,total_ms,config_load_ms,ipc_ms,processing_ms";

            let needs_header = !timing_file.exists();
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&timing_file) {
                use std::io::Write;
                if needs_header {
                    let _ = writeln!(file, "{}", header);
                }
                let _ = writeln!(file, "{}", csv_line);
            }
        }
    }
}

pub fn export_timing_csv() -> Result<String> {
    let timing_file = get_timing_file_path()?;
    if !timing_file.exists() {
        return Ok("No timing data available".to_string());
    }

    let content = std::fs::read_to_string(&timing_file)?;
    Ok(content)
}

pub fn get_timing_summary() -> Result<(Duration, Duration, usize, usize)> {
    let timing_file = get_timing_file_path()?;
    if !timing_file.exists() {
        return Ok((Duration::from_secs(0), Duration::from_secs(0), 0, 0));
    }

    let content = std::fs::read_to_string(&timing_file)?;
    let lines: Vec<&str> = content.lines().collect();

    if lines.len() <= 1 { // Header only or empty
        return Ok((Duration::from_secs(0), Duration::from_secs(0), 0, 0));
    }

    let mut daemon_count = 0;
    let mut direct_count = 0;
    let mut total_daemon_time = Duration::from_secs(0);
    let mut total_direct_time = Duration::from_secs(0);

    for line in lines.iter().skip(1) { // Skip header
        if let Ok(timing) = parse_csv_line(line) {
            match timing.mode {
                ProcessingMode::Daemon => {
                    daemon_count += 1;
                    total_daemon_time += timing.total_duration;
                }
                ProcessingMode::Direct => {
                    direct_count += 1;
                    total_direct_time += timing.total_duration;
                }
            }
        }
    }

    let avg_daemon_time = if daemon_count > 0 {
        total_daemon_time / daemon_count as u32
    } else {
        Duration::from_secs(0)
    };

    let avg_direct_time = if direct_count > 0 {
        total_direct_time / direct_count as u32
    } else {
        Duration::from_secs(0)
    };

    Ok((avg_daemon_time, avg_direct_time, daemon_count, direct_count))
}

pub fn get_timing_file_path() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka")
        .join("logs");

    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("timing.csv"))
}

pub fn get_config_path() -> Result<PathBuf> {
    let config_path = dirs::config_dir()
        .ok_or_else(|| eyre!("Could not determine config directory"))?
        .join("aka")
        .join("aka.yml");

    if config_path.exists() {
        Ok(config_path)
    } else {
        eprintln!("Error: Config file not found at {:?}", config_path);
        eprintln!("Please create the config file first.");
        Err(eyre!("Config file {:?} not found", config_path))
    }
}

pub fn test_config(file: &PathBuf) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.clone());
    }
    Err(eyre!("config {:?} not found!", file))
}

pub fn setup_logging() -> Result<()> {
    if is_benchmark_mode() {
        // In benchmark mode, log to stdout for visibility
        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Stdout)
            .init();
    } else {
        // In normal mode, log to file
        let log_dir = dirs::data_local_dir()
            .ok_or_else(|| eyre!("Could not determine local data directory"))?
            .join("aka")
            .join("logs");

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

pub fn execute_health_check(config: &Option<PathBuf>) -> Result<i32> {
    use std::os::unix::net::UnixStream;
    use std::io::{BufRead, BufReader, Write};
    use serde_json;

    debug!("🔍 Starting comprehensive health check");
    debug!("🔍 Health check input: config = {:?}", config);

    // Step 1: Check daemon health first
    debug!("📋 Step 1: Checking daemon health");
    if let Ok(socket_path) = determine_socket_path() {
        debug!("🔌 Socket path determined: {:?}", socket_path);
        if socket_path.exists() {
            debug!("✅ Daemon socket exists, testing connection");

            // Try to connect and send health request
            match UnixStream::connect(&socket_path) {
                Ok(mut stream) => {
                    debug!("🔗 Connected to daemon successfully, sending health request");

                    let health_request = r#"{"type":"Health"}"#;
                    debug!("📤 Sending health request: {}", health_request);

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
                                            return Ok(0); // Daemon healthy - best case
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
                }
                Err(e) => {
                    debug!("⚠️ Failed to connect to daemon socket: {}", e);
                }
            }
            debug!("❌ Daemon socket exists but health check failed");
        } else {
            debug!("❌ Daemon socket not found at path: {:?}", socket_path);
        }
    } else {
        debug!("❌ Cannot determine daemon socket path");
    }

    // Step 2: Daemon not available, validate config directly
    debug!("📋 Step 2: Daemon unavailable, validating config directly");

    let config_path = match config {
        Some(file) => {
            debug!("🔍 Using specified config file: {:?}", file);
            if !file.exists() {
                debug!("❌ Health check failed: specified config file {:?} not found", file);
                debug!("🎯 Health check result: CONFIG_NOT_FOUND (returning 1)");
                return Ok(1); // Config file not found
            }
            file.clone()
        }
        None => {
            debug!("🔍 No config specified, using default config path");
            let default_config = get_config_path();
            match default_config {
                Ok(path) => {
                    debug!("✅ Default config path resolved: {:?}", path);
                    path
                }
                Err(e) => {
                    debug!("❌ Health check failed: no config file found: {}", e);
                    debug!("🎯 Health check result: CONFIG_NOT_FOUND (returning 1)");
                    return Ok(1); // Config file not found
                }
            }
        }
    };

    // Step 3: Try to load and validate the config
    debug!("📋 Step 3: Attempting to load and validate config");

    let loader = Loader::new();
    debug!("🔄 Loading config from: {:?}", config_path);
    match loader.load(&config_path) {
        Ok(spec) => {
            debug!("✅ Config loaded successfully");

            // Check if we have any aliases
            if spec.aliases.is_empty() {
                debug!("⚠️ Config valid but no aliases defined");
                debug!("🎯 Health check result: NO_ALIASES (returning 3)");
                return Ok(3); // No aliases defined
            }

            debug!("✅ Config valid with {} aliases", spec.aliases.len());
            debug!("🎯 Health check result: CONFIG_VALID (returning 0)");
            Ok(0) // All good
        }
        Err(e) => {
            debug!("❌ Health check failed: config file invalid: {}", e);
            debug!("🚨 Config validation failed - aka cannot be used");
            debug!("🎯 Health check result: CONFIG_INVALID (returning 2)");
            Ok(2) // Config file invalid - critical failure
        }
    }
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
    pub cache_dir: Option<PathBuf>,
}

impl AKA {
    pub fn new(eol: bool, config: &Option<PathBuf>) -> Result<Self> {
        Self::new_with_cache_dir(eol, config, None)
    }

    pub fn new_with_cache_dir(eol: bool, config: &Option<PathBuf>, cache_dir: Option<&PathBuf>) -> Result<Self> {
        use std::time::Instant;

        let start_total = Instant::now();

        // Time config path resolution
        let start_path = Instant::now();
        let config_path = match config {
            Some(file) => test_config(file)?,
            None => get_config_path()?,
        };
        let path_duration = start_path.elapsed();

        // Calculate config hash
        let config_hash = hash_config_file(&config_path)?;
        debug!("🔒 Config hash: {}", config_hash);

        // Time loader creation and config loading
        let start_load = Instant::now();
        let loader = Loader::new();
        let mut spec = loader.load(&config_path)?;
        let load_duration = start_load.elapsed();

        // Try to load from cache first
        let start_cache = Instant::now();
        if let Some(cached_aliases) = load_alias_cache_with_base(&config_hash, cache_dir)? {
            debug!("📋 Using cached aliases with usage counts");
            debug!("📋 Cache loaded {} aliases", cached_aliases.len());
            // Log a sample alias count for debugging
            if let Some((name, alias)) = cached_aliases.iter().next() {
                debug!("📋 Sample alias '{}' has count: {}", name, alias.count);
            }
            spec.aliases = cached_aliases;
        } else {
            debug!("📋 No cache found, initializing usage counts to 0");
            // Initialize all counts to 0 (they already are due to skip_deserializing) and save to cache
            save_alias_cache_with_base(&config_hash, &spec.aliases, cache_dir)?;
        }
        let cache_duration = start_cache.elapsed();

        let total_duration = start_total.elapsed();

        debug!("🏗️  AKA::new() timing breakdown:");
        debug!("  📂 Path resolution: {:.3}ms", path_duration.as_secs_f64() * 1000.0);
        debug!("  📋 Config loading: {:.3}ms", load_duration.as_secs_f64() * 1000.0);
        debug!("  🗃️  Cache handling: {:.3}ms", cache_duration.as_secs_f64() * 1000.0);
        debug!("  🎯 Total AKA::new(): {:.3}ms", total_duration.as_secs_f64() * 1000.0);

        Ok(AKA { eol, spec, config_hash, cache_dir: cache_dir.cloned() })
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

            let (value, count) = if should_use_alias {
                // Now we can safely get mutable reference
                if let Some(alias) = self.spec.aliases.get_mut(&current_arg) {
                    if (alias.global && cmdline.contains(&alias.value))
                        || (!alias.global && pos == 0 && cmdline.starts_with(&alias.value))
                    {
                        (current_arg.clone(), 0)
                    } else {
                        space = if alias.space { " " } else { "" };
                        let (v, c) = alias.replace(&mut remainders)?;
                        if v != alias.name {
                            replaced = true;
                            // Increment usage count when alias is actually used
                            alias.count += 1;
                            debug!("📊 Alias '{}' used, count now: {}", alias.name, alias.count);
                        }
                        (v, c)
                    }
                } else {
                    (current_arg.clone(), 0)
                }
            } else {
                (current_arg.clone(), 0)
            };

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
                if let Err(e) = save_alias_cache_with_base(&self.config_hash, &self.spec.aliases, self.cache_dir.as_ref()) {
                    debug!("⚠️ Failed to save alias cache: {}", e);
                }
            }
        }

        Ok(result)
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
pub fn determine_socket_path() -> Result<PathBuf> {
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

pub fn get_alias_cache_path(config_hash: &str) -> Result<PathBuf> {
    get_alias_cache_path_with_base(config_hash, None)
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

pub fn load_alias_cache(config_hash: &str) -> Result<Option<HashMap<String, Alias>>> {
    load_alias_cache_with_base(config_hash, None)
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

pub fn save_alias_cache(config_hash: &str, aliases: &HashMap<String, Alias>) -> Result<()> {
    save_alias_cache_with_base(config_hash, aliases, None)
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
