use clap::Parser;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use eyre::{eyre, Result};
use log::{debug, error, info, warn};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::thread;

// Import from the shared library
use aka_lib::{
    determine_socket_path, get_config_path_with_override, hash_config_file, save_alias_cache, setup_logging,
    AliasCache, DaemonRequest, DaemonResponse, ProcessingMode, AKA,
};

// Version constant for compatibility checking
const DAEMON_VERSION: &str = env!("GIT_DESCRIBE");

// Read/write timeout on an accepted client stream. The accept loop is serial,
// so a client that connects and never sends a newline must not stall the whole
// daemon on an unbounded `read_line`. Same 500ms class as the client probes.
const CLIENT_IO_TIMEOUT_MS: u64 = 500;

// Minimum interval between debounced cache flushes. On every daemon-mode query
// the daemon buffers usage counts in memory and marks the cache dirty; the
// file-watcher loop's 100ms tick flushes at most once per this window instead
// of rewriting the whole cache on every query.
const CACHE_FLUSH_INTERVAL_SECS: u64 = 5;

// After this many consecutive flush failures the daemon appends
// `:degraded-persistence` to its Health status so `aka daemon --status` can
// surface that usage counts are not being persisted (counts stay advisory).
const FLUSH_DEGRADED_THRESHOLD: u32 = 5;

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

// Timing state for the debounced cache flush. `last_flush` gates the 5s cadence;
// `last_successful_flush` is log-visible observability only.
struct FlushTiming {
    last_flush: Instant,
    last_successful_flush: Option<Instant>,
}

// Shared debounced-flush state. Bundled so the watcher thread and the server's
// own paths pass one clonable handle (the Arcs are shared, not duplicated).
#[derive(Clone)]
struct FlushHandle {
    // Set true after any daemon-mode query that transforms the command line
    // (its usage counts changed); cleared by a successful flush.
    cache_dirty: Arc<AtomicBool>,
    // Consecutive flush failures; drives the Health `:degraded-persistence` suffix.
    failures: Arc<AtomicU32>,
    timing: Arc<Mutex<FlushTiming>>,
}

impl FlushHandle {
    fn new() -> Self {
        FlushHandle {
            cache_dirty: Arc::new(AtomicBool::new(false)),
            failures: Arc::new(AtomicU32::new(0)),
            timing: Arc::new(Mutex::new(FlushTiming {
                last_flush: Instant::now(),
                last_successful_flush: None,
            })),
        }
    }
}

struct DaemonServer {
    aka: Arc<RwLock<AKA>>,
    config_path: PathBuf,
    config_hash: Arc<RwLock<String>>,
    shutdown: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
    reload_receiver: Arc<Mutex<Receiver<()>>>,
    flush: FlushHandle,
}

impl DaemonServer {
    fn new(config: &Option<PathBuf>) -> Result<Self> {
        use std::time::Instant;

        let start_daemon_init = Instant::now();
        debug!("🚀 Daemon initializing, loading config...");

        // Determine config path using the same logic as direct mode
        // Respect HOME environment variable for tests
        let home_dir = std::env::var("HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let config_path = get_config_path_with_override(&home_dir, config)?;

        // Load initial config
        let aka = AKA::new(false, home_dir.clone(), config_path.clone())?;
        let aka = Arc::new(RwLock::new(aka));

        // Calculate initial config hash
        let initial_hash = hash_config_file(&config_path)?;
        let config_hash = Arc::new(RwLock::new(initial_hash.clone()));

        let shutdown = Arc::new(AtomicBool::new(false));

        // Set up file watcher
        let (reload_sender, reload_receiver) = channel();
        let reload_receiver = Arc::new(Mutex::new(reload_receiver));

        let config_path_for_watcher = config_path.clone();
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
            Ok(event) => {
                if let EventKind::Modify(_) = event.kind {
                    if event.paths.iter().any(|p| p == &config_path_for_watcher) {
                        debug!("📁 Config file change detected: {config_path_for_watcher:?}");
                        if let Err(e) = reload_sender.send(()) {
                            error!("Failed to send reload signal: {e}");
                        }
                    }
                }
            }
            Err(e) => error!("File watcher error: {e}"),
        })
        .map_err(|e| eyre!("Failed to create file watcher: {}", e))?;

        // Watch the config file
        watcher
            .watch(&config_path, RecursiveMode::NonRecursive)
            .map_err(|e| eyre!("Failed to watch config file: {}", e))?;
        debug!("👀 File watcher set up for: {config_path:?}");

        let daemon_init_duration = start_daemon_init.elapsed();
        debug!(
            "✅ Daemon initialization complete: {:.3}ms",
            daemon_init_duration.as_secs_f64() * 1000.0
        );

        let alias_count = {
            let aka_guard = aka
                .read()
                .map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
            aka_guard.spec.aliases.len()
        };
        debug!("📦 Daemon has {alias_count} aliases cached in memory");
        debug!("🔒 Initial config hash: {initial_hash}");

        Ok(DaemonServer {
            aka,
            config_path,
            config_hash,
            shutdown,
            _watcher: Some(watcher),
            reload_receiver,
            flush: FlushHandle::new(),
        })
    }

    /// Flush buffered usage counts to the cache file if the daemon has recorded
    /// any since the last successful flush. This is the ONLY place daemon code
    /// writes the cache (ownership invariant).
    ///
    /// Advisory (Data Model): a failed flush warns, keeps the dirty flag set,
    /// and is retried later; it never propagates and never stops the daemon.
    ///
    /// Lock order: take the AKA read lock, snapshot, DROP the lock, then write
    /// (atomic tmp+rename inside `save_alias_cache`). The dirty flag is claimed
    /// with an atomic read-and-clear up front, so a query that fires mid-flush
    /// re-sets it (worst case: one redundant flush next tick, never a lost
    /// count); on write failure the flag is re-armed.
    fn flush_counts(aka: &Arc<RwLock<AKA>>, flush: &FlushHandle) {
        if !flush.cache_dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        debug!("flush_counts: cache dirty, flushing usage counts");
        let start = Instant::now();

        // Reset the cadence timer on every attempt (success or failure) so a
        // persistently failing disk retries at the flush interval, not every tick.
        if let Ok(mut timing) = flush.timing.lock() {
            timing.last_flush = Instant::now();
        }

        // Snapshot under the read lock, then release before touching the disk.
        let snapshot = match aka.read() {
            Ok(guard) => Some((
                AliasCache {
                    hash: guard.config_hash.clone(),
                    aliases: guard.spec.aliases.clone(),
                },
                guard.home_dir.clone(),
                guard.spec.aliases.len(),
            )),
            Err(e) => {
                warn!("flush_counts: failed to acquire read lock on AKA: {e}");
                None
            }
        };

        let Some((cache, home_dir, alias_count)) = snapshot else {
            // Couldn't snapshot; re-arm so the next tick retries.
            flush.cache_dirty.store(true, Ordering::Release);
            return;
        };

        match save_alias_cache(&cache, &home_dir) {
            Ok(()) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                flush.failures.store(0, Ordering::Relaxed);
                if let Ok(mut timing) = flush.timing.lock() {
                    timing.last_successful_flush = Some(Instant::now());
                }
                debug!("flush_counts: flushed {alias_count} aliases in {elapsed_ms:.3}ms");
            }
            Err(e) => {
                // Advisory: keep the dirty flag set and retry on the next window.
                flush.cache_dirty.store(true, Ordering::Release);
                let failures = flush.failures.fetch_add(1, Ordering::Relaxed) + 1;
                warn!("flush_counts: failed to persist usage counts (consecutive failures: {failures}): {e}");
            }
        }
    }

    /// `&self` convenience over [`Self::flush_counts`] for the paths that own the
    /// server: pre-Freq, manual reload, version-mismatch shutdown, and run()
    /// cleanup. Unconditional (ignores the cadence timer): flushes if dirty.
    fn flush_counts_if_dirty(&self) {
        Self::flush_counts(&self.aka, &self.flush);
    }

    /// True if at least `CACHE_FLUSH_INTERVAL_SECS` have elapsed since the last
    /// flush attempt. A poisoned timing lock errs toward flushing.
    fn flush_due(flush: &FlushHandle) -> bool {
        match flush.timing.lock() {
            Ok(timing) => timing.last_flush.elapsed() >= Duration::from_secs(CACHE_FLUSH_INTERVAL_SECS),
            Err(e) => {
                warn!("flush_due: timing lock poisoned, flushing defensively: {e}");
                true
            }
        }
    }

    fn reload_config(&self) -> Result<String> {
        use std::time::Instant;

        let start_reload = Instant::now();
        debug!("🔄 Manual config reload requested");

        // Calculate new hash
        let new_hash = hash_config_file(&self.config_path)?;
        let current_hash = {
            let hash_guard = self
                .config_hash
                .read()
                .map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;
            hash_guard.clone()
        };

        if new_hash == current_hash {
            debug!("⚡ Config hash unchanged, skipping reload");
            return Ok("Config unchanged".to_string());
        }

        debug!("🔄 Config hash changed: {current_hash} -> {new_hash}");

        // Flush buffered counts before reconstruction: AKA::new re-reads the
        // cache from disk, so any in-memory counts must hit the file first or
        // they are lost by the reload.
        self.flush_counts_if_dirty();

        // Load new config using sync function
        let home_dir = dirs::home_dir().ok_or_else(|| eyre!("Unable to determine home directory"))?;
        let new_aka = AKA::new(false, home_dir.clone(), self.config_path.clone())?;

        // Update stored config and hash atomically (hold both locks simultaneously)
        {
            let mut aka_guard = self
                .aka
                .write()
                .map_err(|e| eyre!("Failed to acquire write lock on AKA: {}", e))?;
            let mut hash_guard = self
                .config_hash
                .write()
                .map_err(|e| eyre!("Failed to acquire write lock on config hash: {}", e))?;

            *aka_guard = new_aka;
            *hash_guard = new_hash.clone();
        }

        let reload_duration = start_reload.elapsed();
        let alias_count = {
            let aka_guard = self
                .aka
                .read()
                .map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
            aka_guard.spec.aliases.len()
        };

        let message = format!(
            "Config reloaded: {} aliases in {:.3}ms",
            alias_count,
            reload_duration.as_secs_f64() * 1000.0
        );
        debug!("✅ {message}");

        Ok(message)
    }

    fn ensure_cache_fresh(&self) -> Result<()> {
        let current_hash = hash_config_file(&self.config_path)?;
        let cached_hash = {
            let hash_guard = self
                .config_hash
                .read()
                .map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;
            hash_guard.clone()
        };

        if current_hash != cached_hash {
            warn!("⚠️  Config hash mismatch detected, auto-reloading");
            warn!("   Cached: {cached_hash} → Current: {current_hash}");
            if let Err(e) = self.reload_config() {
                warn!("❌ Auto-reload failed (keeping previous config): {e}");
            }
        }

        Ok(())
    }

    fn check_version_compatibility(&self, client_version: &str) -> Result<()> {
        if client_version != DAEMON_VERSION {
            warn!("🔄 Version mismatch detected!");
            warn!("   Daemon version: {DAEMON_VERSION}");
            warn!("   Client version: {client_version}");
            warn!("   Initiating graceful shutdown for auto-restart");

            // Flush buffered counts before the restart: the replacement daemon
            // process reconstructs AKA from the cache file on startup, so any
            // in-memory counts must be persisted first.
            self.flush_counts_if_dirty();

            // Trigger shutdown
            self.shutdown.store(true, Ordering::Relaxed);

            // Return error to stop processing this request
            return Err(eyre!(
                "Version mismatch: daemon={}, client={}. Daemon shutting down for restart.",
                DAEMON_VERSION,
                client_version
            ));
        }
        Ok(())
    }

    fn process_health_request(&self) -> Result<Response> {
        let aka_guard = self
            .aka
            .read()
            .map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;
        let hash_guard = self
            .config_hash
            .read()
            .map_err(|e| eyre!("Failed to acquire read lock on config hash: {}", e))?;

        debug!("📤 Processing health check");

        // Check if config is in sync
        let current_hash = match hash_config_file(&self.config_path) {
            Ok(hash) => hash,
            Err(e) => {
                warn!("❌ Failed to calculate config hash: {e}");
                return Ok(Response::Error {
                    message: format!("Failed to calculate config hash: {e}"),
                });
            }
        };

        let mut status = if current_hash == *hash_guard {
            format!("healthy:{}:synced", aka_guard.spec.aliases.len())
        } else {
            format!("healthy:{}:stale", aka_guard.spec.aliases.len())
        };

        // Surface degraded persistence (advisory counts not reaching disk) so
        // `aka daemon --status` can report it. Routing is unaffected: the client
        // parser tolerates this suffix and still treats the daemon as reachable.
        let failures = self.flush.failures.load(Ordering::Relaxed);
        if failures >= FLUSH_DEGRADED_THRESHOLD {
            warn!("⚠️ Persistence degraded: {failures} consecutive cache-flush failures");
            status.push_str(":degraded-persistence");
        }

        debug!("✅ Health check complete: {status}");
        Ok(Response::Health { status })
    }

    fn handle_client(&self, mut stream: UnixStream) -> Result<()> {
        // Bound reads/writes: a client that connects and never sends a newline
        // must not stall the serial accept loop on an unbounded read_line.
        let io_timeout = std::time::Duration::from_millis(CLIENT_IO_TIMEOUT_MS);
        if let Err(e) = stream.set_read_timeout(Some(io_timeout)) {
            warn!("⚠️ Failed to set client read timeout: {e}");
        }
        if let Err(e) = stream.set_write_timeout(Some(io_timeout)) {
            warn!("⚠️ Failed to set client write timeout: {e}");
        }

        // Check if config has changed and reload if necessary
        self.ensure_cache_fresh()?;

        let mut reader = BufReader::new(&stream);
        let mut line = String::new();

        // Read request line
        reader.read_line(&mut line)?;

        // Basic message size check
        if let Err(e) = aka_lib::protocol::validate_message_size(&line) {
            let error_response = Response::Error {
                message: format!("Message too large: {e}"),
            };
            let response_json = serde_json::to_string(&error_response)?;
            writeln!(stream, "{response_json}")?;
            return Ok(());
        }

        let request: Request = serde_json::from_str(line.trim())?;

        debug!("Received request: {request:?}");

        // Extract version from request and check compatibility
        let client_version = match &request {
            Request::Query { version, .. } => Some(version.as_str()),
            Request::List { version, .. } => Some(version.as_str()),
            Request::Freq { version, .. } => Some(version.as_str()),
            Request::CompleteAliases { version, .. } => Some(version.as_str()),
            _ => None, // Admin commands don't require version check
        };

        if let Some(client_version) = client_version {
            if let Err(e) = self.check_version_compatibility(client_version) {
                // Send version mismatch response
                let response = Response::VersionMismatch {
                    daemon_version: DAEMON_VERSION.to_string(),
                    client_version: client_version.to_string(),
                    message: "Daemon restarting to match client version".to_string(),
                };
                let response_json = serde_json::to_string(&response)?;
                writeln!(stream, "{response_json}")?;

                // Return error to trigger shutdown
                return Err(e);
            }
        }

        let response = match request {
            Request::Query {
                version: _,
                cmdline,
                eol,
                config,
            } => {
                debug!("📤 Processing query: {cmdline} (config: {config:?})");

                match &config {
                    Some(custom_config_path) => {
                        // Custom config - create temporary AKA instance
                        debug!("🔧 Using custom config for query: {custom_config_path:?}");
                        let home_dir = std::env::var("HOME")
                            .ok()
                            .map(PathBuf::from)
                            .or_else(dirs::home_dir)
                            .ok_or_else(|| eyre!("Unable to determine home directory"))?;

                        let config_path = get_config_path_with_override(&home_dir, &config)?;
                        let mut temp_aka = AKA::new(eol, home_dir, config_path)?;

                        match temp_aka.replace_with_mode(&cmdline, ProcessingMode::Direct) {
                            Ok(result) => {
                                debug!("✅ Custom config query processed successfully");
                                Response::Success { data: result }
                            }
                            Err(e) => {
                                warn!("❌ Custom config query processing failed: {e}");
                                Response::Error { message: e.to_string() }
                            }
                        }
                    }
                    None => {
                        // Default config - use daemon's AKA instance
                        debug!("🔧 Using daemon's default config for query");
                        let mut aka_guard = self
                            .aka
                            .write()
                            .map_err(|e| eyre!("Failed to acquire write lock on AKA: {}", e))?;
                        // Update AKA's eol setting to match the request
                        aka_guard.eol = eol;

                        match aka_guard.replace_with_mode(&cmdline, ProcessingMode::Daemon) {
                            Ok(result) => {
                                debug!("✅ Query processed successfully");
                                // A non-empty result means the command line was
                                // transformed, so usage counts changed in memory.
                                // Mark the cache dirty; the debounce timer flushes
                                // it. (A sudo-only transform with no alias bump is a
                                // harmless redundant flush at worst.)
                                if !result.is_empty() {
                                    self.flush.cache_dirty.store(true, Ordering::Relaxed);
                                }
                                Response::Success { data: result }
                            }
                            Err(e) => {
                                warn!("❌ Query processing failed: {e}");
                                Response::Error { message: e.to_string() }
                            }
                        }
                    }
                }
            }
            Request::List {
                version: _,
                global,
                patterns,
                config,
            } => {
                debug!("📤 Processing list request (global: {global}, patterns: {patterns:?}, config: {config:?})");

                match &config {
                    Some(custom_config_path) => {
                        // Custom config - create temporary AKA instance
                        debug!("🔧 Using custom config for list: {custom_config_path:?}");
                        let home_dir = std::env::var("HOME")
                            .ok()
                            .map(PathBuf::from)
                            .or_else(dirs::home_dir)
                            .ok_or_else(|| eyre!("Unable to determine home directory"))?;

                        let config_path = get_config_path_with_override(&home_dir, &config)?;
                        let temp_aka = AKA::new(false, home_dir, config_path)?;

                        let output = aka_lib::format_aliases_efficiently(
                            temp_aka.spec.aliases.values(),
                            false, // show_counts
                            true,  // show_all
                            global,
                            &patterns,
                        );

                        debug!("✅ Custom config list processed successfully");
                        Response::Success { data: output }
                    }
                    None => {
                        // Default config - use daemon's AKA instance
                        debug!("🔧 Using daemon's default config for list");
                        let aka_guard = self
                            .aka
                            .read()
                            .map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;

                        let output = aka_lib::format_aliases_efficiently(
                            aka_guard.spec.aliases.values(),
                            false, // show_counts
                            true,  // show_all
                            global,
                            &patterns,
                        );

                        debug!("✅ List processed successfully");
                        Response::Success { data: output }
                    }
                }
            }
            Request::Freq {
                version: _,
                all,
                config,
            } => {
                debug!("📤 Processing frequency request (all: {all}, config: {config:?})");

                // Daemon-served freq reads counts from memory (always fresh), but
                // flush first so the cache FILE is current for any later
                // direct-mode fallback that reads it.
                self.flush_counts_if_dirty();

                match &config {
                    Some(custom_config_path) => {
                        // Custom config - create temporary AKA instance
                        debug!("🔧 Using custom config for freq: {custom_config_path:?}");
                        let home_dir = std::env::var("HOME")
                            .ok()
                            .map(PathBuf::from)
                            .or_else(dirs::home_dir)
                            .ok_or_else(|| eyre!("Unable to determine home directory"))?;

                        let config_path = get_config_path_with_override(&home_dir, &config)?;
                        let temp_aka = AKA::new(false, home_dir, config_path)?;

                        let output = aka_lib::format_aliases_efficiently(
                            temp_aka.spec.aliases.values(),
                            true, // show_counts
                            all,
                            false, // global_only
                            &[],   // patterns
                        );

                        debug!("✅ Custom config frequency processed successfully");
                        Response::Success { data: output }
                    }
                    None => {
                        // Default config - use daemon's AKA instance
                        debug!("🔧 Using daemon's default config for freq");
                        let aka_guard = self
                            .aka
                            .read()
                            .map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;

                        let output = aka_lib::format_aliases_efficiently(
                            aka_guard.spec.aliases.values(),
                            true, // show_counts
                            all,
                            false, // global_only
                            &[],   // patterns
                        );

                        debug!("✅ Frequency processed successfully");
                        Response::Success { data: output }
                    }
                }
            }
            Request::Health => self.process_health_request()?,
            Request::ReloadConfig => {
                debug!("📤 Processing config reload request");
                match self.reload_config() {
                    Ok(message) => {
                        debug!("✅ Config reload completed successfully");
                        Response::ConfigReloaded { success: true, message }
                    }
                    Err(e) => {
                        warn!("❌ Config reload failed: {e}");
                        Response::ConfigReloaded {
                            success: false,
                            message: e.to_string(),
                        }
                    }
                }
            }
            Request::Shutdown => {
                debug!("📤 Processing shutdown request");
                self.shutdown.store(true, Ordering::Relaxed);
                Response::ShutdownAck
            }
            Request::CompleteAliases { version: _, config } => {
                debug!("📤 Processing complete aliases request (config: {config:?})");

                match &config {
                    Some(custom_config_path) => {
                        // Custom config - create temporary AKA instance
                        debug!("🔧 Using custom config for complete aliases: {custom_config_path:?}");
                        let home_dir = std::env::var("HOME")
                            .ok()
                            .map(PathBuf::from)
                            .or_else(dirs::home_dir)
                            .ok_or_else(|| eyre!("Unable to determine home directory"))?;

                        let config_path = get_config_path_with_override(&home_dir, &config)?;
                        let temp_aka = AKA::new(false, home_dir, config_path)?;

                        let alias_names = aka_lib::get_alias_names_for_completion(&temp_aka);
                        let output = alias_names.join("\n");

                        debug!("✅ Custom config complete aliases processed successfully");
                        Response::Success { data: output }
                    }
                    None => {
                        // Default config - use daemon's AKA instance
                        debug!("🔧 Using daemon's default config for complete aliases");
                        let aka_guard = self
                            .aka
                            .read()
                            .map_err(|e| eyre!("Failed to acquire read lock on AKA: {}", e))?;

                        let alias_names = aka_lib::get_alias_names_for_completion(&aka_guard);
                        let output = alias_names.join("\n");

                        debug!("✅ Complete aliases processed successfully");
                        Response::Success { data: output }
                    }
                }
            }
        };

        let response_json = serde_json::to_string(&response)?;
        writeln!(stream, "{response_json}")?;

        Ok(())
    }

    fn handle_config_file_change(
        new_hash: String,
        current_hash: String,
        aka_for_watcher: &Arc<RwLock<AKA>>,
        config_hash_for_watcher: &Arc<RwLock<String>>,
        home_dir: PathBuf,
        config_path: PathBuf,
    ) -> Result<()> {
        debug!("🔄 Auto-reload: hash changed {current_hash} -> {new_hash}");

        // Load new config using sync function
        match AKA::new(false, home_dir.clone(), config_path) {
            Ok(new_aka) => {
                // Update stored config and hash atomically (hold both locks simultaneously)
                {
                    match (aka_for_watcher.write(), config_hash_for_watcher.write()) {
                        (Ok(mut aka_guard), Ok(mut hash_guard)) => {
                            *aka_guard = new_aka;
                            *hash_guard = new_hash.clone();
                        }
                        (Err(e), _) => {
                            error!("Failed to acquire write lock on AKA: {e}");
                            return Err(eyre!("Failed to acquire write lock on AKA: {}", e));
                        }
                        (_, Err(e)) => {
                            error!("Failed to acquire write lock on config hash: {e}");
                            return Err(eyre!("Failed to acquire write lock on config hash: {}", e));
                        }
                    }
                }

                debug!("✅ Auto-reload completed successfully");
                Ok(())
            }
            Err(e) => {
                error!("Failed to reload config: {e}");
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
        flush: FlushHandle,
    ) -> Result<()> {
        while !shutdown_for_watcher.load(Ordering::Relaxed) {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(()) => {
                    debug!("📁 File change detected, reloading config automatically");

                    // Flush buffered counts before reconstruction: the reload
                    // rebuilds AKA from the cache file, so counts must land first.
                    Self::flush_counts(&aka_for_watcher, &flush);

                    // Calculate new hash
                    match hash_config_file(&config_path_for_watcher) {
                        Ok(new_hash) => {
                            let current_hash = {
                                match config_hash_for_watcher.read() {
                                    Ok(guard) => guard.clone(),
                                    Err(e) => {
                                        error!("Failed to acquire read lock on config hash: {e}");
                                        continue;
                                    }
                                }
                            };

                            if new_hash != current_hash {
                                let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
                                if let Err(e) = Self::handle_config_file_change(
                                    new_hash,
                                    current_hash,
                                    &aka_for_watcher,
                                    &config_hash_for_watcher,
                                    home_dir,
                                    config_path_for_watcher.clone(),
                                ) {
                                    error!("Failed to handle config file change: {e}");
                                }
                            } else {
                                debug!("⚡ Auto-reload: hash unchanged, skipping");
                            }
                        }
                        Err(e) => {
                            error!("Failed to calculate config hash for auto-reload: {e}");
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Debounced flush: piggyback on the 100ms tick, but only
                    // actually write once per CACHE_FLUSH_INTERVAL_SECS.
                    if flush.cache_dirty.load(Ordering::Relaxed) && Self::flush_due(&flush) {
                        Self::flush_counts(&aka_for_watcher, &flush);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    debug!("📁 Reload channel disconnected, stopping watcher loop");
                    break;
                }
            }
        }
        debug!("🛑 File watcher thread shutting down");
        Ok(())
    }

    fn handle_incoming_connections(&self, listener: UnixListener) -> Result<()> {
        // Main server loop
        for stream in listener.incoming() {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }

            match stream {
                Ok(stream) => {
                    if let Err(e) = self.handle_client(stream) {
                        error!("Error handling client: {e}");
                    }
                }
                Err(e) => {
                    error!("Error accepting connection: {e}");
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
        debug!("📡 Socket listening at: {socket_path:?}");

        // Start background file watching thread
        let reload_receiver = Arc::clone(&self.reload_receiver);
        let aka_for_watcher = Arc::clone(&self.aka);
        let config_path_for_watcher = self.config_path.clone();
        let config_hash_for_watcher = Arc::clone(&self.config_hash);
        let shutdown_for_watcher = Arc::clone(&self.shutdown);
        let flush_for_watcher = self.flush.clone();

        thread::spawn(move || {
            let receiver = reload_receiver.lock().map_err(|e| {
                error!("Failed to acquire lock on reload receiver: {e}");
            });

            if let Ok(receiver) = receiver {
                if let Err(e) = Self::handle_file_watcher_loop(
                    &receiver,
                    shutdown_for_watcher,
                    config_path_for_watcher,
                    aka_for_watcher,
                    config_hash_for_watcher,
                    flush_for_watcher,
                ) {
                    error!("Failed to run file watcher loop: {e}");
                }
            }
        });

        let result = self.handle_incoming_connections(listener);

        // Final flush on shutdown: the signal handler only sets the flag and
        // removes the socket (it runs in a signal context), so persisting any
        // buffered counts belongs here on the main thread after the accept loop.
        self.flush_counts_if_dirty();

        // Clean up socket file on shutdown
        if socket_path.exists() {
            debug!("🧹 Cleaning up socket file on shutdown");
            if let Err(e) = fs::remove_file(socket_path) {
                error!("Failed to remove socket file on shutdown: {e}");
            } else {
                debug!("✅ Socket file removed successfully");
            }
        }

        result
    }
}

fn initialize_daemon_server(opts: &DaemonOpts, home_dir: &std::path::Path) -> Result<(DaemonServer, PathBuf)> {
    // Determine socket path
    let socket_path = match determine_socket_path(home_dir) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to determine socket path: {e}");
            std::process::exit(1);
        }
    };

    // Create daemon server
    let server = match DaemonServer::new(&opts.config) {
        Ok(server) => server,
        Err(e) => {
            error!("Failed to create daemon server: {e}");
            std::process::exit(1);
        }
    };

    Ok((server, socket_path))
}

fn main() {
    let opts = DaemonOpts::parse();

    // Force colored output - daemon output goes to client which displays on terminal
    colored::control::set_override(true);

    // Set up logging - respect HOME environment variable for tests
    let home_dir = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .ok_or_else(|| eyre!("Unable to determine home directory"))
        .unwrap_or_else(|e| {
            eprintln!("Error: {e}");
            std::process::exit(1);
        });
    if let Err(e) = setup_logging(&home_dir) {
        eprintln!("Warning: Failed to set up logging: {e}");
    }

    info!("🚀 AKA Daemon starting...");

    // Initialize daemon server
    let (server, socket_path) = match initialize_daemon_server(&opts, &home_dir) {
        Ok((server, socket_path)) => (server, socket_path),
        Err(e) => {
            error!("Failed to initialize daemon server: {e}");
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
                error!("Failed to remove socket file on signal: {e}");
            } else {
                debug!("✅ Socket file removed successfully on signal");
            }
        }
    }) {
        error!("Error setting signal handler: {e}");
        std::process::exit(1);
    }

    info!("✅ Daemon running (PID: {})", std::process::id());

    // Run the server
    if let Err(e) = server.run(&socket_path) {
        error!("Server error: {e}");
        std::process::exit(1);
    }

    info!("👋 Daemon stopped");
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
    fn test_daemon_opts_no_config() {
        let opts = DaemonOpts {
            foreground: false,
            config: None,
        };
        assert!(!opts.foreground);
        assert!(opts.config.is_none());
    }

    #[test]
    fn test_request_serialization() {
        // Test that IPC requests can be serialized
        let health_request = Request::Health;
        let serialized = serde_json::to_string(&health_request);
        assert!(serialized.is_ok());

        let query_request = Request::Query {
            version: "v0.5.0".to_string(),
            cmdline: "test command".to_string(),
            eol: false,
            config: None,
        };
        let serialized = serde_json::to_string(&query_request);
        assert!(serialized.is_ok());

        let list_request = Request::List {
            version: "v0.5.0".to_string(),
            global: true,
            patterns: vec!["test".to_string()],
            config: None,
        };
        let serialized = serde_json::to_string(&list_request);
        assert!(serialized.is_ok());
    }

    #[test]
    fn test_response_serialization() {
        // Test that IPC responses can be serialized
        let success_response = Response::Success {
            data: "test data".to_string(),
        };
        let serialized = serde_json::to_string(&success_response);
        assert!(serialized.is_ok());

        let error_response = Response::Error {
            message: "test error".to_string(),
        };
        let serialized = serde_json::to_string(&error_response);
        assert!(serialized.is_ok());

        let health_response = Response::Health {
            status: "healthy:5:synced".to_string(),
        };
        let serialized = serde_json::to_string(&health_response);
        assert!(serialized.is_ok());
    }

    #[test]
    fn test_request_response_roundtrip() {
        // Test that requests and responses can be serialized and deserialized
        let original_request = Request::Query {
            version: "v0.5.0".to_string(),
            cmdline: "test command".to_string(),
            eol: true,
            config: None,
        };
        let serialized = serde_json::to_string(&original_request)
            .map_err(|e| eyre!("Failed to serialize request: {}", e))
            .expect("Serialization should succeed");
        let deserialized: Request = serde_json::from_str(&serialized)
            .map_err(|e| eyre!("Failed to deserialize request: {}", e))
            .expect("Deserialization should succeed");

        match (original_request, deserialized) {
            (
                Request::Query {
                    cmdline: orig,
                    eol: orig_eol,
                    ..
                },
                Request::Query {
                    cmdline: deser,
                    eol: deser_eol,
                    ..
                },
            ) => {
                assert_eq!(orig, deser);
                assert_eq!(orig_eol, deser_eol);
            }
            _ => panic!("Request roundtrip failed"),
        }
    }

    // Additional protocol tests
    #[test]
    fn test_request_freq_serialization() {
        let request = Request::Freq {
            version: "v0.5.0".to_string(),
            all: true,
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("Freq"));
        assert!(serialized.contains("all"));
    }

    #[test]
    fn test_request_reload_config_serialization() {
        let request = Request::ReloadConfig;
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("ReloadConfig"));
    }

    #[test]
    fn test_request_shutdown_serialization() {
        let request = Request::Shutdown;
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("Shutdown"));
    }

    #[test]
    fn test_request_complete_aliases_serialization() {
        let request = Request::CompleteAliases {
            version: "v0.5.0".to_string(),
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("CompleteAliases"));
    }

    #[test]
    fn test_request_with_config_override() {
        let config_path = PathBuf::from("/custom/config.yml");
        let request = Request::Query {
            version: "v0.5.0".to_string(),
            cmdline: "test".to_string(),
            eol: false,
            config: Some(config_path.clone()),
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("config"));

        let deserialized: Request = serde_json::from_str(&serialized).unwrap();
        if let Request::Query { config, .. } = deserialized {
            assert_eq!(config, Some(config_path));
        } else {
            panic!("Wrong request type");
        }
    }

    #[test]
    fn test_response_config_reloaded_serialization() {
        let response = Response::ConfigReloaded {
            success: true,
            message: "Config reloaded successfully".to_string(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("ConfigReloaded"));
        assert!(serialized.contains("success"));
    }

    #[test]
    fn test_response_shutdown_ack_serialization() {
        let response = Response::ShutdownAck;
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("ShutdownAck"));
    }

    #[test]
    fn test_response_version_mismatch_serialization() {
        let response = Response::VersionMismatch {
            daemon_version: "v1.0.0".to_string(),
            client_version: "v0.9.0".to_string(),
            message: "Please restart daemon".to_string(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("VersionMismatch"));
        assert!(serialized.contains("daemon_version"));
        assert!(serialized.contains("client_version"));
    }

    #[test]
    fn test_response_deserialization() {
        let json = r#"{"type":"Success","data":"test result"}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        if let Response::Success { data } = response {
            assert_eq!(data, "test result");
        } else {
            panic!("Wrong response type");
        }
    }

    #[test]
    fn test_request_list_with_multiple_patterns() {
        let request = Request::List {
            version: "v0.5.0".to_string(),
            global: false,
            patterns: vec!["git".to_string(), "docker".to_string(), "k8s".to_string()],
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("git"));
        assert!(serialized.contains("docker"));
        assert!(serialized.contains("k8s"));
    }

    #[test]
    fn test_request_query_with_eol_true() {
        let request = Request::Query {
            version: "v0.5.0".to_string(),
            cmdline: "ls -la !".to_string(),
            eol: true,
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("eol"));
        assert!(serialized.contains("true"));
    }

    #[test]
    fn test_daemon_version_constant() {
        // Verify the version constant is defined
        let version = DAEMON_VERSION;
        assert!(!version.is_empty());
    }
}
