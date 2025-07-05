use eyre::{eyre, Result};
use log::{info, debug};
use std::fs::OpenOptions;
use std::path::PathBuf;
use xxhash_rust::xxh3::xxh3_64;

pub mod cfg;
use cfg::alias::Alias;
use cfg::loader::Loader;
use cfg::spec::Spec;

// Re-export for binaries
pub use cfg::alias::Alias as AliasType;
pub use cfg::loader::Loader as ConfigLoader;
pub use cfg::spec::Spec as ConfigSpec;

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

    Ok(())
}

pub fn get_hash_cache_path() -> Result<PathBuf> {
    let cache_dir = dirs::data_local_dir()
        .ok_or_else(|| eyre!("Could not determine local data directory"))?
        .join("aka");

    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("config.hash"))
}

pub fn hash_config_file(config_path: &PathBuf) -> Result<String> {
    let content = std::fs::read(config_path)?;
    let hash = xxh3_64(&content);
    Ok(format!("{:016x}", hash))
}

pub fn get_stored_hash() -> Result<Option<String>> {
    let hash_path = get_hash_cache_path()?;
    if hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path)?;
        Ok(Some(stored_hash.trim().to_string()))
    } else {
        Ok(None)
    }
}

pub fn store_hash(hash: &str) -> Result<()> {
    let hash_path = get_hash_cache_path()?;
    std::fs::write(&hash_path, hash)?;
    Ok(())
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

    // Step 2: Daemon not available, check config file cache
    debug!("📋 Step 2: Daemon unavailable, checking config cache");

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
    let stored_hash = get_stored_hash().unwrap_or(None);

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

    // Try to load and parse the config
    let loader = Loader::new();
    debug!("🔄 Loading fresh config from: {:?}", config_path);
    match loader.load(&config_path) {
        Ok(spec) => {
            debug!("✅ Fresh config loaded successfully");

            // Config is valid, store the new hash
            if let Err(e) = store_hash(&current_hash) {
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
}

impl AKA {
    pub fn new(eol: bool, config: &Option<PathBuf>) -> Result<Self> {
        use std::time::Instant;

        let start_total = Instant::now();

        // Time config path resolution
        let start_path = Instant::now();
        let config_path = match config {
            Some(file) => test_config(file)?,
            None => get_config_path()?,
        };
        let path_duration = start_path.elapsed();

        // Time loader creation and config loading
        let start_load = Instant::now();
        let loader = Loader::new();
        let spec = loader.load(&config_path)?;
        let load_duration = start_load.elapsed();

        let total_duration = start_total.elapsed();

        debug!("🏗️  AKA::new() timing breakdown:");
        debug!("  📂 Path resolution: {:.3}ms", path_duration.as_secs_f64() * 1000.0);
        debug!("  📋 Config loading: {:.3}ms", load_duration.as_secs_f64() * 1000.0);
        debug!("  🎯 Total AKA::new(): {:.3}ms", total_duration.as_secs_f64() * 1000.0);

        Ok(AKA { eol, spec })
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

    pub fn replace(&self, cmdline: &str) -> Result<String> {
        self.replace_with_mode(cmdline, ProcessingMode::Direct)
    }

    pub fn replace_with_mode(&self, cmdline: &str, mode: ProcessingMode) -> Result<String> {
        debug!("Processing command line: {}", cmdline);
        let mut pos: usize = 0;
        let mut space = " ";
        let mut replaced = false;
        let mut sudo = false;
        let mut args = Self::split_respecting_quotes(cmdline);

        if self.eol && !args.is_empty() {
            if let Some(last_arg) = args.last() {
                if last_arg == "!" || last_arg.ends_with("!") {
                    args.pop();
                    sudo = true;
                } else if last_arg.starts_with("!") {
                    let next_arg = last_arg[1..].to_string();
                    args[0] = next_arg;
                    replaced = true;

                    let mut i = 1;
                    while i < args.len() {
                        if args[i].starts_with("-") {
                            args.remove(i);
                        } else if args[i] == "|" || args[i] == ">" || args[i] == "<" {
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    args.pop();
                }
            }
        }

        while pos < args.len() {
            let current_arg = args[pos].clone(); // Clone to avoid borrowing conflicts

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
            let (value, count) = match self.spec.aliases.get(&current_arg) {
                Some(alias) if self.use_alias(alias, pos) => {
                    if (alias.global && cmdline.contains(&alias.value))
                        || (!alias.global && pos == 0 && cmdline.starts_with(&alias.value))
                    {
                        (current_arg.clone(), 0)
                    } else {
                        space = if alias.space { " " } else { "" };
                        let (v, c) = alias.replace(&mut remainders)?;
                        if v != alias.name {
                            replaced = true;
                        }
                        (v, c)
                    }
                }
                Some(_) | None => (current_arg.clone(), 0),
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
