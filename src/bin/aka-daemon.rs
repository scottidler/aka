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

        // Load new config using sync function
        let home_dir = dirs::home_dir()
            .ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let new_aka = AKA::new(false, home_dir.clone())?;

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

    fn process_health_request(&self) -> Result<Response> {
        let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
        let hash_guard = self.config_hash.read().map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;

        debug!("📤 Processing health check");

        // Check if config is in sync
        let current_hash = match hash_config_file(&self.config_path) {
            Ok(hash) => hash,
            Err(e) => {
                warn!("❌ Failed to calculate config hash: {}", e);
                return Ok(Response::Error { message: format!("Failed to calculate config hash: {}", e) });
            }
        };

        let status = if current_hash == *hash_guard {
            format!("healthy:{}:synced", aka_guard.spec.aliases.len())
        } else {
            format!("healthy:{}:stale", aka_guard.spec.aliases.len())
        };

        debug!("✅ Health check complete: {}", status);
        Ok(Response::Health { status })
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

                let output = aka_lib::format_aliases_efficiently(
                    aka_guard.spec.aliases.values(),
                    false, // show_counts
                    true,  // show_all
                    global,
                    &patterns,
                );

                debug!("✅ List processed successfully");
                Response::Success { data: output }
            },
            Request::Freq { all } => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
                debug!("📤 Processing frequency request (all: {})", all);

                let output = aka_lib::format_aliases_efficiently(
                    aka_guard.spec.aliases.values(),
                    true, // show_counts
                    all,
                    false, // global_only
                    &[], // patterns
                );

                debug!("✅ Frequency processed successfully");
                Response::Success { data: output }
            },
            Request::Health => {
                self.process_health_request()?
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
                debug!("📤 Processing shutdown request");
                self.shutdown.store(true, Ordering::Relaxed);
                Response::ShutdownAck
            },
            Request::CompleteAliases => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
                debug!("📤 Processing complete aliases request");

                let alias_names = aka_lib::get_alias_names_for_completion(&aka_guard);
                let output = alias_names.join("\n");

                debug!("✅ Complete aliases processed successfully");
                Response::Success { data: output }
            },
        };

        let response_json = serde_json::to_string(&response)?;
        writeln!(stream, "{}", response_json)?;

        Ok(())
    }

    fn handle_config_file_change(
        new_hash: String,
        current_hash: String,
        aka_for_watcher: &Arc<RwLock<AKA>>,
        config_hash_for_watcher: &Arc<RwLock<String>>,
        home_dir: PathBuf,
    ) -> Result<()> {
        debug!("🔄 Auto-reload: hash changed {} -> {}", current_hash, new_hash);

        // Load new config using sync function
        match AKA::new(false, home_dir.clone()) {
            Ok(new_aka) => {
                // Update stored config and hash atomically (hold both locks simultaneously)
                {
                    match (aka_for_watcher.write(), config_hash_for_watcher.write()) {
                        (Ok(mut aka_guard), Ok(mut hash_guard)) => {
                            *aka_guard = new_aka;
                            *hash_guard = new_hash.clone();
                        }
                        (Err(e), _) => {
                            error!("Failed to acquire write lock on AKA: {}", e);
                            return Err(eyre!("Failed to acquire write lock on AKA: {}", e));
                        }
                        (_, Err(e)) => {
                            error!("Failed to acquire write lock on config hash: {}", e);
                            return Err(eyre!("Failed to acquire write lock on config hash: {}", e));
                        }
                    }
                }

                // Store hash for CLI comparison
                if let Err(e) = store_hash(&new_hash, &home_dir) {
                    warn!("Failed to store updated config hash: {}", e);
                }

                debug!("✅ Auto-reload completed successfully");
                Ok(())
            }
            Err(e) => {
                error!("Failed to reload config: {}", e);
                Err(eyre!("Failed to reload config: {}", e))
            }
        }
    }

    fn handle_file_watcher_loop(
        receiver: &Receiver<()>,
        shutdown_for_watcher: Arc<AtomicBool>,
        config_path_for_watcher: PathBuf,
        aka_for_watcher: Arc<RwLock<AKA>>,
        config_hash_for_watcher: Arc<RwLock<String>>,
    ) -> Result<()> {
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
                            let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                            if let Err(e) = Self::handle_config_file_change(new_hash, current_hash, &aka_for_watcher, &config_hash_for_watcher, home_dir) {
                                error!("Failed to handle config file change: {}", e);
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
        debug!("🛑 File watcher thread shutting down");
        Ok(())
    }

    fn handle_incoming_connections(
        &self,
        listener: UnixListener,
    ) -> Result<()> {
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
                if let Err(e) = Self::handle_file_watcher_loop(&receiver, shutdown_for_watcher, config_path_for_watcher, aka_for_watcher, config_hash_for_watcher) {
                    error!("Failed to run file watcher loop: {}", e);
                }
            }
        });

        let result = self.handle_incoming_connections(listener);

        // Clean up socket file on shutdown
        if socket_path.exists() {
            debug!("🧹 Cleaning up socket file on shutdown");
            if let Err(e) = fs::remove_file(socket_path) {
                error!("Failed to remove socket file on shutdown: {}", e);
            } else {
                debug!("✅ Socket file removed successfully");
            }
        }

        result
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

fn initialize_daemon_server(
    opts: &DaemonOpts,
    home_dir: &PathBuf,
) -> Result<(DaemonServer, PathBuf)> {
    // Determine socket path
    let socket_path = match determine_socket_path(home_dir) {
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

    Ok((server, socket_path))
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

    // Initialize daemon server
    let (server, socket_path) = match initialize_daemon_server(&opts, &home_dir) {
        Ok((server, socket_path)) => (server, socket_path),
        Err(e) => {
            error!("Failed to initialize daemon server: {}", e);
            std::process::exit(1);
        }
    };

    // Set up signal handling
    let shutdown_clone = server.shutdown.clone();
    let socket_path_clone = socket_path.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        debug!("🛑 Shutdown signal received");
        shutdown_clone.store(true, Ordering::Relaxed);

        // Clean up socket file on signal
        if socket_path_clone.exists() {
            debug!("🧹 Cleaning up socket file on signal");
            if let Err(e) = std::fs::remove_file(&socket_path_clone) {
                error!("Failed to remove socket file on signal: {}", e);
            } else {
                debug!("✅ Socket file removed successfully on signal");
            }
        }
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
