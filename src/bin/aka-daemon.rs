use clap::Parser;
use std::path::PathBuf;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{BufRead, BufReader, Write};
use serde::{Deserialize, Serialize};
use log::{info, error, debug};

// Import from the shared library
use aka_lib::{determine_socket_path, AKA, setup_logging, ProcessingMode};

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
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum Response {
    Success { data: String },
    Error { message: String },
    Health { status: String },
}

struct DaemonServer {
    aka: AKA,
    shutdown: Arc<AtomicBool>,
}

impl DaemonServer {
    fn new(config: &Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        use std::time::Instant;

        let start_daemon_init = Instant::now();
        debug!("🚀 Daemon initializing, loading config...");

        let aka = AKA::new(false, config)?;
        let shutdown = Arc::new(AtomicBool::new(false));

        let daemon_init_duration = start_daemon_init.elapsed();
        debug!("✅ Daemon initialization complete: {:.3}ms", daemon_init_duration.as_secs_f64() * 1000.0);
        debug!("📦 Daemon has {} aliases cached in memory", aka.spec.aliases.len());

        Ok(DaemonServer { aka, shutdown })
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
                match self.aka.replace_with_mode(&cmdline, ProcessingMode::Daemon) {
                    Ok(result) => Response::Success { data: result },
                    Err(e) => Response::Error { message: e.to_string() },
                }
            },
            Request::List { global, patterns } => {
                let mut aliases: Vec<_> = self.aka.spec.aliases.values().cloned().collect();
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
                let alias_count = self.aka.spec.aliases.len();
                let status = format!("healthy:{}:aliases", alias_count);
                Response::Health { status }
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
        let serialized = serde_json::to_string(&original_request).unwrap();
        let deserialized: Request = serde_json::from_str(&serialized).unwrap();

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
