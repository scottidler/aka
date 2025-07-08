use clap::Parser;
use std::path::PathBuf;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Mutex};
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{BufRead, BufReader, Write};
use std::time::Instant;
use std::sync::mpsc::{channel, Receiver, Sender};

use log::{info, error, debug, warn};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Event, EventKind};
use std::thread;
use eyre::{Result, eyre};

// Import from the shared library
use aka_lib::{determine_socket_path, AKA, setup_logging, ProcessingMode, hash_config_file, store_hash, DaemonRequest, DaemonResponse};

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

// Debouncing configuration
const DEBOUNCE_DELAY_MS: u64 = 100;  // Wait 100ms for additional changes
const MAX_DEBOUNCE_WAIT_MS: u64 = 1000;  // Maximum wait time before forcing reload

// State management for atomic updates
struct DaemonState {
    aka: AKA,
    config_hash: String,
}

impl DaemonState {
    fn new(aka: AKA, config_hash: String) -> Self {
        Self { aka, config_hash }
    }
}

struct DaemonServer {
    // Atomic state management - single RwLock for consistency
    state: Arc<RwLock<DaemonState>>,
    config_path: PathBuf,
    
    // Reload synchronization
    reload_mutex: Arc<Mutex<()>>,  // Ensures only one reload at a time
    reload_sender: Sender<()>,
    reload_receiver: Arc<Mutex<Receiver<()>>>,
    
    // Debouncing state
    last_reload_time: Arc<Mutex<Instant>>,
    pending_reload: Arc<AtomicBool>,
    
    shutdown: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
}

impl DaemonServer {
    fn new(config: &Option<PathBuf>) -> Result<Self> {
        use std::time::Instant;

        let start_daemon_init = Instant::now();
        debug!("🚀 Daemon initializing, loading config...");

        // Determine config path
        let home_dir = dirs::home_dir()
            .ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let config_path = match config {
            Some(path) => path.clone(),
            None => aka_lib::get_config_path(&home_dir)?,
        };

        // Load initial config
        let aka = AKA::new(false, home_dir.clone())?;
        let initial_hash = hash_config_file(&config_path)?;
        
        // Create atomic state
        let initial_state = DaemonState::new(aka, initial_hash.clone());
        let state = Arc::new(RwLock::new(initial_state));

        // Store hash for CLI comparison
        if let Err(e) = store_hash(&initial_hash, &home_dir) {
            warn!("Failed to store initial config hash: {}", e);
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let reload_mutex = Arc::new(Mutex::new(()));
        let last_reload_time = Arc::new(Mutex::new(Instant::now()));
        let pending_reload = Arc::new(AtomicBool::new(false));

        // Set up file watcher with debouncing
        let (reload_sender, reload_receiver) = channel();
        let reload_receiver = Arc::new(Mutex::new(reload_receiver));

        let config_path_for_watcher = config_path.clone();
        let reload_sender_clone = reload_sender.clone();
        let pending_reload_clone = Arc::clone(&pending_reload);
        
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if let EventKind::Modify(_) = event.kind {
                        if event.paths.iter().any(|p| p == &config_path_for_watcher) {
                            debug!("📁 Config file change detected: {:?}", config_path_for_watcher);
                            
                            // Set pending reload flag for debouncing
                            pending_reload_clone.store(true, Ordering::Relaxed);
                            
                            // Send reload signal (will be debounced)
                            if let Err(e) = reload_sender_clone.send(()) {
                                error!("Failed to send reload signal: {}", e);
                            }
                        }
                    }
                }
                Err(e) => error!("File watcher error: {}", e),
            }
        }).map_err(|e| eyre!("Failed to create file watcher: {}", e))?;

        // Watch the config file
        watcher.watch(&config_path, RecursiveMode::NonRecursive)
            .map_err(|e| eyre!("Failed to watch config file: {}", e))?;
        debug!("👀 File watcher set up for: {:?}", config_path);

        let daemon_init_duration = start_daemon_init.elapsed();
        debug!("✅ Daemon initialization complete: {:.3}ms", daemon_init_duration.as_secs_f64() * 1000.0);

        let alias_count = {
            let state_guard = state.read().map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
            state_guard.aka.spec.aliases.len()
        };
        debug!("📦 Daemon has {} aliases cached in memory", alias_count);
        debug!("🔒 Initial config hash: {}", initial_hash);

        Ok(DaemonServer {
            state,
            config_path,
            reload_mutex,
            reload_sender,
            reload_receiver,
            last_reload_time,
            pending_reload,
            shutdown,
            _watcher: Some(watcher),
        })
    }

    fn reload_config(&self) -> Result<String> {
        // Acquire reload mutex to ensure only one reload at a time
        let _reload_guard = self.reload_mutex.lock()
            .map_err(|e| eyre!("Failed to acquire reload mutex: {}", e))?;

        use std::time::Instant;
        let start_reload = Instant::now();
        debug!("🔄 Config reload requested");

        // Calculate new hash
        let new_hash = hash_config_file(&self.config_path)?;
        
        // Check current state atomically
        let needs_reload = {
            let state_guard = self.state.read()
                .map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
            state_guard.config_hash != new_hash
        };

        if !needs_reload {
            debug!("⚡ Config hash unchanged, skipping reload");
            return Ok("Config unchanged".to_string());
        }

        debug!("🔄 Config hash changed, reloading...");

        // Load new config
        let home_dir = dirs::home_dir()
            .ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let mut new_aka = AKA::new(false, home_dir.clone())?;

        // Migrate usage counts from old cache to new cache
        let old_hash = {
            let state_guard = self.state.read()
                .map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
            state_guard.config_hash.clone()
        };

        if old_hash != new_hash {
            if let Err(e) = aka_lib::migrate_alias_counts(&old_hash, &new_hash, &mut new_aka.spec.aliases) {
                debug!("⚠️ Failed to migrate alias counts: {}", e);
            } else {
                // Save the migrated aliases back to cache
                if let Err(e) = aka_lib::save_alias_cache(&new_hash, &new_aka.spec.aliases, &home_dir) {
                    debug!("⚠️ Failed to save migrated alias cache: {}", e);
                }
            }
        }

        // ATOMIC STATE UPDATE: Update both config and hash together
        {
            let mut state_guard = self.state.write()
                .map_err(|e| eyre!("Failed to acquire write lock on state: {}", e))?;
            
            // Update both aka and hash atomically
            state_guard.aka = new_aka;
            state_guard.config_hash = new_hash.clone();
        }

        // Store hash for CLI comparison
        if let Err(e) = store_hash(&new_hash, &home_dir) {
            warn!("Failed to store updated config hash: {}", e);
        }

        // Update last reload time
        {
            let mut last_reload = self.last_reload_time.lock()
                .map_err(|e| eyre!("Failed to acquire lock on last reload time: {}", e))?;
            *last_reload = Instant::now();
        }

        let reload_duration = start_reload.elapsed();
        let alias_count = {
            let state_guard = self.state.read()
                .map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
            state_guard.aka.spec.aliases.len()
        };
        debug!("✅ Config reloaded successfully: {:.3}ms, {} aliases", reload_duration.as_secs_f64() * 1000.0, alias_count);
        debug!("🔒 New config hash: {}", new_hash);

        Ok(format!("Config reloaded: {} aliases", alias_count))
    }

    fn handle_client(&self, mut stream: UnixStream) -> Result<()> {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        
        match reader.read_line(&mut line) {
            Ok(0) => {
                debug!("Client disconnected");
                return Ok(());
            }
            Ok(_) => {
                let request: Request = serde_json::from_str(line.trim())
                    .map_err(|e| eyre!("Failed to parse request: {}", e))?;
                
                debug!("📨 Received request: {:?}", request);
                
                let response = match request {
                    Request::Query { pattern, eol } => {
                        // Atomic read of current state
                        let state_guard = self.state.read()
                            .map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
                        
                        match state_guard.aka.spec.aliases.get(&pattern) {
                            Some(alias) => {
                                debug!("✅ Found alias: {} -> {}", pattern, alias.command);
                                
                                // Update usage count atomically
                                drop(state_guard);  // Release read lock
                                
                                {
                                    let mut state_guard = self.state.write()
                                        .map_err(|e| eyre!("Failed to acquire write lock on state: {}", e))?;
                                    
                                    if let Some(alias) = state_guard.aka.spec.aliases.get_mut(&pattern) {
                                        alias.count += 1;
                                        alias.last_used = Some(chrono::Utc::now());
                                    }
                                    
                                    // Save updated cache
                                    let home_dir = dirs::home_dir()
                                        .ok_or_else(|| eyre!("Unable to determine home directory"))?;
                                    if let Err(e) = aka_lib::save_alias_cache(&state_guard.config_hash, &state_guard.aka.spec.aliases, &home_dir) {
                                        debug!("⚠️ Failed to save alias cache: {}", e);
                                    }
                                }
                                
                                // Re-acquire read lock to get updated alias
                                let state_guard = self.state.read()
                                    .map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
                                
                                if let Some(alias) = state_guard.aka.spec.aliases.get(&pattern) {
                                    Response::Found { 
                                        command: alias.command.clone(), 
                                        eol: eol.unwrap_or_default() 
                                    }
                                } else {
                                    Response::NotFound
                                }
                            }
                            None => {
                                debug!("❌ Alias not found: {}", pattern);
                                Response::NotFound
                            }
                        }
                    }
                    Request::Reload => {
                        match self.reload_config() {
                            Ok(message) => {
                                debug!("✅ Manual reload successful: {}", message);
                                Response::Success { message }
                            }
                            Err(e) => {
                                error!("❌ Manual reload failed: {}", e);
                                Response::Error { message: e.to_string() }
                            }
                        }
                    }
                    Request::Shutdown => {
                        debug!("🛑 Shutdown requested");
                        self.shutdown.store(true, Ordering::Relaxed);
                        Response::Success { message: "Daemon shutting down".to_string() }
                    }
                    Request::Status => {
                        let state_guard = self.state.read()
                            .map_err(|e| eyre!("Failed to acquire read lock on state: {}", e))?;
                        
                        let message = format!("Daemon running: {} aliases loaded", state_guard.aka.spec.aliases.len());
                        debug!("📊 Status check: {}", message);
                        Response::Success { message }
                    }
                };

                let response_json = serde_json::to_string(&response)
                    .map_err(|e| eyre!("Failed to serialize response: {}", e))?;
                
                stream.write_all(response_json.as_bytes())
                    .map_err(|e| eyre!("Failed to write response: {}", e))?;
                stream.write_all(b"\n")
                    .map_err(|e| eyre!("Failed to write newline: {}", e))?;
                
                debug!("📤 Sent response: {:?}", response);
            }
            Err(e) => {
                error!("Failed to read from client: {}", e);
                return Err(eyre!("Failed to read from client: {}", e));
            }
        }
        
        Ok(())
    }

    fn run(&self, socket_path: &PathBuf) -> Result<()> {
        // Start the debounced reload thread
        self.start_debounced_reload_thread();

        let listener = UnixListener::bind(socket_path)
            .map_err(|e| eyre!("Failed to bind to socket: {}", e))?;
        
        info!("🚀 Daemon listening on: {:?}", socket_path);

        for stream in listener.incoming() {
            if self.shutdown.load(Ordering::Relaxed) {
                debug!("🛑 Shutdown signal received, stopping daemon");
                break;
            }

            match stream {
                Ok(stream) => {
                    if let Err(e) = self.handle_client(stream) {
                        error!("Error handling client: {}", e);
                    }
                }
                Err(e) => {
                    error!("Error accepting connection: {}", e);
                }
            }
        }

        Ok(())
    }

    fn start_debounced_reload_thread(&self) {
        let state = Arc::clone(&self.state);
        let config_path = self.config_path.clone();
        let reload_mutex = Arc::clone(&self.reload_mutex);
        let reload_receiver = Arc::clone(&self.reload_receiver);
        let last_reload_time = Arc::clone(&self.last_reload_time);
        let pending_reload = Arc::clone(&self.pending_reload);
        let shutdown = Arc::clone(&self.shutdown);

        thread::spawn(move || {
            debug!("🧵 Debounced reload thread started");
            
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    debug!("🛑 Shutdown signal received, stopping debounced reload thread");
                    break;
                }

                // Wait for reload signals
                let receiver = reload_receiver.lock().expect("Failed to lock reload receiver");
                match receiver.recv() {
                    Ok(()) => {
                        drop(receiver);  // Release lock immediately
                        
                        // Perform debounced reload
                        Self::perform_debounced_reload(
                            &state,
                            &config_path,
                            &reload_mutex,
                            &last_reload_time,
                            &pending_reload,
                        );
                    }
                    Err(_) => {
                        // Channel closed, likely shutting down
                        debug!("📡 Reload channel closed, stopping debounced reload thread");
                        break;
                    }
                }
            }
            
            debug!("🧵 Debounced reload thread stopped");
        });
    }

    fn perform_debounced_reload(
        state: &Arc<RwLock<DaemonState>>,
        config_path: &PathBuf,
        reload_mutex: &Arc<Mutex<()>>,
        last_reload_time: &Arc<Mutex<Instant>>,
        pending_reload: &Arc<AtomicBool>,
    ) {
        use std::time::{Duration, Instant};
        
        let start_time = Instant::now();
        let debounce_delay = Duration::from_millis(DEBOUNCE_DELAY_MS);
        let max_wait = Duration::from_millis(MAX_DEBOUNCE_WAIT_MS);

        debug!("⏱️ Starting debounced reload (delay: {}ms, max wait: {}ms)", DEBOUNCE_DELAY_MS, MAX_DEBOUNCE_WAIT_MS);

        // Debouncing loop
        loop {
            // Check if we've exceeded maximum wait time
            if start_time.elapsed() >= max_wait {
                debug!("⏰ Maximum debounce wait time exceeded, forcing reload");
                break;
            }

            // Wait for debounce delay
            thread::sleep(debounce_delay);

            // Check if there are still pending changes
            if !pending_reload.load(Ordering::Relaxed) {
                debug!("✅ No pending reload, debounce complete");
                return;  // No reload needed
            }

            // Check if another change happened during our wait
            let time_since_last_change = {
                let last_reload = last_reload_time.lock().expect("Failed to lock last reload time");
                last_reload.elapsed()
            };

            if time_since_last_change >= debounce_delay {
                debug!("⏱️ Debounce period elapsed, proceeding with reload");
                break;  // Proceed with reload
            }

            debug!("⏳ Additional changes detected, continuing debounce...");
        }

        // Clear pending reload flag
        pending_reload.store(false, Ordering::Relaxed);

        // Perform the actual reload
        debug!("🔄 Executing debounced config reload");
        
        // Acquire reload mutex to ensure only one reload at a time
        let _reload_guard = match reload_mutex.lock() {
            Ok(guard) => guard,
            Err(e) => {
                error!("Failed to acquire reload mutex: {}", e);
                return;
            }
        };

        let reload_start = Instant::now();

        // Calculate new hash
        let new_hash = match hash_config_file(config_path) {
            Ok(hash) => hash,
            Err(e) => {
                error!("Failed to calculate config hash: {}", e);
                return;
            }
        };

        // Check current state atomically
        let needs_reload = {
            match state.read() {
                Ok(state_guard) => state_guard.config_hash != new_hash,
                Err(e) => {
                    error!("Failed to acquire read lock on state: {}", e);
                    return;
                }
            }
        };

        if !needs_reload {
            debug!("⚡ Config hash unchanged during debounce, skipping reload");
            return;
        }

        debug!("🔄 Config hash changed during debounce, reloading...");

        // Load new config
        let home_dir = match dirs::home_dir() {
            Some(dir) => dir,
            None => {
                error!("Unable to determine home directory");
                return;
            }
        };

        let mut new_aka = match AKA::new(false, home_dir.clone()) {
            Ok(aka) => aka,
            Err(e) => {
                error!("Failed to load new config: {}", e);
                return;
            }
        };

        // Migrate usage counts from old cache to new cache
        let old_hash = {
            match state.read() {
                Ok(state_guard) => state_guard.config_hash.clone(),
                Err(e) => {
                    error!("Failed to acquire read lock on state: {}", e);
                    return;
                }
            }
        };

        if old_hash != new_hash {
            if let Err(e) = aka_lib::migrate_alias_counts(&old_hash, &new_hash, &mut new_aka.spec.aliases) {
                debug!("⚠️ Failed to migrate alias counts: {}", e);
            } else {
                // Save the migrated aliases back to cache
                if let Err(e) = aka_lib::save_alias_cache(&new_hash, &new_aka.spec.aliases, &home_dir) {
                    debug!("⚠️ Failed to save migrated alias cache: {}", e);
                }
            }
        }

        // ATOMIC STATE UPDATE: Update both config and hash together
        {
            match state.write() {
                Ok(mut state_guard) => {
                    // Update both aka and hash atomically
                    state_guard.aka = new_aka;
                    state_guard.config_hash = new_hash.clone();
                }
                Err(e) => {
                    error!("Failed to acquire write lock on state: {}", e);
                    return;
                }
            }
        }

        // Store hash for CLI comparison
        if let Err(e) = store_hash(&new_hash, &home_dir) {
            warn!("Failed to store updated config hash: {}", e);
        }

        // Update last reload time
        {
            match last_reload_time.lock() {
                Ok(mut last_reload) => *last_reload = Instant::now(),
                Err(e) => {
                    error!("Failed to acquire lock on last reload time: {}", e);
                    return;
                }
            }
        }

        let reload_duration = reload_start.elapsed();
        let alias_count = {
            match state.read() {
                Ok(state_guard) => state_guard.aka.spec.aliases.len(),
                Err(e) => {
                    error!("Failed to acquire read lock on state: {}", e);
                    return;
                }
            }
        };

        debug!("✅ Debounced config reload complete: {:.3}ms, {} aliases", reload_duration.as_secs_f64() * 1000.0, alias_count);
        debug!("🔒 New config hash: {}", new_hash);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_opts_parsing() {
        // Test parsing command line options
        let args = vec!["aka-daemon", "--foreground", "--config", "/tmp/test.yml"];
        let opts = DaemonOpts::try_parse_from(args).unwrap();
        
        assert!(opts.foreground);
        assert_eq!(opts.config, Some(PathBuf::from("/tmp/test.yml")));
    }

    #[test]
    fn test_request_serialization() {
        // Test that requests serialize correctly
        let request = Request::Query {
            cmdline: "test command".to_string(),
            eol: true,
        };
        
        let serialized = serde_json::to_string(&request).unwrap();
        let deserialized: Request = serde_json::from_str(&serialized).unwrap();
        
        match deserialized {
            Request::Query { cmdline, eol } => {
                assert_eq!(cmdline, "test command");
                assert!(eol);
            }
            _ => panic!("Wrong request type"),
        }
    }

    #[test]
    fn test_response_serialization() {
        // Test that responses serialize correctly
        let response = Response::Success {
            data: "test result".to_string(),
        };
        
        let serialized = serde_json::to_string(&response).unwrap();
        let deserialized: Response = serde_json::from_str(&serialized).unwrap();
        
        match deserialized {
            Response::Success { data } => {
                assert_eq!(data, "test result");
            }
            _ => panic!("Wrong response type"),
        }
    }

    #[test]
    fn test_request_response_roundtrip() {
        // Test full request-response cycle serialization
        let request = Request::Health;
        let response = Response::Health {
            status: "healthy:5:aliases:abc123:synced".to_string(),
        };
        
        // Serialize request
        let request_json = serde_json::to_string(&request).unwrap();
        let parsed_request: Request = serde_json::from_str(&request_json).unwrap();
        
        // Serialize response
        let response_json = serde_json::to_string(&response).unwrap();
        let parsed_response: Response = serde_json::from_str(&response_json).unwrap();
        
        // Verify they match
        assert!(matches!(parsed_request, Request::Health));
        match parsed_response {
            Response::Health { status } => {
                assert_eq!(status, "healthy:5:aliases:abc123:synced");
            }
            _ => panic!("Wrong response type"),
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
    
    info!("🚀 Starting AKA daemon...");
    
    // Create daemon server
    let daemon = match DaemonServer::new(&opts.config) {
        Ok(daemon) => daemon,
        Err(e) => {
            error!("Failed to create daemon server: {}", e);
            std::process::exit(1);
        }
    };
    
    // Determine socket path
    let socket_path = match determine_socket_path(&home_dir) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to determine socket path: {}", e);
            std::process::exit(1);
        }
    };
    
    // Run daemon
    if let Err(e) = daemon.run(&socket_path) {
        error!("Daemon error: {}", e);
        std::process::exit(1);
    }
    
    info!("🛑 AKA daemon shutting down");
}
