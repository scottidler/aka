use clap::Parser;
use std::path::PathBuf;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Mutex};
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{BufRead, BufReader, Write};
use serde::{Deserialize, Serialize};
use log::{info, error, debug, warn};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Event, EventKind};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use eyre::{Result, eyre};

// Import from the shared library
use aka_lib::{determine_socket_path, AKA, setup_logging, ProcessingMode, hash_config_file, store_hash};

#[derive(Parser)]
#[command(name = "aka-daemon", about = "AKA Alias Daemon")]
struct DaemonOpts {
    #[clap(long, help = "Run in foreground (don't daemonize)")]
    foreground: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,
}

// IPC Protocol Messages
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum Request {
    Query { cmdline: String },
    List { global: bool, patterns: Vec<String> },
    Health,
    ReloadConfig,
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum Response {
    Success { data: String },
    Error { message: String },
    Health { status: String },
    ConfigReloaded { success: bool, message: String },
}

struct DaemonServer {
    aka: Arc<RwLock<AKA>>,
    config_path: PathBuf,
    config_hash: Arc<RwLock<String>>,
    shutdown: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
    reload_receiver: Arc<Mutex<Receiver<()>>>,
}

impl DaemonServer {
    fn new(config: &Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        use std::time::Instant;

        let start_daemon_init = Instant::now();
        debug!("🚀 Daemon initializing, loading config...");

        // Determine config path
        let config_path = match config {
            Some(path) => path.clone(),
            None => aka_lib::get_config_path()?,
        };

        // Load initial config
        let aka = AKA::new(false, &Some(config_path.clone()))?;
        let aka = Arc::new(RwLock::new(aka));
        
        // Calculate initial config hash
        let initial_hash = hash_config_file(&config_path)?;
        let config_hash = Arc::new(RwLock::new(initial_hash.clone()));
        
        // Store hash for CLI comparison
        if let Err(e) = store_hash(&initial_hash) {
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
        })?;

        // Watch the config file
        watcher.watch(&config_path, RecursiveMode::NonRecursive)?;
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

    fn reload_config(&self) -> Result<String, Box<dyn std::error::Error>> {
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
        
        // Load new config
        let new_aka = AKA::new(false, &Some(self.config_path.clone()))?;
        
        // Update stored config
        {
            let mut aka_guard = self.aka.write().map_err(|e| eyre!("Failed to acquire write lock on AKA: {}", e))?;
            *aka_guard = new_aka;
        }
        
        // Update hash
        {
            let mut hash_guard = self.config_hash.write().map_err(|e| eyre!("Failed to acquire write lock on config hash: {}", e))?;
            *hash_guard = new_hash.clone();
        }
        
        // Store hash for CLI comparison
        if let Err(e) = store_hash(&new_hash) {
            warn!("Failed to store updated config hash: {}", e);
        }
        
        let reload_duration = start_reload.elapsed();
        let alias_count = {
            let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
            aka_guard.spec.aliases.len()
        };
        
        let message = format!("Config reloaded: {} aliases in {:.3}ms", alias_count, reload_duration.as_secs_f64() * 1000.0);
        info!("✅ {}", message);
        
        Ok(message)
    }
    
    fn handle_client(&self, mut stream: UnixStream) -> Result<(), Box<dyn std::error::Error>> {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();

        // Read request line
        reader.read_line(&mut line)?;
        let request: Request = serde_json::from_str(&line.trim())?;

        debug!("Received request: {:?}", request);

        let response = match request {
            Request::Query { cmdline } => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
                match aka_guard.replace_with_mode(&cmdline, ProcessingMode::Daemon) {
                    Ok(result) => Response::Success { data: result },
                    Err(e) => Response::Error { message: e.to_string() },
                }
            },
            Request::List { global, patterns } => {
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
                let mut aliases: Vec<_> = aka_guard.spec.aliases.values().cloned().collect();
                aliases.sort_by_key(|a| a.name.clone());

                if global {
                    aliases = aliases.into_iter().filter(|alias| alias.global).collect();
                }

                let filtered_aliases: Vec<_> = if patterns.is_empty() {
                    aliases
                } else {
                    aliases.into_iter()
                        .filter(|alias| patterns.iter().any(|pattern| alias.name.starts_with(pattern)))
                        .collect()
                };

                let output = filtered_aliases.iter()
                    .map(|alias| format!("{}: {}", alias.name, alias.value))
                    .collect::<Vec<_>>()
                    .join("\n");

                Response::Success { data: output }
            },
            Request::Health => {
                // Enhanced health check - confirm config is loaded and valid
                let aka_guard = self.aka.read().map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
                let alias_count = aka_guard.spec.aliases.len();
                let current_hash = {
                    let hash_guard = self.config_hash.read().map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;
                    hash_guard.clone()
                };
                
                // Check if config file has changed
                match hash_config_file(&self.config_path) {
                    Ok(file_hash) => {
                        let sync_status = if file_hash == current_hash { "synced" } else { "stale" };
                        let status = format!("healthy:{}:aliases:{}:{}", alias_count, current_hash[..8].to_string(), sync_status);
                        Response::Health { status }
                    },
                    Err(_) => {
                        let status = format!("healthy:{}:aliases:{}:unknown", alias_count, current_hash[..8].to_string());
                        Response::Health { status }
                    }
                }
            },
            Request::ReloadConfig => {
                info!("Config reload request received");
                match self.reload_config() {
                    Ok(message) => Response::ConfigReloaded { success: true, message },
                    Err(e) => Response::ConfigReloaded { success: false, message: e.to_string() },
                }
            },
            Request::Shutdown => {
                info!("Shutdown request received");
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

    fn run(&self, socket_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
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
        info!("📡 Socket listening at: {:?}", socket_path);

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
                                    match AKA::new(false, &Some(config_path_for_watcher.clone())) {
                                        Ok(new_aka) => {
                                            // Update stored config
                                            {
                                                match aka_for_watcher.write() {
                                                    Ok(mut aka_guard) => {
                                                        *aka_guard = new_aka;
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to acquire write lock on AKA: {}", e);
                                                        continue;
                                                    }
                                                }
                                            }
                                            
                                            // Update hash
                                            {
                                                match config_hash_for_watcher.write() {
                                                    Ok(mut hash_guard) => {
                                                        *hash_guard = new_hash.clone();
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to acquire write lock on config hash: {}", e);
                                                        continue;
                                                    }
                                                }
                                            }
                                            
                                            // Store hash for CLI comparison
                                            if let Err(e) = store_hash(&new_hash) {
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
                                            
                                            info!("🔄 Config auto-reloaded: {} aliases", alias_count);
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
    use tempfile::TempDir;
    use std::fs;

    const TEST_CONFIG: &str = r#"
lookups: {}

aliases:
  test-daemon:
    value: echo "daemon test"
    global: true
"#;

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

        let query_request = Request::Query { cmdline: "test command".to_string() };
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
    fn test_daemon_server_creation() {
        // Test daemon server creation with test config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("test.yml");
        fs::write(&config_file, TEST_CONFIG).expect("Failed to write test config");

        let result = DaemonServer::new(&Some(config_file));
        assert!(result.is_ok(), "Should create daemon server with valid config");

        if let Ok(server) = result {
            assert!(!server.shutdown.load(std::sync::atomic::Ordering::Relaxed));
        }
    }

    #[test]
    fn test_daemon_server_invalid_config() {
        // Test daemon server creation with invalid config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("invalid.yml");
        fs::write(&config_file, "invalid: yaml: [").expect("Failed to write invalid config");

        let result = DaemonServer::new(&Some(config_file));
        assert!(result.is_err(), "Should fail with invalid config");
    }

    #[test]
    fn test_request_response_roundtrip() {
        // Test that requests and responses can be serialized and deserialized
        let original_request = Request::Query { cmdline: "test command".to_string() };
        let serialized = serde_json::to_string(&original_request).map_err(|e| eyre!("Failed to serialize request: {}", e)).expect("Serialization should succeed");
        let deserialized: Request = serde_json::from_str(&serialized).map_err(|e| eyre!("Failed to deserialize request: {}", e)).expect("Deserialization should succeed");

        match (original_request, deserialized) {
            (Request::Query { cmdline: orig }, Request::Query { cmdline: deser }) => {
                assert_eq!(orig, deser);
            },
            _ => panic!("Request roundtrip failed"),
        }
    }
}

fn main() {
    let opts = DaemonOpts::parse();

    // Set up logging
    if let Err(e) = setup_logging() {
        eprintln!("Warning: Failed to set up logging: {}", e);
    }

    info!("🚀 AKA Daemon starting...");

    // Determine socket path
    let socket_path = match determine_socket_path() {
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
        info!("🛑 Shutdown signal received");
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

    // Cleanup
    if socket_path.exists() {
        if let Err(e) = fs::remove_file(&socket_path) {
            error!("Failed to remove socket file: {}", e);
        } else {
            info!("🧹 Socket file cleaned up");
        }
    }

    info!("👋 Daemon stopped");
}
