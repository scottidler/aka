use clap::Parser;
use std::path::PathBuf;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Mutex};
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{BufRead, BufReader, Write};

use log::{info, error, debug, warn};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Event, EventKind};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use eyre::{Result, eyre};

// Import from the shared library
use aka_lib::{determine_socket_path, get_config_path_with_override, AKA, setup_logging, ProcessingMode, hash_config_file, store_hash, DaemonRequest, DaemonResponse};

#[derive(Parser)]
#[command(name = "aka-daemon", about = "AKA Alias Daemon")]
struct DaemonOpts {
    #[clap(long, help = "Run in foreground (don't daemonize)")]
    foreground: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,
}

// IPC Protocol Messages - now using shared types from aka_lib
type Request = DaemonRequest;
type Response = DaemonResponse;

struct DaemonServer {
    aka: Arc<RwLock<AKA>>,
    config_path: PathBuf,
    config_hash: Arc<RwLock<String>>,
    shutdown: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
    reload_receiver: Arc<Mutex<Receiver<()>>>,
}

impl DaemonServer {
    fn new(config: &Option<PathBuf>) -> Result<Self> {
        use std::time::Instant;

        let start_daemon_init = Instant::now();
        debug!("🚀 Daemon initializing, loading config...");

        // Determine config path using the same logic as direct mode
        let home_dir = dirs::home_dir()
            .ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let config_path = get_config_path_with_override(&home_dir, config)?;

        // Load initial config
        let aka = AKA::new(false, home_dir.clone())?;
        let aka = Arc::new(RwLock::new(aka));

        // Calculate initial config hash
        let initial_hash = hash_config_file(&config_path)?;
        let config_hash = Arc::new(RwLock::new(initial_hash.clone()));

        // Store hash for CLI comparison
        if let Err(e) = store_hash(&initial_hash, &home_dir) {
            warn!("Failed to store initial config hash: {}", e);
        }

        let shutdown = Arc::new(AtomicBool::new(false));

        // Set up file watcher
        let (reload_sender, reload_receiver) = channel();
        let reload_receiver = Arc::new(Mutex::new(reload_receiver));

        let config_path_for_watcher = config_path.clone();
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if let EventKind::Modify(_) = event.kind {
                        if event.paths.iter().any(|p| p == &config_path_for_watcher) {
                            debug!("📁 Config file change detected: {:?}", config_path_for_watcher);
                            if let Err(e) = reload_sender.send(()) {
                                error!("Failed to send reload signal: {}", e);
                            }
                        }
                    }
                }
                Err(e) => error!("File watcher error: {}", e),
            }
        }).map_err(|e| eyre!("Failed to create file watcher: {}", e))?;

        // Watch the config file
        watcher.watch(&config_path, RecursiveMode::NonRecursive).map_err(|e| eyre!("Failed to watch config file: {}", e))?;
        debug!("👀 File watcher set up for: {:?}", config_path);

        let daemon_init_duration = start_daemon_init.elapsed();
        debug!("✅ Daemon initialization complete: {:.3}ms", daemon_init_duration.as_secs_f64() * 1000.0);

        let alias_count = {
            let aka_guard = aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
            aka_guard.spec.aliases.len()
        };
        debug!("📦 Daemon has {} aliases cached in memory", alias_count);
        debug!("🔒 Initial config hash: {}", initial_hash);

        Ok(DaemonServer {
            aka,
            config_path,
            config_hash,
            shutdown,
            _watcher: Some(watcher),
            reload_receiver,
        })
    }

    fn reload_config(&self) -> Result<String> {
        use std::time::Instant;

        let start_reload = Instant::now();
        debug!("🔄 Manual config reload requested");

        // Calculate new hash
        let new_hash = hash_config_file(&self.config_path)?;
        let current_hash = {
            let hash_guard = self.config_hash.read().map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;
            hash_guard.clone()
        };

        if new_hash == current_hash {
            debug!("⚡ Config hash unchanged, skipping reload");
            return Ok("Config unchanged".to_string());
        }

        debug!("🔄 Config hash changed: {} -> {}", current_hash, new_hash);

        // Load new config (this will create the new cache or load existing one)
        let home_dir = dirs::home_dir()
            .ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let mut new_aka = AKA::new(false, home_dir.clone())?;

        // If we have an old cache, migrate counts to the new one
        if current_hash != new_hash {
            if let Err(e) = aka_lib::migrate_alias_counts(&current_hash, &new_hash, &mut new_aka.spec.aliases) {
                debug!("⚠️ Failed to migrate alias counts: {}", e);
            } else {
                // Save the migrated aliases back to cache
                if let Err(e) = aka_lib::save_alias_cache(&new_hash, &new_aka.spec.aliases, &home_dir) {
                    debug!("⚠️ Failed to save migrated alias cache: {}", e);
                }
            }
        }

        // Update stored config and hash atomically (hold both locks simultaneously)
        {
            let mut aka_guard = self.aka.write().map_err(|e| eyre!("Failed to acquire write lock on AKA: {}", e))?;
            let mut hash_guard = self.config_hash.write().map_err(|e| eyre!("Failed to acquire write lock on config hash: {}", e))?;

            *aka_guard = new_aka;
            *hash_guard = new_hash.clone();
        }

        // Store hash for CLI comparison
        if let Err(e) = store_hash(&new_hash, &home_dir) {
            warn!("Failed to store updated config hash: {}", e);
        }

        let reload_duration = start_reload.elapsed();
        let alias_count = {
            let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
            aka_guard.spec.aliases.len()
        };

        let message = format!("Config reloaded: {} aliases in {:.3}ms", alias_count, reload_duration.as_secs_f64() * 1000.0);
        debug!("✅ {}", message);

        Ok(message)
    }

    fn handle_client(&self, mut stream: UnixStream) -> Result<()> {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();

        // Read request line
        reader.read_line(&mut line)?;

        // Basic message size check
        if let Err(e) = aka_lib::protocol::validate_message_size(&line) {
            let error_response = Response::Error { message: format!("Message too large: {}", e) };
            let response_json = serde_json::to_string(&error_response)?;
            writeln!(stream, "{}", response_json)?;
            return Ok(());
        }

        let request: Request = serde_json::from_str(&line.trim())?;

        debug!("Received request: {:?}", request);

        let response = match request {
            Request::Query { cmdline, eol } => {
                let mut aka_guard = self.aka.write().map_err(|e| eyre!("Failed to acquire write lock on AKA: {}", e))?;
                // Update AKA's eol setting to match the request
                aka_guard.eol = eol;
                debug!("📤 Processing query: {}", cmdline);
                match aka_guard.replace_with_mode(&cmdline, ProcessingMode::Daemon) {
                    Ok(result) => {
                        debug!("✅ Query processed successfully");
                        Response::Success { data: result }
                    },
                    Err(e) => {
                        warn!("❌ Query processing failed: {}", e);
                        Response::Error { message: e.to_string() }
                    },
                }
            },
            Request::List { global, patterns } => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;

                debug!("📤 Processing list request (global: {}, patterns: {:?})", global, patterns);

                // Use iterator chains instead of collecting into Vec - much more memory efficient
                let output = aka_guard.spec.aliases
                    .iter()
                    .filter(|(_, alias)| !global || alias.global)
                    .filter(|(name, _)| patterns.is_empty() ||
                            patterns.iter().any(|pattern| name.starts_with(pattern)))
                    .map(|(name, alias)| format!("{}: {}", name, alias.value))
                    .collect::<Vec<_>>()
                    .join("\n");

                debug!("✅ List processed successfully");
                Response::Success { data: output }
            },
            Request::Freq { top } => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;

                debug!("📤 Processing frequency request (top: {:?})", top);

                // Collect aliases and sort by count (descending) then by name (ascending)
                let mut aliases: Vec<_> = aka_guard.spec.aliases.values().cloned().collect();
                aliases.sort_by(|a, b| {
                    match b.count.cmp(&a.count) {
                        std::cmp::Ordering::Equal => a.name.cmp(&b.name),
                        other => other,
                    }
                });

                // Apply top limit if specified
                if let Some(top_limit) = top {
                    aliases.truncate(top_limit);
                }

                // Format output
                let output = if aliases.is_empty() {
                    "No aliases found.".to_string()
                } else {
                    let max_count_len = aliases.iter().map(|a| a.count.to_string().len()).max().unwrap_or(0);

                    aliases.iter()
                        .map(|alias| {
                            let prefix = format!("{:>count_width$} {} -> ",
                                alias.count,
                                alias.name,
                                count_width = max_count_len
                            );
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
                        .join("\n")
                };

                debug!("✅ Frequency processed successfully for {} aliases", aliases.len());
                Response::Success { data: output }
            },
            Request::Health => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
                let hash_guard = self.config_hash.read().map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;

                debug!("📤 Processing health check");

                // Check if config is in sync
                let current_hash = match hash_config_file(&self.config_path) {
                    Ok(hash) => hash,
                    Err(e) => {
                        warn!("❌ Failed to calculate config hash: {}", e);
                        let error_response = Response::Error { message: format!("Failed to calculate config hash: {}", e) };
                        let response_json = serde_json::to_string(&error_response)?;
                        writeln!(stream, "{}", response_json)?;
                        return Ok(());
                    }
                };

                let status = if current_hash == *hash_guard {
                    format!("healthy:{}:synced", aka_guard.spec.aliases.len())
                } else {
                    format!("healthy:{}:stale", aka_guard.spec.aliases.len())
                };

                debug!("✅ Health check complete: {}", status);
                Response::Health { status }
            },
            Request::ReloadConfig => {
                debug!("📤 Processing config reload request");
                match self.reload_config() {
                    Ok(message) => {
                        debug!("✅ Config reload completed successfully");
                        Response::ConfigReloaded { success: true, message }
                    },
                    Err(e) => {
                        warn!("❌ Config reload failed: {}", e);
                        Response::ConfigReloaded { success: false, message: e.to_string() }
                    },
                }
            },
            Request::Shutdown => {
                debug!("Shutdown request received");
                self.shutdown.store(true, Ordering::Relaxed);
                Response::Success { data: "Shutting down".to_string() }
            },
        };

        // Send response
        let response_json = serde_json::to_string(&response)?;
        writeln!(stream, "{}", response_json)?;

        debug!("Sent response: {:?}", response);
        Ok(())
    }

    fn run(&self, socket_path: &PathBuf) -> Result<()> {
        // Remove existing socket file if it exists
        if socket_path.exists() {
            fs::remove_file(socket_path)?;
        }

        // Ensure socket directory exists
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create Unix socket listener
        let listener = UnixListener::bind(socket_path)?;
        debug!("📡 Socket listening at: {:?}", socket_path);

        // Start background file watching thread
        let reload_receiver = Arc::clone(&self.reload_receiver);
        let aka_for_watcher = Arc::clone(&self.aka);
        let config_path_for_watcher = self.config_path.clone();
        let config_hash_for_watcher = Arc::clone(&self.config_hash);
        let shutdown_for_watcher = Arc::clone(&self.shutdown);

        thread::spawn(move || {
            let receiver = reload_receiver.lock().map_err(|e| {
                error!("Failed to acquire lock on reload receiver: {}", e);
            });

            if let Ok(receiver) = receiver {
                while !shutdown_for_watcher.load(Ordering::Relaxed) {
                    if let Ok(()) = receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        debug!("📁 File change detected, reloading config automatically");

                        // Calculate new hash
                        match hash_config_file(&config_path_for_watcher) {
                            Ok(new_hash) => {
                                let current_hash = {
                                    match config_hash_for_watcher.read() {
                                        Ok(guard) => guard.clone(),
                                        Err(e) => {
                                            error!("Failed to acquire read lock on config hash: {}", e);
                                            continue;
                                        }
                                    }
                                };

                                if new_hash != current_hash {
                                    debug!("🔄 Auto-reload: hash changed {} -> {}", current_hash, new_hash);

                                    // Load new config
                                    let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                                    match AKA::new(false, home_dir.clone()) {
                                        Ok(mut new_aka) => {
                                            // Migrate usage counts from old cache to new cache
                                            if let Err(e) = aka_lib::migrate_alias_counts(&current_hash, &new_hash, &mut new_aka.spec.aliases) {
                                                debug!("⚠️ Failed to migrate alias counts: {}", e);
                                            } else {
                                                // Save the migrated aliases back to cache
                                                let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                                                if let Err(e) = aka_lib::save_alias_cache(&new_hash, &new_aka.spec.aliases, &home_dir) {
                                                    debug!("⚠️ Failed to save migrated alias cache: {}", e);
                                                }
                                            }

                                            // Update stored config and hash atomically (hold both locks simultaneously)
                                            {
                                                match (aka_for_watcher.write(), config_hash_for_watcher.write()) {
                                                    (Ok(mut aka_guard), Ok(mut hash_guard)) => {
                                                        *aka_guard = new_aka;
                                                        *hash_guard = new_hash.clone();
                                                    }
                                                    (Err(e), _) => {
                                                        error!("Failed to acquire write lock on AKA: {}", e);
                                                        continue;
                                                    }
                                                    (_, Err(e)) => {
                                                        error!("Failed to acquire write lock on config hash: {}", e);
                                                        continue;
                                                    }
                                                }
                                            }

                                            // Store hash for CLI comparison
                                            let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                                            if let Err(e) = store_hash(&new_hash, &home_dir) {
                                                warn!("Failed to store updated config hash: {}", e);
                                            }

                                            let alias_count = {
                                                match aka_for_watcher.read() {
                                                    Ok(aka_guard) => aka_guard.spec.aliases.len(),
                                                    Err(e) => {
                                                        error!("Failed to acquire read lock on AKA: {}", e);
                                                        0
                                                    }
                                                }
                                            };

                                            debug!("🔄 Config auto-reloaded: {} aliases", alias_count);
                                        },
                                        Err(e) => {
                                            error!("Failed to auto-reload config: {}", e);
                                        }
                                    }
                                } else {
                                    debug!("⚡ Auto-reload: hash unchanged, skipping");
                                }
                            },
                            Err(e) => {
                                error!("Failed to calculate config hash for auto-reload: {}", e);
                            }
                        }
                    }
                }
            }
            debug!("🛑 File watcher thread shutting down");
        });

        // Main server loop
        for stream in listener.incoming() {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }

            match stream {
                Ok(stream) => {
                    if let Err(e) = self.handle_client(stream) {
                        error!("Error handling client: {}", e);
                    }
                },
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_daemon_opts_parsing() {
        // Test that daemon options can be created
        let opts = DaemonOpts {
            foreground: true,
            config: Some(PathBuf::from("/tmp/test.yml")),
        };
        assert!(opts.foreground);
        assert!(opts.config.is_some());
    }

    #[test]
    fn test_request_serialization() {
        // Test that IPC requests can be serialized
        let health_request = Request::Health;
        let serialized = serde_json::to_string(&health_request);
        assert!(serialized.is_ok());

        let query_request = Request::Query {
            cmdline: "test command".to_string(),
            eol: false,
        };
        let serialized = serde_json::to_string(&query_request);
        assert!(serialized.is_ok());

        let list_request = Request::List { global: true, patterns: vec!["test".to_string()] };
        let serialized = serde_json::to_string(&list_request);
        assert!(serialized.is_ok());
    }

    #[test]
    fn test_response_serialization() {
        // Test that IPC responses can be serialized
        let success_response = Response::Success { data: "test data".to_string() };
        let serialized = serde_json::to_string(&success_response);
        assert!(serialized.is_ok());

        let error_response = Response::Error { message: "test error".to_string() };
        let serialized = serde_json::to_string(&error_response);
        assert!(serialized.is_ok());

        let health_response = Response::Health { status: "healthy:5:aliases".to_string() };
        let serialized = serde_json::to_string(&health_response);
        assert!(serialized.is_ok());
    }

    #[test]
    fn test_request_response_roundtrip() {
        // Test that requests and responses can be serialized and deserialized
        let original_request = Request::Query {
            cmdline: "test command".to_string(),
            eol: true,
        };
        let serialized = serde_json::to_string(&original_request).map_err(|e| eyre!("Failed to serialize request: {}", e)).expect("Serialization should succeed");
        let deserialized: Request = serde_json::from_str(&serialized).map_err(|e| eyre!("Failed to deserialize request: {}", e)).expect("Deserialization should succeed");

        match (original_request, deserialized) {
            (Request::Query { cmdline: orig, eol: orig_eol }, Request::Query { cmdline: deser, eol: deser_eol }) => {
                assert_eq!(orig, deser);
                assert_eq!(orig_eol, deser_eol);
            },
            _ => panic!("Request roundtrip failed"),
        }
    }
}

fn main() {
    let opts = DaemonOpts::parse();

    // Set up logging
    let home_dir = dirs::home_dir()
        .ok_or_else(|| eyre!("Unable to determine home directory"))
        .unwrap_or_else(|e| {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        });
    if let Err(e) = setup_logging(&home_dir) {
        eprintln!("Warning: Failed to set up logging: {}", e);
    }

            info!("🚀 AKA Daemon starting...");

    // Determine socket path
    let socket_path = match determine_socket_path(&home_dir) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to determine socket path: {}", e);
            std::process::exit(1);
        }
    };

    // Create daemon server
    let server = match DaemonServer::new(&opts.config) {
        Ok(server) => server,
        Err(e) => {
            error!("Failed to create daemon server: {}", e);
            std::process::exit(1);
        }
    };

    // Set up signal handling
    let shutdown_clone = server.shutdown.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        debug!("🛑 Shutdown signal received");
        shutdown_clone.store(true, Ordering::Relaxed);
    }) {
        error!("Error setting signal handler: {}", e);
        std::process::exit(1);
    }

            info!("✅ Daemon running (PID: {})", std::process::id());

    // Run the server
    if let Err(e) = server.run(&socket_path) {
        error!("Server error: {}", e);
        std::process::exit(1);
    }

            info!("👋 Daemon stopped");
}
