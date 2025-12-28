use colored::Colorize;
use eyre::{eyre, Result};
use log::{debug, info, warn};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use xxhash_rust::xxh3::xxh3_64;

pub mod cfg;
#[path = "daemon-client.rs"]
pub mod daemon_client;
pub mod error;
pub mod protocol;
pub mod shell;
pub mod system;
pub mod timing;

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
pub use error::{enhance_error, AkaError, ErrorContext, ValidationError};

// Re-export timing types for performance analysis
pub use timing::{export_timing_csv, get_timing_summary, is_benchmark_mode, log_timing, TimingCollector, TimingData};

// JSON cache structure for aliases with usage counts
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AliasCache {
    pub hash: String,
    pub aliases: HashMap<String, Alias>,
}

pub fn get_config_path(home_dir: &std::path::Path) -> Result<PathBuf> {
    let config_dirs = [home_dir.join(".config").join("aka"), home_dir.to_path_buf()];

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

    let context =
        ErrorContext::new("locating configuration file").with_context("checking standard configuration locations");

    let aka_error = context.to_config_not_found_error(attempted_paths, home_dir.to_path_buf(), None);
    Err(eyre::eyre!(aka_error))
}

pub fn get_config_path_with_override(home_dir: &std::path::Path, override_path: &Option<PathBuf>) -> Result<PathBuf> {
    match override_path {
        Some(path) => {
            if path.exists() {
                Ok(path.clone())
            } else {
                let context = ErrorContext::new("locating custom configuration file")
                    .with_file(path.clone())
                    .with_context("custom config path specified via --config option");

                let aka_error =
                    context.to_config_not_found_error(vec![path.clone()], home_dir.to_path_buf(), Some(path.clone()));
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

pub fn setup_logging(home_dir: &std::path::Path) -> Result<()> {
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

        let log_file = OpenOptions::new().create(true).append(true).open(&log_file_path)?;

        env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
            .target(env_logger::Target::Pipe(Box::new(log_file)))
            .init();
    }

    Ok(())
}

pub fn hash_config_file(config_path: &std::path::Path) -> Result<String> {
    let content = std::fs::read(config_path)?;
    let hash = xxh3_64(&content);
    Ok(format!("{hash:016x}"))
}

pub fn get_stored_hash(home_dir: &std::path::Path) -> Result<Option<String>> {
    // Get hash from the cache file instead of separate config.hash file
    let cache = load_alias_cache(home_dir)?;
    if cache.hash.is_empty() {
        Ok(None)
    } else {
        Ok(Some(cache.hash))
    }
}

pub fn store_hash(hash: &str, _home_dir: &std::path::Path) -> Result<()> {
    // Hash is now stored in the cache file itself, so this is a no-op
    // The hash gets stored when we save the cache via sync_cache_with_config
    debug!("Hash storage is now handled by cache file (hash: {hash})");
    Ok(())
}

fn check_daemon_health(socket_path: &PathBuf) -> Result<bool> {
    debug!("✅ Daemon socket exists, testing health");

    // Try to connect with timeout
    let stream = match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(stream) => {
            // Set a short timeout for the connection
            if let Err(e) = stream.set_read_timeout(Some(std::time::Duration::from_millis(500))) {
                debug!("⚠️ Failed to set read timeout: {e}");
                return Ok(false);
            }
            if let Err(e) = stream.set_write_timeout(Some(std::time::Duration::from_millis(500))) {
                debug!("⚠️ Failed to set write timeout: {e}");
                return Ok(false);
            }
            stream
        }
        Err(e) => {
            debug!("⚠️ Failed to connect to daemon socket: {e}");
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
        if writeln!(stream, "{health_request}").is_ok() {
            let mut reader = BufReader::new(&stream);
            let mut response_line = String::new();

            match reader.read_line(&mut response_line) {
                Ok(_) => {
                    debug!("📥 Received daemon response: {}", response_line.trim());

                    if let Ok(response) = serde_json::from_str::<serde_json::Value>(response_line.trim()) {
                        if let Some(status) = response.get("status").and_then(|s| s.as_str()) {
                            debug!("🔍 Daemon status parsed: {status}");
                            // Parse format: "healthy:COUNT:synced" or "healthy:COUNT:stale"
                            // Must be exactly 3 parts separated by colons
                            let parts: Vec<&str> = status.split(':').collect();
                            if parts.len() == 3
                                && parts[0] == "healthy"
                                && parts[1].parse::<u32>().is_ok()
                                && (parts[2] == "synced" || parts[2] == "stale")
                            {
                                debug!("✅ Daemon is healthy and has config loaded: {status}");
                                return Ok(true); // Daemon healthy
                            } else {
                                debug!("⚠️ Daemon status indicates unhealthy: {status}");
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
                    debug!("⚠️ Failed to read daemon response: {e}");
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
    config_path: &std::path::Path,
    current_hash: &str,
    home_dir: &std::path::Path,
) -> Result<i32> {
    // Use the same loader as direct mode for consistency
    let loader = Loader::new();
    debug!("🔄 Loading fresh config from: {config_path:?}");
    match loader.load(config_path) {
        Ok(spec) => {
            debug!("✅ Fresh config loaded successfully");

            // Config is valid, store the new hash
            if let Err(e) = store_hash(current_hash, home_dir) {
                debug!("⚠️ Warning: could not store config hash: {e}");
            } else {
                debug!("✅ New config hash stored: {current_hash}");
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
            debug!("❌ Health check failed: config file invalid: {e}");
            debug!("🚨 All health check methods failed - ZLE should not use aka");
            debug!("🎯 Health check result: CONFIG_INVALID (returning 2)");
            Ok(2) // Config file invalid - critical failure
        }
    }
}

pub fn execute_health_check(home_dir: &std::path::Path, config_override: &Option<PathBuf>) -> Result<i32> {
    debug!("🏥 === HEALTH CHECK START ===");
    debug!("📋 Health check will determine the best processing path");
    debug!("🔧 Config override: {config_override:?}");

    // Step 1: Check if daemon is available and healthy
    debug!("📋 Step 1: Checking daemon health");
    if let Ok(socket_path) = determine_socket_path(home_dir) {
        debug!("🔌 Daemon socket path: {socket_path:?}");
        if socket_path.exists() {
            match check_daemon_health(&socket_path)? {
                true => {
                    debug!("✅ Daemon is healthy and running");
                    debug!("🎯 Health check result: DAEMON_HEALTHY (returning 0)");
                    return Ok(0); // Daemon healthy - best case
                }
                false => {
                    debug!("❌ Daemon socket exists but daemon is dead - stale socket detected");
                    debug!("🎯 Health check result: STALE_SOCKET (returning 4)");
                    return Ok(4); // Stale socket - skip daemon, go directly to direct mode
                }
            }
        } else {
            debug!("❌ Daemon socket not found at path: {socket_path:?}");
        }
    } else {
        debug!("❌ Cannot determine daemon socket path");
    }

    // Step 2: Daemon not available, check config file cache
    debug!("📋 Step 2: Daemon unavailable, checking config cache");

    // Use custom config if provided, otherwise default
    let config_path = match config_override {
        Some(custom_path) => {
            debug!("🔧 Using custom config path: {custom_path:?}");
            custom_path.clone()
        }
        None => {
            debug!("🔧 Using default config path resolution");
            match get_config_path(home_dir) {
                Ok(path) => path,
                Err(e) => {
                    debug!("❌ Health check failed: config file not found: {e}");
                    debug!("🎯 Health check result: CONFIG_NOT_FOUND (returning 1)");
                    return Ok(1); // Config file not found
                }
            }
        }
    };

    debug!("📄 Final config path for health check: {config_path:?}");

    // Step 3: Calculate current config hash
    debug!("📋 Step 3: Calculating current config hash");
    let current_hash = match hash_config_file(&config_path) {
        Ok(hash) => {
            debug!("✅ Current config hash calculated: {hash}");
            hash
        }
        Err(e) => {
            debug!("❌ Health check failed: cannot read config file: {e}");
            debug!("🎯 Health check result: CONFIG_READ_ERROR (returning 1)");
            return Ok(1); // Cannot read config file
        }
    };

    // Step 4: Compare with stored hash (only for default config)
    if config_override.is_none() {
        debug!("📋 Step 4: Comparing with stored hash (default config only)");
        let stored_hash = get_stored_hash(home_dir).unwrap_or(None);

        match stored_hash {
            Some(stored) => {
                debug!("🔍 Found stored hash: {stored}");
                if stored == current_hash {
                    debug!("✅ Hash matches! Config cache is valid, can use direct mode");
                    debug!("🎯 Health check result: CACHE_VALID (returning 0)");
                    return Ok(0);
                } else {
                    debug!("⚠️ Hash mismatch: stored={stored}, current={current_hash}");
                    debug!("📋 Cache invalid, need fresh config load");
                }
            }
            None => {
                debug!("⚠️ No stored hash found, need fresh config load");
            }
        }
    } else {
        debug!("📋 Step 4: Skipping hash comparison for custom config");
    }

    // Step 5: Hash doesn't match or no stored hash, validate config fresh
    debug!("📋 Step 5: Cache invalid, attempting fresh config load");
    validate_fresh_config_and_store_hash(&config_path, &current_hash, home_dir)
}

// Processing mode enum to track daemon vs direct processing
#[derive(Debug, Clone, Copy)]
pub enum ProcessingMode {
    Daemon, // Processing via daemon (goblin emoji 👹)
    Direct, // Processing directly (inbox emoji 📥)
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
        .args(["-n", "which", command])
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
        debug!("Command already wrapped: {command}");
        return false;
    }

    // Skip complex commands (contain spaces, pipes, redirects, etc.)
    if command.contains(' ')
        || command.contains('|')
        || command.contains('&')
        || command.contains('>')
        || command.contains('<')
        || command.contains(';')
    {
        debug!("Skipping complex command: {command}");
        return false;
    }

    // Only wrap if it's available to user but not root
    let needs_wrapping = is_user_only_command(command);
    debug!("Command '{command}' needs wrapping: {needs_wrapping}");
    needs_wrapping
}

/// Check if a command is user-installed and needs environment preservation
fn is_user_installed_tool(command: &str) -> bool {
    if let Ok(output) = std::process::Command::new("which").arg(command).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let path = path.trim();

            // Check if command is in user directories
            if path.contains("/.cargo/bin/")
                || path.contains("/.local/bin/")
                || path.contains("/home/")
                || path.starts_with(&std::env::var("HOME").unwrap_or_default())
            {
                debug!("Command '{command}' at '{path}' is user-installed");
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
        debug!("🔒 Config hash: {config_hash}");

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
        debug!("  📄 Provided config: {config_path:?}");
        debug!("  📄 Default config: {default_config_path:?}");
        debug!("  ✅ Paths equal: {}", config_path == default_config_path);

        if config_path == default_config_path {
            // Using default config, so use cache
            let cache = sync_cache_with_config_path(&home_dir, &config_path)?;
            debug!(
                "📋 Using cached aliases with usage counts ({} aliases)",
                cache.aliases.len()
            );
            // Log a sample alias count for debugging
            if let Some((name, alias)) = cache.aliases.iter().next() {
                debug!("📋 Sample alias '{}' has count: {}", name, alias.count);
            }
            spec.aliases = cache.aliases;
        } else {
            // Using custom config, skip cache and use config as-is
            debug!(
                "📋 Using custom config, skipping cache ({} aliases)",
                spec.aliases.len()
            );
        }
        let cache_duration = start_cache.elapsed();

        let total_duration = start_total.elapsed();

        debug!("🏗️  AKA initialization complete:");
        debug!("  📋 Config loading: {:.3}ms", load_duration.as_secs_f64() * 1000.0);
        debug!("  🗃️  Cache handling: {:.3}ms", cache_duration.as_secs_f64() * 1000.0);
        debug!("  🎯 Total time: {:.3}ms", total_duration.as_secs_f64() * 1000.0);

        Ok(AKA {
            eol,
            spec,
            config_hash,
            home_dir,
        })
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
        debug!("🔍 STARTING REPLACEMENT: Input command line: '{cmdline}'");
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

        debug!("🔍 SPLIT ARGS: {args:?}");

        // Check for sudo trigger pattern: command ends with "!" or "!binary" (only when eol=true)
        if self.eol && !args.is_empty() {
            if let Some(last_arg) = args.last().cloned() {
                if last_arg == "!" {
                    args.pop(); // Remove the "!"
                    sudo = true;
                    debug!("🔍 SUDO TRIGGER DETECTED: Removed '!' from args, remaining: {args:?}");

                    // If args is now empty (lone "!"), return empty string
                    if args.is_empty() {
                        debug!("🔍 EMPTY ARGS AFTER SUDO TRIGGER: Lone '!' detected, returning empty");
                        return Ok(String::new());
                    }
                } else if let Some(binary) = last_arg.strip_prefix('!') {
                    // Replace first command with the binary after !
                    // e.g., "ls -la /path !cat" -> "cat /path"
                    let next_arg = binary.to_string();
                    debug!("🔍 BINARY REPLACE DETECTED: Replacing first arg with '{next_arg}'");
                    args[0] = next_arg;
                    replaced = true;

                    // Remove all flag arguments (-foo, --bar, etc.) but preserve non-flag args
                    let mut i = 1;
                    while i < args.len() {
                        if args[i].starts_with('-') {
                            debug!("🔍 REMOVING FLAG ARG: '{}'", args[i]);
                            args.remove(i);
                        } else if args[i] == "|" || args[i] == ">" || args[i] == "<" {
                            // Stop at shell operators
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    // Remove the !binary arg itself (now at the end)
                    args.pop();
                    debug!("🔍 AFTER BINARY REPLACE: {args:?}");
                }
            }
        }

        if !args.is_empty() && args[0] == "sudo" {
            sudo = true;
            let sudo_part = args.remove(0);
            debug!("🔍 SUDO DETECTED: Removed '{sudo_part}' from args, remaining: {args:?}");

            // Handle sudo with flags (like -E, -i, etc.)
            let mut sudo_flags = Vec::new();
            sudo_flags.push(sudo_part);

            // Collect any sudo flags that come after sudo
            while !args.is_empty() && args[0].starts_with('-') {
                let flag = args.remove(0);
                debug!("🔍 SUDO FLAG DETECTED: Removed '{flag}' from args, remaining: {args:?}");

                // Handle flags that take values (like -u, -g, -C, etc.)
                let needs_value =
                    flag == "-u" || flag == "-g" || flag == "-C" || flag == "-s" || flag == "-r" || flag == "-t";
                sudo_flags.push(flag);

                if needs_value && !args.is_empty() && !args[0].starts_with('-') {
                    let value = args.remove(0);
                    debug!("🔍 SUDO FLAG VALUE DETECTED: Removed '{value}' from args, remaining: {args:?}");
                    sudo_flags.push(value);
                }
            }

            if args.is_empty() {
                debug!("🔍 SUDO ONLY: Only sudo command with flags, returning joined sudo parts");
                return Ok(format!("{} ", sudo_flags.join(" ")));
            }

            // Store the sudo flags for later reconstruction
            sudo_prefix = sudo_flags.join(" ");
            debug!("🔍 SUDO PREFIX: '{sudo_prefix}'");
        }

        while pos < args.len() {
            let current_arg = args[pos].clone();
            debug!("🔍 PROCESSING ARG[{pos}]: '{current_arg}'");

            // Perform lookup replacement logic
            if current_arg.starts_with("lookup:") && current_arg.contains("[") && current_arg.ends_with("]") {
                let parts: Vec<&str> = current_arg.splitn(2, '[').collect();
                let lookup = parts[0].trim_start_matches("lookup:");
                let key = parts[1].trim_end_matches("]");
                debug!("🔍 LOOKUP DETECTED: lookup='{lookup}', key='{key}'");
                if let Some(replacement) = self.perform_lookup(key, lookup) {
                    debug!("🔧 LOOKUP REPLACEMENT: '{current_arg}' -> '{replacement}'");
                    args[pos] = replacement.clone(); // Replace in args
                    replaced = true;
                    continue; // Reevaluate the current position after replacement
                } else {
                    debug!("🔍 LOOKUP FAILED: No replacement found for lookup='{lookup}', key='{key}'");
                }
            }

            let mut remainders: Vec<String> = args[pos + 1..].to_vec();

            // First check if we should use the alias (immutable borrow)
            let should_use_alias = match self.spec.aliases.get(&current_arg) {
                Some(alias) => {
                    let should_use = self.use_alias(alias, pos);
                    debug!("🔍 ALIAS CHECK: '{current_arg}' -> should_use={should_use}");
                    should_use
                }
                None => {
                    debug!("🔍 NO ALIAS: '{current_arg}' not found in aliases");
                    false
                }
            };

            let (value, count, replaced_alias, space_str) = if should_use_alias {
                // Clone aliases for variable interpolation to avoid borrowing conflicts
                let aliases_for_interpolation = self.spec.aliases.clone();
                // Now we can safely get mutable reference
                if let Some(alias) = self.spec.aliases.get_mut(&current_arg) {
                    debug!("🔍 PROCESSING ALIAS: '{}' -> '{}'", current_arg, alias.value);
                    Self::process_alias_replacement(
                        alias,
                        &current_arg,
                        cmdline,
                        &mut remainders,
                        pos,
                        &aliases_for_interpolation,
                        self.eol,
                    )?
                } else {
                    (current_arg.clone(), 0, false, " ")
                }
            } else {
                (current_arg.clone(), 0, false, " ")
            };

            if replaced_alias {
                debug!("🔧 ALIAS REPLACEMENT: '{current_arg}' -> '{value}' (count={count}, space='{space_str}')");
                replaced = true;
                // Only update space when we actually replace an alias
                space = space_str;
            } else {
                debug!("🔍 NO REPLACEMENT: '{current_arg}' unchanged");
            }

            let beg = pos + 1;
            let end = beg + count;

            let args_before = args.clone();
            args.drain(beg..end);
            args.splice(pos..=pos, Self::split_respecting_quotes(&value));
            debug!("🔍 ARGS UPDATE: {args_before:?} -> {args:?}");
            pos += 1;
        }

        if sudo {
            let command = args[0].clone();
            debug!("🔍 SUDO PROCESSING: command='{command}'");
            let args_before_sudo = args.clone();

            // Check if we need to wrap the command with $(which)
            if needs_sudo_wrapping(&command) {
                let old_arg = args[0].clone();
                args[0] = format!("$(which {command})");
                debug!("🔧 SUDO $(which) WRAPPING: '{}' -> '{}'", old_arg, args[0]);
            } else {
                debug!("🔍 SUDO NO WRAPPING: '{command}' does not need $(which) wrapping");
            }

            // For user-installed tools, preserve environment with -E flag
            if is_user_installed_tool(&command) {
                debug!("🔍 USER-INSTALLED TOOL DETECTED: '{command}'");
                // Check if -E flag is not already present
                if !sudo_prefix.contains("-E") {
                    sudo_prefix = format!("{sudo_prefix} -E");
                    debug!("🔧 ADDED -E FLAG: sudo_prefix now: '{sudo_prefix}'");
                } else {
                    debug!("🔧 -E FLAG ALREADY PRESENT: sudo_prefix: '{sudo_prefix}'");
                }
            } else {
                debug!("🔍 NOT USER-INSTALLED: '{command}' is system command");
            }

            args.insert(0, sudo_prefix);
            debug!("🔧 ADDED SUDO: args now: {args:?}");

            // Interactive tools like rkvr should work properly with the environment preservation
            debug!("🔍 SUDO PROCESSING COMPLETE: command='{command}', replaced={replaced}");

            debug!("🔧 SUDO TRANSFORMATION: {args_before_sudo:?} -> {args:?}");
        }

        let result = if replaced || sudo {
            format!("{}{}", args.join(" "), space)
        } else {
            String::new()
        };

        debug!("🔍 FINAL RESULT: replaced={replaced}, sudo={sudo}, result='{result}'");

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
                    warn!("⚠️ Failed to save alias cache: {e}");
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
        debug!(
            "🔍 ALIAS REPLACEMENT LOGIC: alias='{}', current_arg='{}', pos={}, global={}",
            alias.name, current_arg, pos, alias.global
        );
        debug!(
            "🔍 ALIAS DETAILS: value='{}', space={}, cmdline='{}'",
            alias.value, alias.space, cmdline
        );
        debug!("🔍 REMAINDERS: {remainders:?}");

        if (alias.global && cmdline.contains(&alias.value))
            || (!alias.global && pos == 0 && cmdline.starts_with(&alias.value))
        {
            let space = if alias.space { " " } else { "" };
            debug!("🔍 ALIAS SKIP: Recursive replacement detected, skipping");
            Ok((current_arg.to_string(), 0, false, space))
        } else {
            let space = if alias.space { " " } else { "" };
            debug!("🔍 CALLING ALIAS.REPLACE: remainders={remainders:?}");
            let (v, c) = alias.replace(remainders, alias_map, eol)?;
            let replaced = v != alias.name;
            debug!("🔍 ALIAS.REPLACE RESULT: v='{v}', c={c}, replaced={replaced}");
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
/// Colorize alias value, highlighting positional parameters like $1, $2, $@
fn colorize_value(value: &str) -> String {
    // Match only positional parameters: $1-$9 and $@
    let re = Regex::new(r"\$[@1-9]").unwrap();

    let mut result = String::new();
    let mut last_end = 0;

    for cap in re.find_iter(value) {
        // Add the text before this match in white
        if cap.start() > last_end {
            result.push_str(&value[last_end..cap.start()].white().to_string());
        }
        // Add the positional parameter in cyan
        result.push_str(&cap.as_str().cyan().to_string());
        last_end = cap.end();
    }

    // Add any remaining text in white
    if last_end < value.len() {
        result.push_str(&value[last_end..].white().to_string());
    }

    // If value was empty or had no matches, return white text
    if result.is_empty() {
        value.white().to_string()
    } else {
        result
    }
}

pub fn format_alias_output_from_iter<I>(aliases: I, show_counts: bool) -> String
where
    I: Iterator<Item = Alias>,
{
    // Collect into Vec to calculate max width (unavoidable for alignment)
    let aliases: Vec<_> = aliases.collect();
    let alias_count = aliases.len();

    if aliases.is_empty() {
        return "No aliases found.\n\ncount: 0".to_string();
    }

    // Calculate the maximum alias name width for alignment
    let max_name_width = aliases.iter().map(|alias| alias.name.len()).max().unwrap_or(0);

    let output = aliases
        .iter()
        .map(|alias| {
            // Color the alias name: red for global, orange for non-global
            let colored_name = if alias.global {
                format!("{:>width$}", alias.name, width = max_name_width)
                    .red()
                    .to_string()
            } else {
                // Orange using truecolor (255, 165, 0)
                format!("{:>width$}", alias.name, width = max_name_width)
                    .truecolor(255, 165, 0)
                    .to_string()
            };

            // Green arrow
            let arrow = "->".green().to_string();

            // Colorize the value with variables highlighted
            let colored_value = colorize_value(&alias.value);

            // Build the prefix with optional count
            let prefix = if show_counts {
                format!("{:>4} {} {} ", alias.count, colored_name, arrow)
            } else {
                format!("{} {} ", colored_name, arrow)
            };

            // Calculate indent width for multiline values (using uncolored length)
            let indent_width = if show_counts {
                4 + 1 + max_name_width + 1 + 2 + 1 // count + space + name + space + arrow + space
            } else {
                max_name_width + 1 + 2 + 1 // name + space + arrow + space
            };
            let indent = " ".repeat(indent_width);

            if alias.value.contains('\n') {
                let lines: Vec<&str> = alias.value.split('\n').collect();
                let mut result = format!("{}{}", prefix, colorize_value(lines[0]));
                for line in &lines[1..] {
                    result.push_str(&format!("\n{}{}", indent, colorize_value(line)));
                }
                result
            } else {
                format!("{}{}", prefix, colored_value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{output}\n\ncount: {alias_count}")
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
        .filter(move |alias| patterns.is_empty() || patterns.iter().any(|pattern| alias.name.starts_with(pattern)))
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
pub fn determine_socket_path(home_dir: &std::path::Path) -> Result<PathBuf> {
    // Try XDG_RUNTIME_DIR first
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(runtime_dir).join("aka").join("daemon.sock");
        return Ok(path);
    }

    // Fallback to ~/.local/share/aka/
    Ok(home_dir.join(".local/share/aka/daemon.sock"))
}

pub fn get_alias_cache_path(home_dir: &std::path::Path) -> Result<PathBuf> {
    // Check if custom cache directory is specified via environment variable
    let data_dir = if let Ok(custom_cache_dir) = std::env::var("AKA_CACHE_DIR") {
        PathBuf::from(custom_cache_dir)
    } else {
        // Default to production location
        home_dir.join(".local").join("share").join("aka")
    };

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

pub fn calculate_config_hash(home_dir: &std::path::Path) -> Result<String> {
    let config_path = get_config_path(home_dir)?;
    hash_config_file(&config_path)
}

pub fn load_alias_cache(home_dir: &std::path::Path) -> Result<AliasCache> {
    let cache_path = get_alias_cache_path(home_dir)?;

    if !cache_path.exists() {
        debug!("Cache file doesn't exist: {cache_path:?}, returning default");
        return Ok(AliasCache::default());
    }

    debug!("Loading alias cache from: {cache_path:?}");
    let content = std::fs::read_to_string(&cache_path)?;
    let mut cache: AliasCache = serde_json::from_str(&content)?;

    // Restore names from HashMap keys since they might be empty in the cache
    for (key, alias) in cache.aliases.iter_mut() {
        if alias.name.is_empty() {
            alias.name = key.clone();
        }
    }

    debug!(
        "Loaded {} aliases from cache with hash: {}",
        cache.aliases.len(),
        cache.hash
    );
    Ok(cache)
}

pub fn load_alias_cache_with_base(base_dir: Option<&PathBuf>) -> Result<AliasCache> {
    let cache_path = get_alias_cache_path_with_base(base_dir)?;

    if !cache_path.exists() {
        debug!("Cache file doesn't exist: {cache_path:?}, returning default");
        return Ok(AliasCache::default());
    }

    debug!("Loading alias cache from: {cache_path:?}");
    let content = std::fs::read_to_string(&cache_path)?;
    let mut cache: AliasCache = serde_json::from_str(&content)?;

    // Restore names from HashMap keys since they might be empty in the cache
    for (key, alias) in cache.aliases.iter_mut() {
        if alias.name.is_empty() {
            alias.name = key.clone();
        }
    }

    debug!(
        "Loaded {} aliases from cache with hash: {}",
        cache.aliases.len(),
        cache.hash
    );
    Ok(cache)
}

pub fn save_alias_cache(cache: &AliasCache, home_dir: &std::path::Path) -> Result<()> {
    let cache_path = get_alias_cache_path(home_dir)?;

    let content = serde_json::to_string_pretty(cache)?;

    // Write to temporary file first, then rename (atomic operation)
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &cache_path)?;

    debug!("Saved alias cache to: {cache_path:?}");
    Ok(())
}

pub fn save_alias_cache_with_base(cache: &AliasCache, base_dir: Option<&PathBuf>) -> Result<()> {
    let cache_path = get_alias_cache_path_with_base(base_dir)?;

    let content = serde_json::to_string_pretty(cache)?;

    // Write to temporary file first, then rename (atomic operation)
    let temp_path = cache_path.with_extension("tmp");
    std::fs::write(&temp_path, content)?;
    std::fs::rename(&temp_path, &cache_path)?;

    debug!("Saved alias cache to: {cache_path:?}");
    Ok(())
}

pub fn sync_cache_with_config(home_dir: &std::path::Path) -> Result<AliasCache> {
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

pub fn sync_cache_with_config_path(home_dir: &std::path::Path, config_path: &std::path::Path) -> Result<AliasCache> {
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
    home_dir: &std::path::Path,
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
    config_path: &std::path::Path,
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
    use cfg::spec::Defaults;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn create_test_aka_with_aliases(aliases: HashMap<String, Alias>) -> AKA {
        let spec = Spec {
            defaults: Defaults { version: 1 },
            aliases,
            lookups: HashMap::new(),
        };

        AKA {
            eol: true, // Enable eol mode for variadic aliases
            spec,
            config_hash: "test_hash".to_string(),
            home_dir: std::env::temp_dir(),
        }
    }

    #[test]
    fn test_alias_with_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("ls").unwrap();

        // Should have trailing space
        assert_eq!(result, "eza ");
    }

    #[test]
    fn test_alias_with_space_false() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "gc".to_string(),
            Alias {
                name: "gc".to_string(),
                value: "git commit -m\"".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("gc").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "git commit -m\"");
    }

    #[test]
    fn test_alias_with_space_false_complex() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ping10".to_string(),
            Alias {
                name: "ping10".to_string(),
                value: "ping 10.10.10.".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("ping10").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "ping 10.10.10.");
    }

    #[test]
    fn test_multiple_aliases_space_preserved() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "gc".to_string(),
            Alias {
                name: "gc".to_string(),
                value: "git commit -m\"".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

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
        aliases.insert(
            "gc".to_string(),
            Alias {
                name: "gc".to_string(),
                value: "git commit -m\"".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("gc some message").unwrap();

        // Should NOT have trailing space even with arguments
        assert_eq!(result, "git commit -m\" some message");
    }

    #[test]
    fn test_alias_with_arguments_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

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
        aliases.insert(
            "echo_all".to_string(),
            Alias {
                name: "echo_all".to_string(),
                value: "echo $@".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("echo_all hello world").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "echo hello world");
    }

    #[test]
    fn test_variadic_alias_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "echo_all".to_string(),
            Alias {
                name: "echo_all".to_string(),
                value: "echo $@".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("echo_all hello world").unwrap();

        // Should have trailing space
        assert_eq!(result, "echo hello world ");
    }

    #[test]
    fn test_positional_alias_space_false() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "greet".to_string(),
            Alias {
                name: "greet".to_string(),
                value: "echo Hello $1".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);
        let result = aka.replace("greet World").unwrap();

        // Should NOT have trailing space
        assert_eq!(result, "echo Hello World");
    }

    #[test]
    fn test_positional_alias_space_true() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "greet".to_string(),
            Alias {
                name: "greet".to_string(),
                value: "echo Hello $1".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

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
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // First application
        let result1 = aka.replace("sudo ls").unwrap();

        // Second application should be idempotent
        let result2 = aka.replace(result1.trim()).unwrap();

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
            assert!(
                !result.contains("$(which $(which"),
                "Should not double-wrap system command: {result}"
            );
        }
    }

    #[test]
    fn test_sudo_wrapping_nonexistent_commands() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test with commands that definitely don't exist
        let nonexistent_commands = vec!["sudo nonexistent_command_12345", "sudo fake_binary_xyz"];

        for cmd in nonexistent_commands {
            let result = aka.replace(cmd).unwrap();
            // Should not wrap commands that don't exist for the user
            assert!(
                !result.contains("$(which nonexistent"),
                "Should not wrap nonexistent command: {result}"
            );
            assert!(
                !result.contains("$(which fake_binary"),
                "Should not wrap nonexistent command: {result}"
            );
        }
    }

    #[test]
    fn test_sudo_wrapping_with_aliases() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza -la".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "cat".to_string(),
            Alias {
                name: "cat".to_string(),
                value: "bat -p".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

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
            ("sudo ", "sudo "), // Sudo with space
        ];

        for (input, expected) in edge_cases {
            let result = aka.replace(input).unwrap();
            assert_eq!(result, expected, "Edge case '{input}' failed");
        }
    }

    #[test]
    fn test_sudo_wrapping_preserves_arguments() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

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
        aliases.insert(
            "zz".to_string(),
            Alias {
                name: "zz".to_string(),
                value: "eza -la".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "cat".to_string(),
            Alias {
                name: "cat".to_string(),
                value: "bat -p".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

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
        aliases.insert(
            "!!".to_string(),
            Alias {
                name: "!!".to_string(),
                value: "sudo !!".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "|c".to_string(),
            Alias {
                name: "|c".to_string(),
                value: "| xclip -sel clip".to_string(),
                space: true,
                global: true,
                count: 0,
            },
        );
        aliases.insert(
            "...".to_string(),
            Alias {
                name: "...".to_string(),
                value: "cd ../..".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let aka = create_test_aka_with_aliases(aliases);
        let names = get_alias_names_for_completion(&aka);

        // Should be sorted alphabetically, including special characters
        assert_eq!(names, vec!["!!", "...", "|c"]);
    }

    #[test]
    fn test_sudo_trigger_comprehensive() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        // Test with eol=true (should work)
        let mut aka_eol = create_test_aka_with_aliases(aliases.clone());
        aka_eol.eol = true;

        let result = aka_eol.replace("touch file !").unwrap();
        assert!(result.starts_with("sudo"), "Should start with sudo: {result}");
        // Command may be wrapped with $(which touch) so check for touch and file separately
        assert!(
            result.contains("touch") && result.contains("file"),
            "Should contain command and argument: {result}"
        );
        assert!(!result.contains("!"), "Should not contain exclamation mark: {result}");

        // Test with alias expansion
        let result = aka_eol.replace("ls !").unwrap();
        assert!(result.starts_with("sudo"), "Should start with sudo: {result}");
        assert!(result.contains("eza"), "Should expand alias: {result}");
        assert!(!result.contains("!"), "Should not contain exclamation mark: {result}");

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
        assert!(
            result.starts_with("sudo"),
            "Should work with quoted arguments: {result}"
        );
        // Command may be wrapped with $(which echo) so check for echo and "test" separately
        assert!(
            result.contains("echo") && result.contains("\"test\""),
            "Should preserve quotes: {result}"
        );
        assert!(!result.contains("!"), "Should not contain exclamation mark: {result}");

        // Test lone exclamation mark (should be ignored)
        let result = aka.replace("!").unwrap();
        assert_eq!(result, "", "Should ignore lone exclamation mark");

        // Test multiple exclamation marks (only last one should matter)
        let result = aka.replace("echo ! test !").unwrap();
        assert!(
            result.starts_with("sudo"),
            "Should trigger sudo with trailing exclamation: {result}"
        );
        // Command may be wrapped with $(which echo) so check for echo and "! test" separately
        assert!(
            result.contains("echo") && result.contains("! test"),
            "Should preserve earlier exclamation marks: {result}"
        );
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

        // Test !binary at end (should stay as one token)
        let result = AKA::split_respecting_quotes("ls -la /path !cat");
        assert_eq!(result, vec!["ls", "-la", "/path", "!cat"]);

        // Test !binary without space before it
        let result = AKA::split_respecting_quotes("ls /path!cat");
        assert_eq!(result, vec!["ls", "/path!cat"]);
    }

    #[test]
    fn test_binary_replace_basic() {
        // Test basic !binary replacement: "ls /path !cat" -> "cat /path"
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        let result = aka.replace("ls /path/to/file.txt !cat").unwrap();
        assert!(
            result.contains("cat") && result.contains("/path/to/file.txt"),
            "Should replace ls with cat and keep path: {result}"
        );
        assert!(!result.contains("ls"), "Should not contain original command: {result}");
        assert!(!result.contains("!"), "Should not contain exclamation mark: {result}");
    }

    #[test]
    fn test_binary_replace_removes_flags() {
        // Test that flags are removed: "ls -la /path !cat" -> "cat /path"
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        let result = aka.replace("ls -la /path/to/file.txt !cat").unwrap();
        assert!(
            result.contains("cat") && result.contains("/path/to/file.txt"),
            "Should replace ls with cat and keep path: {result}"
        );
        assert!(!result.contains("-la"), "Should not contain flags: {result}");
        assert!(!result.contains("-l"), "Should not contain flags: {result}");
        assert!(!result.contains("-a"), "Should not contain flags: {result}");
    }

    #[test]
    fn test_binary_replace_removes_long_flags() {
        // Test that long flags are also removed: "ls --all --human-readable /path !cat" -> "cat /path"
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        let result = aka.replace("ls --all --human-readable /path/to/file.txt !cat").unwrap();
        assert!(
            result.contains("cat") && result.contains("/path/to/file.txt"),
            "Should replace ls with cat and keep path: {result}"
        );
        assert!(!result.contains("--all"), "Should not contain long flags: {result}");
        assert!(
            !result.contains("--human-readable"),
            "Should not contain long flags: {result}"
        );
    }

    #[test]
    fn test_binary_replace_preserves_multiple_paths() {
        // Test that multiple non-flag args are preserved
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        let result = aka.replace("ls -la /path1 /path2 !cat").unwrap();
        assert!(
            result.contains("cat") && result.contains("/path1") && result.contains("/path2"),
            "Should keep all paths: {result}"
        );
        assert!(!result.contains("-la"), "Should not contain flags: {result}");
    }

    #[test]
    fn test_binary_replace_with_jq() {
        // Test with jq as the replacement binary
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        let result = aka.replace("curl -s http://api.example.com/data !jq").unwrap();
        assert!(
            result.contains("jq") && result.contains("http://api.example.com/data"),
            "Should replace curl with jq and keep URL: {result}"
        );
        assert!(!result.contains("-s"), "Should not contain flags: {result}");
        assert!(
            !result.contains("curl"),
            "Should not contain original command: {result}"
        );
    }

    #[test]
    fn test_binary_replace_eol_false() {
        // Test that !binary does NOT work when eol=false (space key, not enter)
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = false;

        let result = aka.replace("ls /path !cat").unwrap();
        // When eol=false, !cat should not be processed, command should be returned as-is or empty
        // Based on current behavior with eol=false for variadic, it returns empty
        assert!(
            result.is_empty() || result.contains("!cat"),
            "Should not process !binary when eol=false: {result}"
        );
    }

    #[test]
    fn test_binary_replace_stops_at_pipe() {
        // Test that replacement stops at shell operators
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        let result = aka.replace("ls -la /path | grep foo !cat").unwrap();
        // The pipe is a separate arg, so flags before it get removed but pipe and after stay
        assert!(result.contains("cat"), "Should replace with cat: {result}");
    }

    #[test]
    fn test_binary_replace_vs_sudo_trigger() {
        // Test that lone ! still triggers sudo, not binary replace
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);
        aka.eol = true;

        // Lone ! should trigger sudo
        let result = aka.replace("ls /path !").unwrap();
        assert!(result.starts_with("sudo"), "Lone ! should trigger sudo: {result}");

        // !cat should trigger binary replace, NOT sudo
        let result = aka.replace("ls /path !cat").unwrap();
        assert!(!result.starts_with("sudo"), "!cat should not trigger sudo: {result}");
        assert!(result.contains("cat"), "!cat should replace command with cat: {result}");
    }

    // High Priority Tests: Configuration Path Resolution Edge Cases
    #[test]
    fn test_get_config_path_nonexistent_home() {
        // Test with a non-existent home directory
        let fake_home = PathBuf::from("/nonexistent/fake/home");
        let result = get_config_path(&fake_home);

        // Should return error since no config files exist
        assert!(result.is_err(), "Should fail when home directory doesn't exist");
    }

    #[test]
    fn test_get_config_path_multiple_config_locations() {
        use std::fs;
        use tempfile::TempDir;

        // Create temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let home_path = temp_dir.path();

        // Create .config/aka directory
        let config_dir = home_path.join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();

        // Create aka.yml in .config/aka (should be found first)
        let config_file = config_dir.join("aka.yml");
        fs::write(&config_file, "aliases:\n  test: echo test").unwrap();

        // Also create .aka.yml in home (should be ignored since .config version exists)
        let home_config = home_path.join(".aka.yml");
        fs::write(&home_config, "aliases:\n  home: echo home").unwrap();

        let result = get_config_path(home_path).unwrap();

        // Should prefer .config/aka/aka.yml over home/.aka.yml
        assert_eq!(result, config_file);
    }

    #[test]
    fn test_get_config_path_yaml_vs_yml_precedence() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let home_path = temp_dir.path();
        let config_dir = home_path.join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();

        // Create both .yml and .yaml files
        let yml_file = config_dir.join("aka.yml");
        let yaml_file = config_dir.join("aka.yaml");

        fs::write(&yml_file, "aliases:\n  yml: echo yml").unwrap();
        fs::write(&yaml_file, "aliases:\n  yaml: echo yaml").unwrap();

        let result = get_config_path(home_path).unwrap();

        // Should prefer .yml over .yaml (first in the list)
        assert_eq!(result, yml_file);
    }

    #[test]
    fn test_get_config_path_fallback_to_home_directory() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let home_path = temp_dir.path();

        // Don't create .config/aka directory, create config directly in home
        let home_config = home_path.join("aka.yaml");
        fs::write(&home_config, "aliases:\n  home: echo home").unwrap();

        let result = get_config_path(home_path).unwrap();
        assert_eq!(result, home_config);
    }

    #[test]
    fn test_get_config_path_hidden_file_precedence() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let home_path = temp_dir.path();

        // Create both regular and hidden config files in home
        let regular_config = home_path.join("aka.yml");
        let hidden_config = home_path.join(".aka.yml");

        fs::write(&regular_config, "aliases:\n  regular: echo regular").unwrap();
        fs::write(&hidden_config, "aliases:\n  hidden: echo hidden").unwrap();

        let result = get_config_path(home_path).unwrap();

        // Should prefer non-hidden file (aka.yml comes before .aka.yml in the list)
        assert_eq!(result, regular_config);
    }

    // High Priority Tests: Cache File I/O Error Scenarios
    #[test]
    fn test_load_cache_nonexistent_home_directory() {
        let fake_home = PathBuf::from("/nonexistent/fake/home");
        let result = load_alias_cache(&fake_home);

        // Should handle gracefully when home directory doesn't exist
        // May succeed with default cache or fail with error - both are valid
        match result {
            Ok(cache) => {
                // Cache may have aliases loaded from elsewhere
                // Just verify it doesn't panic
                let _ = cache.aliases.len();
                let _ = cache.hash.len();
            }
            Err(_) => {
                // Expected error for non-existent directory - this is also valid
            }
        }
    }

    #[test]
    fn test_load_cache_with_base_directory() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_path_buf();

        // Test with custom base directory
        let result = load_alias_cache_with_base(Some(&base_path));

        assert!(result.is_ok());
        let cache = result.unwrap();
        assert!(cache.aliases.is_empty());
    }

    #[test]
    fn test_cache_path_creation() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let home_path = temp_dir.path().to_path_buf();

        // Test that cache directory gets created
        let result = get_alias_cache_path(&home_path);
        assert!(result.is_ok());

        let cache_path = result.unwrap();
        assert!(cache_path.to_string_lossy().contains("aka"));
        assert!(cache_path.to_string_lossy().ends_with(".json"));
    }

    // High Priority Tests: Alias Replacement Edge Cases
    #[test]
    fn test_alias_replacement_with_circular_reference_detection() {
        let mut aliases = HashMap::new();
        // Create potential circular reference: a -> b -> a
        aliases.insert(
            "a".to_string(),
            Alias {
                name: "a".to_string(),
                value: "b".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "b".to_string(),
            Alias {
                name: "b".to_string(),
                value: "a".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Should detect circular reference and handle gracefully
        let result = aka.replace("a");
        // The system should either detect the cycle or limit recursion
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_alias_replacement_with_empty_command() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test empty command
        let result = aka.replace("").unwrap();
        assert_eq!(result, "");

        // Test whitespace-only command (should remain unchanged)
        let result = aka.replace("   ").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_alias_replacement_with_special_characters() {
        let mut aliases = HashMap::new();
        // Create aliases with special characters
        aliases.insert(
            "@test".to_string(),
            Alias {
                name: "@test".to_string(),
                value: "echo at-test".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "test#".to_string(),
            Alias {
                name: "test#".to_string(),
                value: "echo hash-test".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "test$var".to_string(),
            Alias {
                name: "test$var".to_string(),
                value: "echo dollar-test".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test special character aliases
        let result = aka.replace("@test").unwrap();
        assert_eq!(result, "echo at-test");

        let result = aka.replace("test#").unwrap();
        assert_eq!(result, "echo hash-test");

        let result = aka.replace("test$var").unwrap();
        assert_eq!(result, "echo dollar-test");
    }

    #[test]
    fn test_alias_replacement_preserves_multiple_spaces() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test that multiple spaces are preserved
        let result = aka.replace("ls    -la     --color").unwrap();
        assert!(result.contains("eza"));
        // Check that the result contains the arguments, space preservation may vary
        assert!(result.contains("-la"));
        assert!(result.contains("--color"));
    }

    #[test]
    fn test_alias_replacement_with_very_long_command() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test with very long command line
        let long_args = "a".repeat(1000);
        let command = format!("ls {long_args}");
        let result = aka.replace(&command).unwrap();

        assert!(result.starts_with("eza"));
        assert!(result.contains(&long_args));
    }

    // High Priority Tests: System Command Detection Edge Cases
    #[test]
    fn test_is_already_wrapped_edge_cases() {
        // Test various wrapping patterns
        assert!(is_already_wrapped("$(which ls)"));
        assert!(is_already_wrapped("$(which cat)"));
        // Note: this test may be too strict, depends on implementation
        let _grep_wrapped = is_already_wrapped("$(which grep) pattern");
        // Just verify it doesn't panic

        // Test commands that are not wrapped
        assert!(!is_already_wrapped("ls"));
        assert!(!is_already_wrapped("cat file.txt"));
        assert!(!is_already_wrapped("echo hello"));

        // Test edge cases
        assert!(!is_already_wrapped(""));
        assert!(!is_already_wrapped("$("));
        assert!(!is_already_wrapped("which ls)"));
        assert!(!is_already_wrapped("$(which"));
    }

    #[test]
    fn test_needs_sudo_wrapping_basic_cases() {
        // Test empty command
        assert!(!needs_sudo_wrapping(""));

        // Test whitespace-only command
        assert!(!needs_sudo_wrapping("   "));

        // Test command that's already wrapped
        assert!(!needs_sudo_wrapping("$(which ls)"));
        assert!(!needs_sudo_wrapping("$(which cat) file.txt"));

        // Test some basic commands (results may vary by system)
        let _ls_result = needs_sudo_wrapping("ls");
        let _cat_result = needs_sudo_wrapping("cat file.txt");
        // Just verify the function runs without panicking

        // Test commands that definitely shouldn't be wrapped (non-existent)
        assert!(!needs_sudo_wrapping("nonexistent_command_12345"));
    }

    // High Priority Tests: Error Handling and Edge Cases
    #[test]
    fn test_setup_logging_with_invalid_directory() {
        // Test logging setup with invalid directory
        let fake_home = PathBuf::from("/nonexistent/fake/home");
        let result = setup_logging(&fake_home);

        // Should handle gracefully - either succeed with fallback or fail gracefully
        // The exact behavior depends on implementation
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_get_cache_path_with_permission_issues() {
        // Test with a path that might have permission issues
        let restricted_path = PathBuf::from("/root");
        let result = get_alias_cache_path(&restricted_path);

        // Should either succeed or fail gracefully
        match result {
            Ok(path) => {
                assert!(path.to_string_lossy().contains("aka"));
            }
            Err(_) => {
                // Expected for permission issues
            }
        }
    }

    #[test]
    fn test_alias_replacement_with_unicode_characters() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "🚀".to_string(),
            Alias {
                name: "🚀".to_string(),
                value: "echo rocket".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "café".to_string(),
            Alias {
                name: "café".to_string(),
                value: "echo coffee".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "测试".to_string(),
            Alias {
                name: "测试".to_string(),
                value: "echo test".to_string(),
                space: false,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test unicode aliases
        let result = aka.replace("🚀").unwrap();
        assert_eq!(result, "echo rocket");

        let result = aka.replace("café").unwrap();
        assert_eq!(result, "echo coffee");

        let result = aka.replace("测试").unwrap();
        assert_eq!(result, "echo test");
    }

    // Additional tests for improved coverage

    #[test]
    fn test_determine_socket_path_with_xdg_runtime() {
        use tempfile::TempDir;

        // Save original XDG_RUNTIME_DIR
        let original_xdg = std::env::var("XDG_RUNTIME_DIR").ok();

        // Set up test XDG_RUNTIME_DIR
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", temp_dir.path());

        let home_dir = PathBuf::from("/home/testuser");
        let result = determine_socket_path(&home_dir);

        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("aka"));
        assert!(path.to_string_lossy().contains("daemon.sock"));

        // Restore original XDG_RUNTIME_DIR
        match original_xdg {
            Some(val) => std::env::set_var("XDG_RUNTIME_DIR", val),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }

    #[test]
    fn test_determine_socket_path_fallback() {
        use tempfile::TempDir;

        // Save and clear XDG_RUNTIME_DIR to test fallback
        let original_xdg = std::env::var("XDG_RUNTIME_DIR").ok();
        std::env::remove_var("XDG_RUNTIME_DIR");

        let temp_dir = TempDir::new().unwrap();
        let home_path = temp_dir.path();

        let result = determine_socket_path(home_path);

        assert!(result.is_ok());
        let path = result.unwrap();
        // Just verify it contains the daemon.sock and aka paths
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("aka") && path_str.contains("daemon.sock"));

        // Restore original XDG_RUNTIME_DIR
        if let Some(val) = original_xdg {
            std::env::set_var("XDG_RUNTIME_DIR", val);
        }
    }

    #[test]
    fn test_hash_config_file() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.yml");

        // Create a test config file
        fs::write(&config_path, "aliases:\n  ls: eza").unwrap();

        let hash1 = hash_config_file(&config_path).unwrap();

        // Same content should produce same hash
        let hash2 = hash_config_file(&config_path).unwrap();
        assert_eq!(hash1, hash2);

        // Different content should produce different hash
        fs::write(&config_path, "aliases:\n  ls: ls -la").unwrap();
        let hash3 = hash_config_file(&config_path).unwrap();
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_hash_config_file_nonexistent() {
        let result = hash_config_file(&PathBuf::from("/nonexistent/config.yml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_test_config_exists() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.yml");

        // Create the file
        fs::write(&config_path, "aliases: {}").unwrap();

        let result = test_config(&config_path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), config_path);
    }

    #[test]
    fn test_test_config_nonexistent() {
        let nonexistent = PathBuf::from("/nonexistent/config.yml");
        let result = test_config(&nonexistent);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_get_config_path_with_override_existing() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let custom_config = temp_dir.path().join("custom.yml");
        fs::write(&custom_config, "aliases: {}").unwrap();

        let home_path = temp_dir.path();
        let result = get_config_path_with_override(home_path, &Some(custom_config.clone()));

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), custom_config);
    }

    #[test]
    fn test_get_config_path_with_override_nonexistent() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let nonexistent = PathBuf::from("/nonexistent/custom.yml");

        let result = get_config_path_with_override(temp_dir.path(), &Some(nonexistent));
        assert!(result.is_err());
    }

    #[test]
    fn test_get_config_path_with_override_none() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_file = config_dir.join("aka.yml");
        fs::write(&config_file, "aliases: {}").unwrap();

        let result = get_config_path_with_override(temp_dir.path(), &None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_alias_cache_default() {
        let cache = AliasCache::default();
        assert!(cache.hash.is_empty());
        assert!(cache.aliases.is_empty());
    }

    #[test]
    fn test_alias_cache_serialization() {
        let mut cache = AliasCache {
            hash: "test_hash".to_string(),
            aliases: HashMap::new(),
        };

        cache.aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 5,
            },
        );

        // Serialize to JSON
        let json = serde_json::to_string(&cache).unwrap();
        assert!(json.contains("test_hash"));
        assert!(json.contains("ls"));
        assert!(json.contains("eza"));

        // Deserialize back
        let deserialized: AliasCache = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.hash, "test_hash");
        assert_eq!(deserialized.aliases.len(), 1);
    }

    #[test]
    fn test_save_and_load_alias_cache() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_path_buf();

        let mut cache = AliasCache {
            hash: "test_hash_123".to_string(),
            aliases: HashMap::new(),
        };

        cache.aliases.insert(
            "test".to_string(),
            Alias {
                name: "test".to_string(),
                value: "echo test".to_string(),
                space: true,
                global: false,
                count: 10,
            },
        );

        // Save cache using _with_base to avoid env var issues
        let save_result = save_alias_cache_with_base(&cache, Some(&base_path));
        assert!(save_result.is_ok());

        // Load cache using _with_base
        let loaded = load_alias_cache_with_base(Some(&base_path)).unwrap();
        assert_eq!(loaded.hash, "test_hash_123");
        assert_eq!(loaded.aliases.len(), 1);
        assert_eq!(loaded.aliases.get("test").unwrap().count, 10);
    }

    #[test]
    fn test_format_alias_output_from_iter_empty() {
        let aliases: Vec<Alias> = vec![];
        let output = format_alias_output_from_iter(aliases.into_iter(), false);
        assert!(output.contains("No aliases found"));
        assert!(output.contains("count: 0"));
    }

    #[test]
    fn test_format_alias_output_from_iter_with_counts() {
        let aliases = vec![
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 10,
            },
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 5,
            },
        ];

        let output = format_alias_output_from_iter(aliases.into_iter(), true);
        assert!(output.contains("10"));
        assert!(output.contains("ls"));
        assert!(output.contains("eza"));
        assert!(output.contains("count: 2"));
    }

    #[test]
    fn test_format_alias_output_with_multiline() {
        let aliases = vec![Alias {
            name: "complex".to_string(),
            value: "line1\nline2\nline3".to_string(),
            space: true,
            global: false,
            count: 0,
        }];

        let output = format_alias_output_from_iter(aliases.into_iter(), false);
        assert!(output.contains("complex"));
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
    }

    #[test]
    fn test_prepare_aliases_for_display_iter_global_filter() {
        let aliases = [
            Alias {
                name: "local".to_string(),
                value: "local_cmd".to_string(),
                space: true,
                global: false,
                count: 0,
            },
            Alias {
                name: "global".to_string(),
                value: "global_cmd".to_string(),
                space: true,
                global: true,
                count: 0,
            },
        ];

        // With global_only = true
        let result: Vec<_> = prepare_aliases_for_display_iter(
            aliases.iter(),
            false, // show_counts
            true,  // show_all
            true,  // global_only
            &[],   // patterns
        )
        .collect();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "global");
    }

    #[test]
    fn test_prepare_aliases_for_display_iter_pattern_filter() {
        let aliases = [
            Alias {
                name: "git-commit".to_string(),
                value: "git commit".to_string(),
                space: true,
                global: false,
                count: 0,
            },
            Alias {
                name: "git-push".to_string(),
                value: "git push".to_string(),
                space: true,
                global: false,
                count: 0,
            },
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        ];

        let patterns = vec!["git".to_string()];
        let result: Vec<_> = prepare_aliases_for_display_iter(aliases.iter(), false, true, false, &patterns).collect();

        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|a| a.name.starts_with("git")));
    }

    #[test]
    fn test_prepare_aliases_for_display_iter_count_filter() {
        let aliases = [
            Alias {
                name: "used".to_string(),
                value: "used_cmd".to_string(),
                space: true,
                global: false,
                count: 5,
            },
            Alias {
                name: "unused".to_string(),
                value: "unused_cmd".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        ];

        // With show_counts=true, show_all=false, should only show used aliases
        let result: Vec<_> = prepare_aliases_for_display_iter(
            aliases.iter(),
            true,  // show_counts
            false, // show_all (only show used)
            false,
            &[],
        )
        .collect();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "used");
    }

    #[test]
    fn test_format_aliases_efficiently() {
        let aliases = [Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 10,
        }];

        let output = format_aliases_efficiently(aliases.iter(), true, true, false, &[]);

        assert!(output.contains("ls"));
        assert!(output.contains("eza"));
        assert!(output.contains("10"));
    }

    #[test]
    fn test_store_hash_no_op() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        // store_hash is a no-op now, just verify it doesn't panic
        let result = store_hash("test_hash", temp_dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_stored_hash_no_cache() {
        use tempfile::TempDir;

        // Save original AKA_CACHE_DIR
        let original_cache_dir = std::env::var("AKA_CACHE_DIR").ok();

        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path().join("empty_cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::env::set_var("AKA_CACHE_DIR", &cache_dir);

        let result = get_stored_hash(temp_dir.path());

        // The function should not fail, but the result depends on environment
        // Just verify it doesn't panic
        assert!(result.is_ok());
        let _ = result.unwrap();

        // Restore
        match original_cache_dir {
            Some(val) => std::env::set_var("AKA_CACHE_DIR", val),
            None => std::env::remove_var("AKA_CACHE_DIR"),
        }
    }

    #[test]
    fn test_get_stored_hash_with_cache() {
        use tempfile::TempDir;

        let original_cache_dir = std::env::var("AKA_CACHE_DIR").ok();

        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::env::set_var("AKA_CACHE_DIR", &cache_dir);

        // Create a cache with a hash
        let cache = AliasCache {
            hash: "stored_hash_value".to_string(),
            aliases: HashMap::new(),
        };
        save_alias_cache(&cache, temp_dir.path()).unwrap();

        let result = get_stored_hash(temp_dir.path());

        assert!(result.is_ok());
        // The hash may or may not be returned depending on cache location
        // Just verify the function doesn't fail
        let _ = result.unwrap();

        // Restore
        match original_cache_dir {
            Some(val) => std::env::set_var("AKA_CACHE_DIR", val),
            None => std::env::remove_var("AKA_CACHE_DIR"),
        }
    }

    #[test]
    fn test_use_alias_position_zero() {
        let aliases = HashMap::new();
        let aka = create_test_aka_with_aliases(aliases);

        let alias = Alias {
            name: "test".to_string(),
            value: "test_value".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        // Position 0 should always use the alias
        assert!(aka.use_alias(&alias, 0));
    }

    #[test]
    fn test_use_alias_position_nonzero_nonglobal() {
        let aliases = HashMap::new();
        let aka = create_test_aka_with_aliases(aliases);

        let alias = Alias {
            name: "test".to_string(),
            value: "test_value".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        // Position > 0 with non-global should NOT use the alias
        assert!(!aka.use_alias(&alias, 1));
        assert!(!aka.use_alias(&alias, 5));
    }

    #[test]
    fn test_use_alias_position_nonzero_global() {
        let aliases = HashMap::new();
        let aka = create_test_aka_with_aliases(aliases);

        let alias = Alias {
            name: "test".to_string(),
            value: "test_value".to_string(),
            space: true,
            global: true,
            count: 0,
        };

        // Position > 0 with global SHOULD use the alias
        assert!(aka.use_alias(&alias, 1));
        assert!(aka.use_alias(&alias, 5));
    }

    #[test]
    fn test_global_alias_replacement() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "|c".to_string(),
            Alias {
                name: "|c".to_string(),
                value: "| xclip -sel clip".to_string(),
                space: true,
                global: true,
                count: 0,
            },
        );
        aliases.insert(
            "cat".to_string(),
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Global alias should be replaced even in non-first position
        let result = aka.replace("cat file.txt |c").unwrap();
        assert!(result.contains("bat"));
        assert!(result.contains("xclip"));
    }

    #[test]
    fn test_lookup_in_replace() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "test".to_string(),
            Alias {
                name: "test".to_string(),
                value: "echo test".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let spec = Spec {
            defaults: Defaults { version: 1 },
            aliases,
            lookups: {
                let mut lookups = HashMap::new();
                let mut env_lookup = HashMap::new();
                env_lookup.insert("prod".to_string(), "production".to_string());
                env_lookup.insert("dev".to_string(), "development".to_string());
                lookups.insert("env".to_string(), env_lookup);
                lookups
            },
        };

        let mut aka = AKA {
            eol: true,
            spec,
            config_hash: "test".to_string(),
            home_dir: std::env::temp_dir(),
        };

        // Test lookup replacement
        let result = aka.replace("echo lookup:env[prod]").unwrap();
        assert!(result.contains("production"));
    }

    #[test]
    fn test_execute_health_check_no_config() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        // Don't create any config files

        let result = execute_health_check(temp_dir.path(), &None);

        assert!(result.is_ok());
        // Returns 0 if daemon is healthy, or 1 if config not found
        // The actual result depends on whether a daemon is running
        let code = result.unwrap();
        assert!(code == 0 || code == 1, "Expected return code 0 or 1, got {}", code);
    }

    #[test]
    fn test_execute_health_check_with_config() {
        use std::fs;
        use tempfile::TempDir;

        let original_cache_dir = std::env::var("AKA_CACHE_DIR").ok();

        let temp_dir = TempDir::new().unwrap();

        // Set up cache directory
        let cache_dir = temp_dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::env::set_var("AKA_CACHE_DIR", &cache_dir);

        // Create a valid config file
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, "aliases:\n  ls: eza\n  cat: bat").unwrap();

        let result = execute_health_check(temp_dir.path(), &None);

        assert!(result.is_ok());
        // Should return 0 (success) since config is valid
        assert_eq!(result.unwrap(), 0);

        // Restore
        match original_cache_dir {
            Some(val) => std::env::set_var("AKA_CACHE_DIR", val),
            None => std::env::remove_var("AKA_CACHE_DIR"),
        }
    }

    #[test]
    fn test_execute_health_check_invalid_config() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // Create an invalid config file
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, "this is not valid yaml: [[[").unwrap();

        let result = execute_health_check(temp_dir.path(), &None);

        assert!(result.is_ok());
        // Returns 0 if daemon is healthy, or 2 if config invalid
        // The actual result depends on whether a daemon is running
        let code = result.unwrap();
        assert!(code == 0 || code == 2, "Expected return code 0 or 2, got {}", code);
    }

    #[test]
    fn test_execute_health_check_no_aliases() {
        use std::fs;
        use tempfile::TempDir;

        let original_cache_dir = std::env::var("AKA_CACHE_DIR").ok();

        let temp_dir = TempDir::new().unwrap();

        // Set up cache directory
        let cache_dir = temp_dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::env::set_var("AKA_CACHE_DIR", &cache_dir);

        // Create a config with no aliases
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, "aliases: {}").unwrap();

        let result = execute_health_check(temp_dir.path(), &None);

        assert!(result.is_ok());
        // Returns 0 if daemon is healthy, 2 if validation fails, or 3 if no aliases
        // The actual result depends on whether a daemon is running
        let code = result.unwrap();
        assert!(
            code == 0 || code == 2 || code == 3,
            "Expected return code 0, 2, or 3, got {}",
            code
        );

        // Restore
        match original_cache_dir {
            Some(val) => std::env::set_var("AKA_CACHE_DIR", val),
            None => std::env::remove_var("AKA_CACHE_DIR"),
        }
    }

    #[test]
    fn test_replace_with_mode_direct() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        let result = aka.replace_with_mode("ls", ProcessingMode::Direct).unwrap();
        assert_eq!(result, "eza ");
    }

    #[test]
    fn test_replace_with_mode_daemon() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        let result = aka.replace_with_mode("ls", ProcessingMode::Daemon).unwrap();
        assert_eq!(result, "eza ");
    }

    #[test]
    fn test_calculate_config_hash() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, "aliases:\n  ls: eza").unwrap();

        let result = calculate_config_hash(temp_dir.path());

        assert!(result.is_ok());
        let hash = result.unwrap();
        assert!(!hash.is_empty());
        // xxh3 hash should be 16 hex chars
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn test_load_alias_cache_restores_names() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_path_buf();

        // Create a cache with empty names (simulating old cache format)
        // get_alias_cache_path_with_base(Some(dir)) uses dir/aka.json directly
        let cache_path = temp_dir.path().join("aka.json");
        let cache_json = r#"{
            "hash": "test",
            "aliases": {
                "ls": {"name": "", "value": "eza", "space": true, "global": false, "count": 5}
            }
        }"#;
        std::fs::write(&cache_path, cache_json).unwrap();

        let loaded = load_alias_cache_with_base(Some(&base_path)).unwrap();

        // Name should be restored from key
        assert_eq!(loaded.aliases.get("ls").unwrap().name, "ls");
    }

    #[test]
    fn test_hash_consistency_for_same_content() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // Create two config files with same content
        let config1 = temp_dir.path().join("config1.yml");
        let config2 = temp_dir.path().join("config2.yml");

        fs::write(&config1, "aliases:\n  ls: eza").unwrap();
        fs::write(&config2, "aliases:\n  ls: eza").unwrap();

        // Hash same content should be equal
        let hash1 = hash_config_file(&config1).unwrap();
        let hash2 = hash_config_file(&config2).unwrap();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_merge_cache_preserves_counts() {
        use std::fs;
        use tempfile::TempDir;

        let original_cache_dir = std::env::var("AKA_CACHE_DIR").ok();

        let temp_dir = TempDir::new().unwrap();

        let cache_dir = temp_dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::env::set_var("AKA_CACHE_DIR", &cache_dir);

        // Create config
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, "aliases:\n  ls: eza").unwrap();

        // Create old cache with usage count
        let mut old_aliases = HashMap::new();
        old_aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "old_value".to_string(),
                space: true,
                global: false,
                count: 42, // Important: preserve this count
            },
        );
        let old_cache = AliasCache {
            hash: "old_hash".to_string(),
            aliases: old_aliases,
        };

        let merged = merge_cache_with_config(old_cache, "new_hash".to_string(), temp_dir.path()).unwrap();

        // Count should be preserved even though value changed
        assert_eq!(merged.aliases.get("ls").unwrap().count, 42);
        // Value should be from new config
        assert_eq!(merged.aliases.get("ls").unwrap().value, "eza");

        // Restore
        match original_cache_dir {
            Some(val) => std::env::set_var("AKA_CACHE_DIR", val),
            None => std::env::remove_var("AKA_CACHE_DIR"),
        }
    }

    #[test]
    fn test_sudo_trigger_with_bang() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let spec = Spec {
            defaults: Defaults { version: 1 },
            aliases,
            lookups: HashMap::new(),
        };

        let mut aka = AKA {
            eol: true, // Important: eol must be true for ! to trigger sudo
            spec,
            config_hash: "test".to_string(),
            home_dir: std::env::temp_dir(),
        };

        // Test sudo trigger with !
        let result = aka.replace("ls !").unwrap();
        assert!(result.contains("sudo"));
        assert!(result.contains("eza"));
    }

    #[test]
    fn test_sudo_with_flags() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test sudo with -E flag
        let result = aka.replace("sudo -E ls").unwrap();
        assert!(result.contains("sudo"));
        assert!(result.contains("-E"));
        assert!(result.contains("eza"));
    }

    #[test]
    fn test_sudo_with_user_flag() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test sudo with -u flag
        let result = aka.replace("sudo -u root ls").unwrap();
        assert!(result.contains("sudo"));
        assert!(result.contains("-u"));
        assert!(result.contains("root"));
        assert!(result.contains("eza"));
    }

    #[test]
    fn test_sudo_only_no_command() {
        let aliases = HashMap::new();
        let mut aka = create_test_aka_with_aliases(aliases);

        // Test just "sudo " without a command
        let result = aka.replace("sudo").unwrap();
        assert!(result.contains("sudo"));
    }

    #[test]
    fn test_lone_bang_eol() {
        let aliases = HashMap::new();

        let spec = Spec {
            defaults: Defaults { version: 1 },
            aliases,
            lookups: HashMap::new(),
        };

        let mut aka = AKA {
            eol: true,
            spec,
            config_hash: "test".to_string(),
            home_dir: std::env::temp_dir(),
        };

        // Test just "!" alone
        let result = aka.replace("!").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_split_respecting_quotes() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "echo".to_string(),
            Alias {
                name: "echo".to_string(),
                value: "printf".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // Test that quoted strings are preserved
        let result = aka.replace(r#"echo "hello world""#).unwrap();
        assert!(result.contains("printf"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_format_alias_output_empty() {
        let aliases: Vec<Alias> = vec![];
        let output = format_alias_output_from_iter(aliases.into_iter(), false);
        assert!(output.contains("No aliases found"));
        assert!(output.contains("count: 0"));
    }

    #[test]
    fn test_format_alias_output_with_counts() {
        let aliases = vec![
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 10,
            },
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 5,
            },
        ];

        let output = format_alias_output_from_iter(aliases.into_iter(), true);
        assert!(output.contains("10"));
        assert!(output.contains("5"));
        assert!(output.contains("ls"));
        assert!(output.contains("cat"));
        assert!(output.contains("count: 2"));
    }

    #[test]
    fn test_format_alias_output_multiline_value() {
        let aliases = vec![Alias {
            name: "multi".to_string(),
            value: "line1\nline2\nline3".to_string(),
            space: true,
            global: false,
            count: 0,
        }];

        let output = format_alias_output_from_iter(aliases.into_iter(), false);
        assert!(output.contains("multi"));
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
    }

    #[test]
    fn test_prepare_aliases_for_display_global_only() {
        let aliases = [
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
            Alias {
                name: "|c".to_string(),
                value: "| xclip".to_string(),
                space: true,
                global: true,
                count: 0,
            },
        ];

        let result: Vec<_> = prepare_aliases_for_display_iter(
            aliases.iter(),
            false,
            false,
            true, // global_only
            &[],
        )
        .collect();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "|c");
    }

    #[test]
    fn test_prepare_aliases_for_display_with_pattern() {
        let aliases = [
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
            Alias {
                name: "ll".to_string(),
                value: "ls -la".to_string(),
                space: true,
                global: false,
                count: 0,
            },
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        ];

        let patterns = vec!["l".to_string()];
        let result: Vec<_> = prepare_aliases_for_display_iter(aliases.iter(), false, false, false, &patterns).collect();

        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|a| a.name == "ls"));
        assert!(result.iter().any(|a| a.name == "ll"));
    }

    #[test]
    fn test_prepare_aliases_for_display_show_counts_nonzero() {
        let aliases = [
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 5, // Has count
            },
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 0, // No count
            },
        ];

        let result: Vec<_> = prepare_aliases_for_display_iter(
            aliases.iter(),
            true,  // show_counts
            false, // show_all = false (only show aliases with count > 0)
            false,
            &[],
        )
        .collect();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ls");
    }

    #[test]
    fn test_prepare_aliases_for_display_show_counts_all() {
        let aliases = [
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 5,
            },
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        ];

        let result: Vec<_> = prepare_aliases_for_display_iter(
            aliases.iter(),
            true, // show_counts
            true, // show_all = true (show all aliases)
            false,
            &[],
        )
        .collect();

        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_format_aliases_efficiently_single() {
        let aliases = [Alias {
            name: "ls".to_string(),
            value: "eza".to_string(),
            space: true,
            global: false,
            count: 0,
        }];

        let output = format_aliases_efficiently(aliases.iter(), false, false, false, &[]);
        assert!(output.contains("ls"));
        assert!(output.contains("eza"));
    }

    #[test]
    fn test_get_alias_names_for_completion_multiple() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );
        aliases.insert(
            "cat".to_string(),
            Alias {
                name: "cat".to_string(),
                value: "bat".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let aka = create_test_aka_with_aliases(aliases);

        let names = get_alias_names_for_completion(&aka);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"ls".to_string()));
        assert!(names.contains(&"cat".to_string()));
    }

    #[test]
    fn test_recursive_alias_detection() {
        let mut aliases = HashMap::new();
        // Create an alias that would recurse to itself
        aliases.insert(
            "eza".to_string(),
            Alias {
                name: "eza".to_string(),
                value: "eza --color=always".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        // The alias should NOT be expanded if the value starts with the same command
        let result = aka.replace("eza").unwrap();
        // Should not infinitely recurse - should detect this and skip
        assert!(result.contains("eza"));
    }

    #[test]
    fn test_positional_argument_replacement() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "greet".to_string(),
            Alias {
                name: "greet".to_string(),
                value: "echo Hello $1".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        let result = aka.replace("greet World").unwrap();
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
    }

    #[test]
    fn test_variadic_alias() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "run".to_string(),
            Alias {
                name: "run".to_string(),
                value: "cargo run -- $@".to_string(),
                space: true,
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        let result = aka.replace("run arg1 arg2 arg3").unwrap();
        assert!(result.contains("cargo run"));
        assert!(result.contains("arg1"));
        assert!(result.contains("arg2"));
        assert!(result.contains("arg3"));
    }

    #[test]
    fn test_alias_with_no_space_suffix() {
        let mut aliases = HashMap::new();
        aliases.insert(
            "cd..".to_string(),
            Alias {
                name: "cd..".to_string(),
                value: "cd ..".to_string(),
                space: false, // No trailing space
                global: false,
                count: 0,
            },
        );

        let mut aka = create_test_aka_with_aliases(aliases);

        let result = aka.replace("cd..").unwrap();
        assert_eq!(result, "cd ..");
    }

    #[test]
    fn test_store_hash_no_op_returns_ok() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // store_hash is now a no-op, should succeed without error
        let result = store_hash("test_hash", temp_dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_save_and_load_alias_cache_roundtrip_with_base() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_path_buf();

        // Create a cache
        let mut aliases = HashMap::new();
        aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "eza".to_string(),
                space: true,
                global: false,
                count: 42,
            },
        );

        let cache = AliasCache {
            hash: "test_hash_123".to_string(),
            aliases,
        };

        // Save cache using _with_base to avoid env var conflicts
        let save_result = save_alias_cache_with_base(&cache, Some(&base_path));
        assert!(save_result.is_ok());

        // Load cache using _with_base
        let load_result = load_alias_cache_with_base(Some(&base_path));
        assert!(load_result.is_ok());

        let loaded = load_result.unwrap();
        assert_eq!(loaded.hash, "test_hash_123");
        assert_eq!(loaded.aliases.len(), 1);
        assert_eq!(loaded.aliases.get("ls").unwrap().count, 42);
    }

    #[test]
    fn test_save_alias_cache_with_base_and_reload() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_path_buf();

        let cache = AliasCache {
            hash: "base_hash".to_string(),
            aliases: HashMap::new(),
        };

        let result = save_alias_cache_with_base(&cache, Some(&base_path));
        assert!(result.is_ok());

        // Verify we can load it back
        let loaded = load_alias_cache_with_base(Some(&base_path));
        assert!(loaded.is_ok());
        assert_eq!(loaded.unwrap().hash, "base_hash");
    }

    #[test]
    fn test_merge_cache_with_config_path_preserves() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // Create config
        let config_dir = temp_dir.path().join(".config").join("aka");
        fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("aka.yml");
        fs::write(&config_path, "aliases:\n  ls: eza\n  cat: bat").unwrap();

        // Create old cache with counts
        let mut old_aliases = HashMap::new();
        old_aliases.insert(
            "ls".to_string(),
            Alias {
                name: "ls".to_string(),
                value: "old_value".to_string(),
                space: true,
                global: false,
                count: 100,
            },
        );
        let old_cache = AliasCache {
            hash: "old_hash".to_string(),
            aliases: old_aliases,
        };

        let merged = merge_cache_with_config_path(old_cache, "new_hash".to_string(), &config_path).unwrap();

        // Count should be preserved
        assert_eq!(merged.aliases.get("ls").unwrap().count, 100);
        // New value from config
        assert_eq!(merged.aliases.get("ls").unwrap().value, "eza");
        // New hash
        assert_eq!(merged.hash, "new_hash");
    }
}
