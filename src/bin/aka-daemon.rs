use clap::Parser;
use std::path::PathBuf;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{BufRead, BufReader, Write};
use std::thread;
use serde::{Deserialize, Serialize};
use log::{info, error, debug, warn};

// Import from the shared library
use aka_lib::{determine_socket_path, AKA, setup_logging};

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
        let aka = AKA::new(false, config)?;
        let shutdown = Arc::new(AtomicBool::new(false));
        
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
                match self.aka.replace(&cmdline) {
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
                Response::Health { status: "healthy".to_string() }
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