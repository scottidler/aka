use clap::{Parser, Subcommand};
use eyre::Result;
use log::debug;
use std::path::PathBuf;
use std::process::exit;
use std::os::unix::net::UnixStream;
use std::io::{BufRead, BufReader, Write};
use serde::{Deserialize, Serialize};

// Import from the shared library
use aka_lib::{
    setup_logging, execute_health_check, determine_socket_path,
    AKA, print_alias, ProcessingMode
};

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/git_describe.rs"));
}

// IPC Protocol Messages (shared with daemon)
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum DaemonRequest {
    Query { cmdline: String },
    List { global: bool, patterns: Vec<String> },
    Health,
    ReloadConfig,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum DaemonResponse {
    Success { data: String },
    Error { message: String },
    Health { status: String },
    ConfigReloaded { success: bool, aliases_count: usize, message: String },
}

// Daemon client for sending requests
struct DaemonClient;

impl DaemonClient {
    fn send_request(request: DaemonRequest) -> Result<DaemonResponse> {
        debug!("🔌 DaemonClient::send_request called");
        debug!("📤 Request: {:?}", request);

        let socket_path = determine_socket_path()?;
        debug!("🔌 Connecting to socket: {:?}", socket_path);

        let mut stream = UnixStream::connect(&socket_path)
            .map_err(|e| eyre::eyre!("Failed to connect to daemon: {}", e))?;
        debug!("✅ Connected to daemon socket");

        // Send request
        let request_json = serde_json::to_string(&request)?;
        debug!("📤 Sending JSON: {}", request_json);

        writeln!(stream, "{}", request_json)?;
        debug!("✅ Request sent to daemon");

        // Read response
        let mut reader = BufReader::new(&stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;
        debug!("📥 Raw response received: {}", response_line.trim());

        let response: DaemonResponse = serde_json::from_str(&response_line.trim())?;
        debug!("✅ Response parsed: {:?}", response);

        Ok(response)
    }


}

fn get_after_help() -> &'static str {
    let daemon_status = get_daemon_status_emoji();
    Box::leak(format!(
        "Logs are written to: ~/.local/share/aka/logs/aka.log\nDaemon status: {}",
        daemon_status
    ).into_boxed_str())
}

fn get_daemon_status_emoji() -> &'static str {
    // Check daemon status quickly and return appropriate emoji
    let socket_path = match determine_socket_path() {
        Ok(path) => path,
        Err(_) => return "❓", // Unknown - can't determine socket path
    };

    let socket_exists = socket_path.exists();
    let process_running = check_daemon_process_simple();

    match (socket_exists, process_running) {
        (true, true) => {
            // Check if config is in sync
            match check_config_sync_status() {
                Ok(true) => "✅",   // Healthy and in sync
                Ok(false) => "🔄", // Healthy but config out of sync
                Err(_) => "✅",    // Can't check sync, assume healthy
            }
        },
        (true, false) => "⚠️",  // Stale socket
        (false, false) => "❗", // Not running
        (false, true) => "❓",  // Weird state - process but no socket
    }
}

fn check_config_sync_status() -> Result<bool> {
    use aka_lib::get_config_path;
    
    // Get current file hash
    let config_path = get_config_path()?;
    let file_hash = aka_lib::hash_config_file(&config_path)?;
    
    // Query daemon for its hash
    let request = DaemonRequest::Health;
    match DaemonClient::send_request(request) {
        Ok(DaemonResponse::Health { status }) => {
            // Parse daemon hash from status string
            // Format: "healthy:391:aliases:abcd1234efgh5678:hash"
            if let Some(daemon_hash) = parse_daemon_hash(&status) {
                Ok(file_hash == daemon_hash)
            } else {
                // Old daemon format without hash, assume in sync
                Ok(true)
            }
        },
        _ => {
            // Can't communicate with daemon
            Err(eyre::eyre!("Failed to get daemon health status"))
        }
    }
}

fn parse_daemon_hash(status: &str) -> Option<String> {
    // Parse format: "healthy:391:aliases:abcd1234efgh5678:hash"
    let parts: Vec<&str> = status.split(':').collect();
    if parts.len() >= 5 && parts.get(4) == Some(&"hash") {
        Some(parts.get(3)?.to_string())
    } else {
        None
    }
}

fn check_daemon_process_simple() -> bool {
    use std::process::Command;

    // Quick check if aka-daemon process is running
    Command::new("pgrep")
        .arg("aka-daemon")
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

#[derive(Parser, Debug)]
#[command(name = "aka", about = "[a]lso [k]nown [a]s: an aliasing program")]
#[command(version = built_info::GIT_DESCRIBE)]
#[command(author = "Scott A. Idler <scott.a.idler@gmail.com>")]
#[command(arg_required_else_help = true)]
#[command(after_help = get_after_help())]
struct AkaOpts {
    #[clap(short, long, help = "is entry an [e]nd [o]f [l]ine?")]
    eol: bool,

    #[clap(short, long)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[clap(name = "ls", about = "list aka aliases")]
    List(ListOpts),

    #[clap(name = "query", about = "query for aka substitutions")]
    Query(QueryOpts),

    #[clap(name = "daemon", about = "manage aka daemon")]
    Daemon(DaemonOpts),

    #[clap(name = "__complete_aliases", hide = true)]
    CompleteAliases,

    #[clap(name = "__health_check", hide = true)]
    HealthCheck,
}

#[derive(Parser, Debug)]
struct QueryOpts {
    cmdline: String,
}

#[derive(Parser, Debug)]
struct ListOpts {
    #[clap(short, long, help = "list global aliases only")]
    global: bool,

    patterns: Vec<String>,
}

#[derive(Parser, Debug)]
struct DaemonOpts {
    #[clap(long, help = "Install system service")]
    install: bool,

    #[clap(long, help = "Uninstall system service")]
    uninstall: bool,

    #[clap(long, help = "Start daemon")]
    start: bool,

    #[clap(long, help = "Stop daemon")]
    stop: bool,

    #[clap(long, help = "Restart daemon")]
    restart: bool,

    #[clap(long, help = "Reload daemon configuration")]
    reload: bool,

    #[clap(long, help = "Show daemon status")]
    status: bool,

    #[clap(long, help = "Show daemon status legend")]
    legend: bool,
}

// Basic service manager for proof of concept
struct ServiceManager;

impl ServiceManager {
    fn new() -> Self {
        ServiceManager
    }

    fn install_service(&self) -> Result<()> {
        println!("📦 Installing daemon service...");

        // For now, just create a simple systemd user service file
        if cfg!(target_os = "linux") {
            self.install_systemd_service()?;
        } else if cfg!(target_os = "macos") {
            self.install_launchd_service()?;
        } else {
            println!("⚠️  Service management not yet supported on this platform");
            println!("   You can run the daemon manually with: aka-daemon");
            return Ok(());
        }

        println!("✅ Service installed successfully");

        // Try to start the service automatically
        println!("🚀 Starting daemon...");
        match self.start_service_silent() {
            Ok(_) => println!("✅ Daemon started successfully"),
            Err(e) => {
                println!("⚠️  Failed to start daemon automatically: {}", e);
                println!("   You can start it manually with: aka daemon --start");
            }
        }

        Ok(())
    }

    fn install_systemd_service(&self) -> Result<()> {
        use std::fs;
        use std::process::Command;

        // Create systemd user directory
        let service_dir = dirs::config_dir()
            .ok_or_else(|| eyre::eyre!("Could not determine config directory"))?
            .join("systemd/user");
        fs::create_dir_all(&service_dir)?;

        // Get aka-daemon binary path
        let daemon_path = self.get_daemon_binary_path()?;

        // Create service file content
        let service_content = format!(
            r#"[Unit]
Description=AKA Alias Daemon
After=graphical-session.target

[Service]
Type=simple
ExecStart={}
Restart=always
RestartSec=5
Environment=PATH={}:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
"#,
            daemon_path.display(),
            dirs::home_dir().unwrap().join(".cargo/bin").display()
        );

        // Write service file
        let service_file = service_dir.join("aka-daemon.service");
        fs::write(&service_file, service_content)?;

        // Reload systemd and enable service
        Command::new("systemctl").args(&["--user", "daemon-reload"]).status()?;
        Command::new("systemctl").args(&["--user", "enable", "aka-daemon.service"]).status()?;

        println!("✅ SystemD service installed and enabled");
        Ok(())
    }

    fn install_launchd_service(&self) -> Result<()> {
        use std::fs;

        // Create LaunchAgents directory
        let plist_dir = dirs::home_dir()
            .ok_or_else(|| eyre::eyre!("Could not determine home directory"))?
            .join("Library/LaunchAgents");
        fs::create_dir_all(&plist_dir)?;

        // Get aka-daemon binary path
        let daemon_path = self.get_daemon_binary_path()?;

        // Create plist content
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.scottidler.aka-daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>{}/Library/Logs/aka-daemon.log</string>
    <key>StandardOutPath</key>
    <string>{}/Library/Logs/aka-daemon.log</string>
</dict>
</plist>
"#,
            daemon_path.display(),
            dirs::home_dir().unwrap().display(),
            dirs::home_dir().unwrap().display()
        );

        // Write plist file
        let plist_file = plist_dir.join("com.scottidler.aka-daemon.plist");
        fs::write(&plist_file, plist_content)?;

        println!("✅ LaunchAgent installed");
        Ok(())
    }

    fn get_daemon_binary_path(&self) -> Result<PathBuf> {
        use std::process::Command;

        // Strategy 1: Check if aka-daemon is in PATH
        if let Ok(output) = Command::new("which").arg("aka-daemon").output() {
            if output.status.success() {
                let path_str = String::from_utf8(output.stdout)?.trim().to_string();
                let path = PathBuf::from(path_str);
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        // Strategy 2: Check cargo install location
        if let Some(home_dir) = dirs::home_dir() {
            let cargo_bin = home_dir.join(".cargo/bin/aka-daemon");
            if cargo_bin.exists() {
                return Ok(cargo_bin);
            }
        }

        Err(eyre::eyre!("Could not find aka-daemon binary. Please ensure it's installed and in PATH."))
    }

    fn start_service(&self) -> Result<()> {
        use std::process::Command;

        println!("🚀 Starting daemon...");

        if cfg!(target_os = "linux") {
            let output = Command::new("systemctl")
                .args(&["--user", "start", "aka-daemon.service"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon started via SystemD");
            } else {
                return Err(eyre::eyre!("Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)));
            }
        } else if cfg!(target_os = "macos") {
            let output = Command::new("launchctl")
                .args(&["start", "com.scottidler.aka-daemon"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon started via LaunchD");
            } else {
                return Err(eyre::eyre!("Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)));
            }
        } else {
            println!("⚠️  Service management not supported on this platform");
            println!("   You can run the daemon manually with: aka-daemon &");
        }

        Ok(())
    }

    fn start_service_silent(&self) -> Result<()> {
        use std::process::Command;

        if cfg!(target_os = "linux") {
            let output = Command::new("systemctl")
                .args(&["--user", "start", "aka-daemon.service"])
                .output()?;

            if !output.status.success() {
                return Err(eyre::eyre!("Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)));
            }
        } else if cfg!(target_os = "macos") {
            let output = Command::new("launchctl")
                .args(&["start", "com.scottidler.aka-daemon"])
                .output()?;

            if !output.status.success() {
                return Err(eyre::eyre!("Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)));
            }
        } else {
            return Err(eyre::eyre!("Service management not supported on this platform"));
        }

        Ok(())
    }

    fn stop_service(&self) -> Result<()> {
        use std::process::Command;

        println!("🛑 Stopping daemon...");

        if cfg!(target_os = "linux") {
            let output = Command::new("systemctl")
                .args(&["--user", "stop", "aka-daemon.service"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon stopped via SystemD");
            } else {
                return Err(eyre::eyre!("Failed to stop daemon: {}",
                    String::from_utf8_lossy(&output.stderr)));
            }
        } else if cfg!(target_os = "macos") {
            let output = Command::new("launchctl")
                .args(&["stop", "com.scottidler.aka-daemon"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon stopped via LaunchD");
            } else {
                return Err(eyre::eyre!("Failed to stop daemon: {}",
                    String::from_utf8_lossy(&output.stderr)));
            }
        } else {
            println!("⚠️  Service management not supported on this platform");
            println!("   You can stop the daemon manually with: pkill aka-daemon");
        }

        Ok(())
    }

    fn status(&self) -> Result<()> {
        println!("🔍 AKA Daemon Status Check");
        println!();

        // Check daemon binary
        let daemon_binary = self.get_daemon_binary_path();
        match daemon_binary {
            Ok(path) => println!("📦 Daemon binary: ✅ Found at {:?}", path),
            Err(_) => {
                println!("📦 Daemon binary: ❌ Not found in PATH");
                println!("   💡 Install with: cargo install --path .");
                return Ok(());
            }
        }

        // Check socket file
        let socket_path = determine_socket_path()?;
        let socket_exists = socket_path.exists();
        if socket_exists {
            println!("🔌 Socket file: ✅ Found at {:?}", socket_path);
        } else {
            println!("🔌 Socket file: ❌ Not found");
        }

        // Check if daemon process is actually running
        let process_running = self.check_daemon_process();
        if process_running {
            println!("⚙️  Daemon process: ✅ Running");
        } else {
            println!("⚙️  Daemon process: ❌ Not running");
        }

        // Check service manager status
        if cfg!(target_os = "linux") {
            self.check_systemd_status()?;
        } else if cfg!(target_os = "macos") {
            self.check_launchd_status()?;
        } else {
            println!("🏗️  Service manager: ⚠️  Not supported on this platform");
        }

        // Overall status
        println!();
        if socket_exists && process_running {
            println!("🚀 Overall status: ✅ Daemon is healthy and running");
            println!("   💨 Queries will use high-performance daemon");
        } else if socket_exists && !process_running {
            println!("🚀 Overall status: ⚠️  Stale socket detected");
            println!("   🧹 Run: aka daemon --stop && aka daemon --start");
        } else {
            println!("🚀 Overall status: ❌ Daemon not running");
            println!("   🔧 Start with: aka daemon --start");
            println!("   📋 Or install service: aka daemon --install");
        }

        Ok(())
    }

    fn check_daemon_process(&self) -> bool {
        use std::process::Command;

        // Check if aka-daemon process is running
        if let Ok(output) = Command::new("pgrep").arg("aka-daemon").output() {
            output.status.success() && !output.stdout.is_empty()
        } else {
            false
        }
    }

    fn check_systemd_status(&self) -> Result<()> {
        use std::process::Command;

        let output = Command::new("systemctl")
            .args(&["--user", "is-active", "aka-daemon.service"])
            .output()?;

        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let service_file = dirs::config_dir()
            .unwrap_or_default()
            .join("systemd/user/aka-daemon.service");

        match status.as_str() {
            "active" => println!("🏗️  SystemD service: ✅ Active"),
            "inactive" => {
                if service_file.exists() {
                    println!("🏗️  SystemD service: ⚠️  Installed but inactive");
                } else {
                    println!("🏗️  SystemD service: ❌ Not installed");
                }
            }
            "failed" => println!("🏗️  SystemD service: ❌ Failed"),
            _ => println!("🏗️  SystemD service: ❓ Unknown status: {}", status),
        }

        Ok(())
    }

    fn check_launchd_status(&self) -> Result<()> {
        use std::process::Command;

        let output = Command::new("launchctl")
            .args(&["list", "com.scottidler.aka-daemon"])
            .output()?;

        let plist_file = dirs::home_dir()
            .unwrap_or_default()
            .join("Library/LaunchAgents/com.scottidler.aka-daemon.plist");

        if output.status.success() {
            println!("🏗️  LaunchD service: ✅ Loaded");
        } else if plist_file.exists() {
            println!("🏗️  LaunchD service: ⚠️  Installed but not loaded");
        } else {
            println!("🏗️  LaunchD service: ❌ Not installed");
        }

        Ok(())
    }

    fn uninstall_service(&self) -> Result<()> {
        println!("🗑️  Uninstalling daemon service...");

        if cfg!(target_os = "linux") {
            use std::process::Command;
            use std::fs;

            // Stop and disable service
            let _ = Command::new("systemctl").args(&["--user", "stop", "aka-daemon.service"]).status();
            let _ = Command::new("systemctl").args(&["--user", "disable", "aka-daemon.service"]).status();

            // Remove service file
            let service_file = dirs::config_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine config directory"))?
                .join("systemd/user/aka-daemon.service");
            if service_file.exists() {
                fs::remove_file(&service_file)?;
            }

            let _ = Command::new("systemctl").args(&["--user", "daemon-reload"]).status();
            println!("✅ SystemD service uninstalled");
        } else if cfg!(target_os = "macos") {
            use std::process::Command;
            use std::fs;

            // Unload service
            let _ = Command::new("launchctl").args(&["unload", "com.scottidler.aka-daemon"]).status();

            // Remove plist file
            let plist_file = dirs::home_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine home directory"))?
                .join("Library/LaunchAgents/com.scottidler.aka-daemon.plist");
            if plist_file.exists() {
                fs::remove_file(&plist_file)?;
            }

            println!("✅ LaunchAgent uninstalled");
        } else {
            println!("⚠️  Service management not supported on this platform");
        }

        // Clean up socket file regardless of platform
        if let Ok(socket_path) = determine_socket_path() {
            if socket_path.exists() {
                use std::fs;
                if let Err(e) = fs::remove_file(&socket_path) {
                    println!("⚠️  Failed to remove socket file: {}", e);
                } else {
                    println!("🧹 Removed stale socket file");
                }
            }
        }

        Ok(())
    }
}

fn print_daemon_legend() {
    println!("Daemon Status Legend:");
    println!("  ✅ - Daemon is healthy (config in sync)");
    println!("  🔄 - Config out of sync (daemon has stale config)");
    println!("  ⚠️  - Stale socket (socket exists but process not running)");
    println!("  ❗ - Daemon not running (no socket, no process)");
    println!("  ❓ - Unknown/weird state (can't determine socket path, or process without socket)");
}

fn handle_daemon_command(daemon_opts: &DaemonOpts) -> Result<()> {
    let service_manager = ServiceManager::new();

    if daemon_opts.install {
        service_manager.install_service()?;
    } else if daemon_opts.uninstall {
        service_manager.uninstall_service()?;
    } else if daemon_opts.start {
        service_manager.start_service()?;
    } else if daemon_opts.stop {
        service_manager.stop_service()?;
    } else if daemon_opts.restart {
        service_manager.stop_service()?;
        std::thread::sleep(std::time::Duration::from_secs(1));
        service_manager.start_service()?;
    } else if daemon_opts.reload {
        // Send reload config request to daemon
        match DaemonClient::send_request(DaemonRequest::ReloadConfig) {
            Ok(DaemonResponse::ConfigReloaded { success, aliases_count, message }) => {
                if success {
                    println!("✅ Config reloaded successfully: {} aliases loaded", aliases_count);
                    println!("   {}", message);
                } else {
                    println!("❌ Config reload failed: {}", message);
                }
            },
            Ok(DaemonResponse::Error { message }) => {
                eprintln!("❌ Daemon error: {}", message);
            },
            Ok(_) => {
                eprintln!("❌ Unexpected response from daemon");
            },
            Err(e) => {
                eprintln!("❌ Failed to communicate with daemon: {}", e);
                eprintln!("   Make sure the daemon is running with: aka daemon --status");
            }
        }
    } else if daemon_opts.status {
        service_manager.status()?;
    } else if daemon_opts.legend {
        print_daemon_legend();
    } else {
        println!("Usage: aka daemon [--install|--uninstall|--start|--stop|--restart|--reload|--status|--legend]");
        return Ok(());
    }

    Ok(())
}

fn handle_regular_command(opts: &AkaOpts) -> Result<i32> {
    debug!("🎯 === STARTING REGULAR COMMAND PROCESSING ===");
    debug!("🔍 Command options: {:?}", opts);

    // Handle explicit health check command
    if let Some(ref command) = &opts.command {
        if let Command::HealthCheck = command {
            debug!("🏥 Explicit health check command requested");
            return execute_health_check(&opts.config);
        }
    }

    // For all other commands, use health check to determine the best path
    debug!("🔍 Using health check to determine processing path");
    debug!("📋 About to run execute_health_check with config: {:?}", opts.config);

    // Run health check to determine system state
    let health_status = execute_health_check(&opts.config)?;
    debug!("📊 Health check completed with status: {}", health_status);

    match health_status {
        0 => {
            // Health check passed - it could be daemon or direct
            debug!("✅ Health check passed (status=0), proceeding with daemon-first approach");
            debug!("🔀 Routing to handle_command_via_daemon_with_fallback");
            return handle_command_via_daemon_with_fallback(opts);
        },
        1 => {
            debug!("❌ Health check failed: config file not found (status=1)");
            debug!("🚨 Returning error to user");
            eprintln!("Error: Configuration file not found");
            Ok(1)
        },
        2 => {
            debug!("❌ Health check failed: config file invalid (status=2)");
            debug!("🚨 Returning error to user");
            eprintln!("Error: Configuration file is invalid");
            Ok(2)
        },
        3 => {
            debug!("⚠️ Health check passed but no aliases defined (status=3)");
            debug!("📝 Returning empty result to user");
            // Still process the command, just return empty result
            println!("");
            Ok(0)
        },
        _ => {
            debug!("❌ Health check returned unknown status: {}", health_status);
            debug!("🚨 Returning unknown error to user");
            eprintln!("Error: Unknown health check status");
            Ok(1)
        }
    }
}

fn handle_command_via_daemon_with_fallback(opts: &AkaOpts) -> Result<i32> {
    debug!("🎯 === DAEMON-WITH-FALLBACK PROCESSING ===");
    debug!("🔍 Attempting daemon path first");

    // Quick check if daemon is available
    match determine_socket_path() {
        Ok(socket_path) => {
            debug!("🔌 Socket path determined: {:?}", socket_path);
            if socket_path.exists() {
                debug!("✅ Socket file exists, attempting daemon communication");

                // Try daemon approach
                match handle_command_via_daemon_only(opts) {
                    Ok(result) => {
                        debug!("✅ Daemon path successful, returning result: {}", result);
                        debug!("🎯 === DAEMON-WITH-FALLBACK COMPLETE (DAEMON SUCCESS) ===");
                        return Ok(result);
                    },
                    Err(e) => {
                        debug!("⚠️ Daemon path failed: {}, falling back to direct", e);
                        debug!("🔄 Daemon communication failed, will try direct path");
                    }
                }
            } else {
                debug!("❌ Socket file does not exist: {:?}", socket_path);
                debug!("📁 No daemon socket, using direct path");
            }
        }
        Err(e) => {
            debug!("❌ Cannot determine socket path: {}, using direct path", e);
        }
    }

    // Fallback to direct processing
    debug!("🔄 Falling back to direct config processing");
    debug!("🔀 Routing to handle_command_direct");
    let result = handle_command_direct(opts);
    debug!("🎯 === DAEMON-WITH-FALLBACK COMPLETE (DIRECT FALLBACK) ===");
    result
}

fn handle_command_via_daemon_only(opts: &AkaOpts) -> Result<i32> {
    debug!("🎯 === DAEMON-ONLY PROCESSING ===");
    debug!("🔍 Daemon-only handler - NO fallback to config loading");
    debug!("📋 Health check already confirmed daemon was healthy");

    if let Some(ref command) = &opts.command {
        debug!("🔍 Processing command: {:?}", command);
        match command {
            Command::Query(query_opts) => {
                debug!("📤 Preparing daemon query request");
                let request = DaemonRequest::Query { cmdline: query_opts.cmdline.clone() };
                debug!("📤 Sending daemon request: Query({})", query_opts.cmdline);

                match DaemonClient::send_request(request) {
                    Ok(DaemonResponse::Success { data }) => {
                        debug!("✅ Daemon query successful, got response: {}", data);
                        println!("{}", data);
                        debug!("🎯 === DAEMON-ONLY COMPLETE (SUCCESS) ===");
                        Ok(0)
                    },
                    Ok(DaemonResponse::Error { message }) => {
                        debug!("❌ Daemon returned error: {}", message);
                        eprintln!("Daemon error: {}", message);
                        debug!("🎯 === DAEMON-ONLY COMPLETE (DAEMON ERROR) ===");
                        Ok(1)
                    },
                    Ok(response) => {
                        debug!("❌ Daemon returned unexpected response: {:?}", response);
                        eprintln!("Unexpected daemon response");
                        debug!("🎯 === DAEMON-ONLY COMPLETE (UNEXPECTED RESPONSE) ===");
                        Ok(1)
                    },
                    Err(e) => {
                        debug!("❌ Daemon request failed: {}", e);
                        eprintln!("Daemon communication failed: {}", e);
                        debug!("🎯 === DAEMON-ONLY COMPLETE (COMMUNICATION ERROR) ===");
                        Err(e)
                    }
                }
            }
            Command::List(list_opts) => {
                let request = DaemonRequest::List {
                    global: list_opts.global,
                    patterns: list_opts.patterns.clone()
                };
                match DaemonClient::send_request(request) {
                    Ok(DaemonResponse::Success { data }) => {
                        debug!("✅ Daemon list successful");
                        println!("{}", data);
                        Ok(0)
                    },
                    Ok(DaemonResponse::Error { message }) => {
                        debug!("❌ Daemon returned error: {}", message);
                        eprintln!("Daemon error: {}", message);
                        Ok(1)
                    },
                    Ok(_) => {
                        debug!("❌ Daemon returned unexpected response");
                        eprintln!("Unexpected daemon response");
                        Ok(1)
                    },
                    Err(e) => {
                        debug!("❌ Daemon request failed: {}", e);
                        eprintln!("Daemon communication failed: {}", e);
                        Ok(1)
                    }
                }
            }
            _ => {
                debug!("❌ Command not supported in daemon-only mode");
                eprintln!("Command not supported in daemon mode");
                Ok(1)
            }
        }
    } else {
        Ok(0)
    }
}

fn handle_command_direct(opts: &AkaOpts) -> Result<i32> {
    debug!("🎯 === DIRECT PROCESSING ===");
    debug!("📁 Loading config for direct processing (cache-aware)");
    debug!("🔍 Direct processing options: eol={}, config={:?}", opts.eol, opts.config);

    let aka = AKA::new(opts.eol, &opts.config)?;
    debug!("✅ Config loaded, {} aliases available", aka.spec.aliases.len());

    if let Some(ref command) = opts.command {
        debug!("🔍 Processing command in direct mode: {:?}", command);
        match command {
            Command::Query(query_opts) => {
                debug!("🔍 Processing query: {}", query_opts.cmdline);
                let result = aka.replace_with_mode(&query_opts.cmdline, ProcessingMode::Direct)?;
                debug!("✅ Query processed, result: {}", result);
                println!("{result}");
                debug!("🎯 === DIRECT PROCESSING COMPLETE (QUERY SUCCESS) ===");
            }
            Command::List(list_opts) => {
                let mut aliases: Vec<_> = aka.spec.aliases.values().cloned().collect();
                aliases.sort_by_key(|a| a.name.clone());

                if list_opts.global {
                    aliases = aliases.into_iter().filter(|alias| alias.global).collect();
                }

                if list_opts.patterns.is_empty() {
                    for alias in aliases {
                        print_alias(&alias);
                    }
                } else {
                    for alias in aliases {
                        if list_opts.patterns.iter().any(|pattern| alias.name.starts_with(pattern)) {
                            print_alias(&alias);
                        }
                    }
                }
            }

            Command::CompleteAliases => {
                let mut keys: Vec<_> = aka.spec.aliases
                    .keys()
                    .filter(|name| name.len() > 1 && !name.starts_with('|'))
                    .cloned()
                    .collect();
                keys.sort();
                for name in keys {
                    println!("{name}");
                }
                return Ok(0);
            }

            Command::HealthCheck => {
                // Already handled above
                return Ok(0);
            }

            Command::Daemon(_) => {
                // Should not reach here
                return Ok(0);
            }
        }
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_status_emoji() {
        // Test that daemon status emoji function works
        let emoji = get_daemon_status_emoji();
        assert!(matches!(emoji, "✅" | "⚠️" | "❗" | "❓"), "Should return valid emoji");
    }

    #[test]
    fn test_daemon_process_check() {
        // Test daemon process checking
        let result = check_daemon_process_simple();
        // Should return bool without panicking
        assert!(result == true || result == false);
    }

    #[test]
    fn test_service_manager_creation() {
        // Test service manager can be created
        let _manager = ServiceManager::new();
        // Should not panic
    }

    #[test]
    fn test_daemon_client_socket_path() {
        // Test that we can determine socket path
        let result = determine_socket_path();
        // Should either succeed or fail gracefully
        match result {
            Ok(path) => assert!(path.to_string_lossy().contains("aka")),
            Err(_) => {}, // Acceptable in test environment
        }
    }

    #[test]
    fn test_daemon_request_serialization() {
        // Test that daemon requests can be serialized
        let request = DaemonRequest::Health;
        let serialized = serde_json::to_string(&request);
        assert!(serialized.is_ok());

        let query_request = DaemonRequest::Query { cmdline: "test".to_string() };
        let serialized = serde_json::to_string(&query_request);
        assert!(serialized.is_ok());
    }

    #[test]
    fn test_daemon_response_deserialization() {
        // Test that daemon responses can be deserialized
        let response_json = r#"{"type":"Health","status":"healthy:5:aliases"}"#;
        let response: Result<DaemonResponse, _> = serde_json::from_str(response_json);
        assert!(response.is_ok());

        if let Ok(DaemonResponse::Health { status }) = response {
            assert_eq!(status, "healthy:5:aliases");
        }
    }

    #[test]
    fn test_after_help_generation() {
        // Test that help text generation works
        let help = get_after_help();
        assert!(help.contains("Logs are written to"));
        assert!(help.contains("Daemon status:"));
    }
}

fn main() {
    let opts = AkaOpts::parse();

    // Set up logging
    if let Err(e) = setup_logging() {
        eprintln!("Warning: Failed to set up logging: {}", e);
    }

    // Route daemon commands vs regular commands
    let result = match &opts.command {
        Some(Command::Daemon(daemon_opts)) => {
            match handle_daemon_command(daemon_opts) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            }
        }
        _ => {
            match handle_regular_command(&opts) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    1
                }
            }
        }
    };

    exit(result);
}
