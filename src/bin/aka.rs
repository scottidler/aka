use clap::{Parser, Subcommand};
use eyre::Result;
use log::{debug, info, warn};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::exit;
use std::time::{Duration, Instant};

// Import from the shared library
use aka_lib::{
    determine_socket_path, execute_health_check, export_timing_csv, get_config_path_with_override, get_timing_summary,
    log_timing, setup_logging, DaemonRequest, DaemonResponse, ProcessingMode, TimingCollector, AKA,
};

// Version constant for compatibility checking
const CLI_VERSION: &str = env!("GIT_DESCRIBE");

// Daemon client constants and types - moved from shared library
const DAEMON_CONNECTION_TIMEOUT_MS: u64 = 100; // 100ms to connect
const DAEMON_READ_TIMEOUT_MS: u64 = 200; // 200ms to read response
const DAEMON_WRITE_TIMEOUT_MS: u64 = 50; // 50ms to write request
const DAEMON_TOTAL_TIMEOUT_MS: u64 = 300; // 300ms total operation limit
const DAEMON_RETRY_DELAY_MS: u64 = 50; // 50ms between retries
const DAEMON_MAX_RETRIES: u32 = 1; // Only 1 retry attempt

#[derive(Debug, Clone)]
enum DaemonError {
    ConnectionTimeout,
    ReadTimeout,
    WriteTimeout,
    ConnectionRefused,
    SocketNotFound,
    SocketPermissionDenied,
    ProtocolError(String),
    DaemonShutdown,
    TotalOperationTimeout,
    UnknownError(String),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::ConnectionTimeout => write!(f, "Daemon connection timeout"),
            DaemonError::ReadTimeout => write!(f, "Daemon read timeout"),
            DaemonError::WriteTimeout => write!(f, "Daemon write timeout"),
            DaemonError::ConnectionRefused => write!(f, "Daemon connection refused"),
            DaemonError::SocketNotFound => write!(f, "Daemon socket not found"),
            DaemonError::SocketPermissionDenied => write!(f, "Daemon socket permission denied"),
            DaemonError::ProtocolError(msg) => write!(f, "Daemon protocol error: {msg}"),
            DaemonError::DaemonShutdown => write!(f, "Daemon is shutting down"),
            DaemonError::TotalOperationTimeout => write!(f, "Total daemon operation timeout"),
            DaemonError::UnknownError(msg) => write!(f, "Unknown daemon error: {msg}"),
        }
    }
}

impl std::error::Error for DaemonError {}

fn should_retry_daemon_error(error: &DaemonError) -> bool {
    match error {
        DaemonError::ConnectionTimeout => true,
        DaemonError::ConnectionRefused => true,
        DaemonError::ReadTimeout => false,  // Don't retry read timeouts
        DaemonError::WriteTimeout => false, // Don't retry write timeouts
        DaemonError::SocketNotFound => false,
        DaemonError::SocketPermissionDenied => false,
        DaemonError::ProtocolError(_) => false,
        DaemonError::DaemonShutdown => false,
        DaemonError::TotalOperationTimeout => false,
        DaemonError::UnknownError(_) => false,
    }
}

fn categorize_daemon_error(error: &std::io::Error) -> DaemonError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::TimedOut => DaemonError::ConnectionTimeout,
        ErrorKind::ConnectionRefused => DaemonError::ConnectionRefused,
        ErrorKind::NotFound => DaemonError::SocketNotFound,
        ErrorKind::PermissionDenied => DaemonError::SocketPermissionDenied,
        ErrorKind::WouldBlock => DaemonError::ReadTimeout,
        _ => DaemonError::UnknownError(error.to_string()),
    }
}

fn validate_socket_path(socket_path: &PathBuf) -> Result<(), DaemonError> {
    if !socket_path.exists() {
        return Err(DaemonError::SocketNotFound);
    }

    // Check if it's actually a socket (not a regular file)
    match std::fs::metadata(socket_path) {
        Ok(metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                if !metadata.file_type().is_socket() {
                    return Err(DaemonError::SocketNotFound);
                }
            }
        }
        Err(e) => {
            return Err(categorize_daemon_error(&e));
        }
    }

    Ok(())
}

// Daemon client for sending requests
struct DaemonClient;

impl DaemonClient {
    fn send_request(request: DaemonRequest) -> Result<DaemonResponse> {
        let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory"))?;
        let socket_path = determine_socket_path(&home_dir)?;

        Self::send_request_with_timeout(request, &socket_path).map_err(|e| e.into())
    }

    fn send_request_with_timeout(request: DaemonRequest, socket_path: &PathBuf) -> Result<DaemonResponse, DaemonError> {
        let operation_start = Instant::now();
        let total_timeout = Duration::from_millis(DAEMON_TOTAL_TIMEOUT_MS);

        // Pre-validate socket before attempting connection
        validate_socket_path(socket_path)?;

        let mut last_error = None;

        for attempt in 0..=DAEMON_MAX_RETRIES {
            // Check total operation timeout
            if operation_start.elapsed() >= total_timeout {
                debug!(
                    "🚨 Total daemon operation timeout exceeded: {}ms",
                    operation_start.elapsed().as_millis()
                );
                return Err(DaemonError::TotalOperationTimeout);
            }

            if attempt > 0 {
                debug!(
                    "🔄 Daemon retry attempt {} after {}ms",
                    attempt,
                    operation_start.elapsed().as_millis()
                );
                std::thread::sleep(Duration::from_millis(DAEMON_RETRY_DELAY_MS));
            }

            match Self::attempt_single_request(&request, socket_path, &operation_start, &total_timeout) {
                Ok(response) => {
                    debug!(
                        "✅ Daemon request succeeded on attempt {} in {}ms",
                        attempt + 1,
                        operation_start.elapsed().as_millis()
                    );
                    return Ok(response);
                }
                Err(error) => {
                    debug!("❌ Daemon attempt {} failed: {}", attempt + 1, error);

                    // Check if we should retry this error type
                    if !should_retry_daemon_error(&error) || attempt >= DAEMON_MAX_RETRIES {
                        return Err(error);
                    }

                    last_error = Some(error);
                }
            }
        }

        // If we get here, all retries failed
        Err(last_error.unwrap_or(DaemonError::UnknownError("All retry attempts failed".to_string())))
    }

    fn attempt_single_request(
        request: &DaemonRequest,
        socket_path: &PathBuf,
        operation_start: &Instant,
        total_timeout: &Duration,
    ) -> Result<DaemonResponse, DaemonError> {
        // Check timeout before connection
        if operation_start.elapsed() >= *total_timeout {
            return Err(DaemonError::TotalOperationTimeout);
        }

        debug!("📡 Connecting to daemon at: {socket_path:?}");

        // Connect with timeout
        let mut stream = Self::connect_with_timeout(socket_path)?;

        // Check timeout after connection
        if operation_start.elapsed() >= *total_timeout {
            return Err(DaemonError::TotalOperationTimeout);
        }

        // Set socket timeouts
        stream
            .set_read_timeout(Some(Duration::from_millis(DAEMON_READ_TIMEOUT_MS)))
            .map_err(|e| categorize_daemon_error(&e))?;
        stream
            .set_write_timeout(Some(Duration::from_millis(DAEMON_WRITE_TIMEOUT_MS)))
            .map_err(|e| categorize_daemon_error(&e))?;

        // Send request
        let request_json = serde_json::to_string(&request)
            .map_err(|e| DaemonError::ProtocolError(format!("Failed to serialize request: {e}")))?;

        debug!("📤 Sending request: {request_json}");
        writeln!(stream, "{request_json}").map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                DaemonError::WriteTimeout
            } else {
                categorize_daemon_error(&e)
            }
        })?;

        // Check timeout after write
        if operation_start.elapsed() >= *total_timeout {
            return Err(DaemonError::TotalOperationTimeout);
        }

        // Read response
        let mut reader = BufReader::new(&stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                DaemonError::ReadTimeout
            } else {
                categorize_daemon_error(&e)
            }
        })?;

        debug!("📥 Received response: {}", response_line.trim());

        // Validate response size
        if let Err(e) = aka_lib::protocol::validate_message_size(&response_line) {
            return Err(DaemonError::ProtocolError(format!("Response validation failed: {e}")));
        }

        // Parse response
        let response: DaemonResponse = serde_json::from_str(response_line.trim())
            .map_err(|e| DaemonError::ProtocolError(format!("Failed to parse response: {e}")))?;

        // Check for daemon shutdown response
        if let DaemonResponse::ShutdownAck = response {
            return Err(DaemonError::DaemonShutdown);
        }

        Ok(response)
    }

    fn connect_with_timeout(socket_path: &PathBuf) -> Result<UnixStream, DaemonError> {
        // Unix sockets don't have built-in connect timeout, so we simulate it
        // by attempting connection in a non-blocking way
        let start = Instant::now();
        let timeout = Duration::from_millis(DAEMON_CONNECTION_TIMEOUT_MS);

        loop {
            match UnixStream::connect(socket_path) {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    if start.elapsed() >= timeout {
                        debug!("🚨 Connection timeout after {}ms", start.elapsed().as_millis());
                        return Err(DaemonError::ConnectionTimeout);
                    }

                    // For connection refused, fail immediately (don't wait for timeout)
                    if e.kind() == std::io::ErrorKind::ConnectionRefused {
                        return Err(DaemonError::ConnectionRefused);
                    }

                    // For other errors, categorize and return
                    if e.kind() != std::io::ErrorKind::WouldBlock {
                        return Err(categorize_daemon_error(&e));
                    }

                    // Brief sleep before retry
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    fn send_request_timed(request: DaemonRequest, timing: &mut TimingCollector) -> Result<DaemonResponse, DaemonError> {
        timing.start_ipc();
        let result = Self::send_request(request).map_err(|e| DaemonError::UnknownError(e.to_string()));
        timing.end_ipc();
        result
    }
}

fn get_after_help() -> &'static str {
    let daemon_status = get_daemon_status_emoji();
    Box::leak(
        format!("Logs are written to: ~/.local/share/aka/logs/aka.log\n\nDaemon status: {daemon_status}")
            .into_boxed_str(),
    )
}

fn get_daemon_status_emoji() -> &'static str {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    // Check daemon status quickly and return appropriate emoji
    let home_dir = match dirs::home_dir() {
        Some(dir) => dir,
        None => return "❓", // Unknown - can't determine home directory
    };
    let socket_path = match determine_socket_path(&home_dir) {
        Ok(path) => path,
        Err(_) => return "❓", // Unknown - can't determine socket path
    };

    let socket_exists = socket_path.exists();
    let process_running = check_daemon_process_simple();

    match (socket_exists, process_running) {
        (true, true) => {
            // Daemon appears to be running, check config sync status
            if let Ok(mut stream) = UnixStream::connect(&socket_path) {
                let health_request = r#"{"type":"Health"}"#;
                if writeln!(stream, "{health_request}").is_ok() {
                    let mut reader = BufReader::new(&stream);
                    let mut response_line = String::new();
                    if reader.read_line(&mut response_line).is_ok() {
                        if let Ok(response) = serde_json::from_str::<serde_json::Value>(response_line.trim()) {
                            if let Some(status) = response.get("status").and_then(|s| s.as_str()) {
                                if status.contains(":stale") {
                                    return "🔄"; // Config out of sync
                                } else if status.contains(":synced") {
                                    return "✅"; // Healthy and synced
                                }
                            }
                        }
                    }
                }
            }
            "⚠️" // Socket exists, process running, but health check failed
        }
        (true, false) => "⚠️",  // Stale socket
        (false, false) => "❗", // Not running
        (false, true) => "❓",  // Weird state - process but no socket
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
#[command(version = env!("GIT_DESCRIBE"))]
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

    #[clap(name = "freq", about = "show alias usage frequency statistics")]
    Freq(FreqOpts),

    #[clap(name = "daemon", about = "manage aka daemon")]
    Daemon(DaemonOpts),

    #[clap(name = "shell-init", about = "print shell initialization script")]
    ShellInit(ShellInitOpts),

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

    #[clap(long, help = "Reinstall system service (uninstall then install)")]
    reinstall: bool,

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

    #[clap(long, help = "Export timing data as CSV")]
    export_timing: bool,

    #[clap(long, help = "Show timing summary")]
    timing_summary: bool,
}

#[derive(Parser, Debug)]
struct FreqOpts {
    #[clap(short, long, help = "show all aliases including unused ones")]
    all: bool,
}

#[derive(Parser, Debug)]
struct ShellInitOpts {
    #[clap(default_value = "zsh", help = "Shell type (zsh)")]
    shell: String,
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
                println!("⚠️  Failed to start daemon automatically: {e}");
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
            dirs::home_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine home directory"))?
                .join(".cargo/bin")
                .display()
        );

        // Write service file
        let service_file = service_dir.join("aka-daemon.service");
        fs::write(&service_file, service_content)?;

        // Reload systemd and enable service
        Command::new("systemctl").args(["--user", "daemon-reload"]).status()?;
        Command::new("systemctl")
            .args(["--user", "enable", "aka-daemon.service"])
            .status()?;

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
            dirs::home_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine home directory"))?
                .display(),
            dirs::home_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine home directory"))?
                .display()
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

        Err(eyre::eyre!(
            "Could not find aka-daemon binary. Please ensure it's installed and in PATH."
        ))
    }

    fn start_service(&self) -> Result<()> {
        use std::process::Command;

        println!("🚀 Starting daemon...");

        if cfg!(target_os = "linux") {
            let output = Command::new("systemctl")
                .args(["--user", "start", "aka-daemon.service"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon started via SystemD");
            } else {
                return Err(eyre::eyre!(
                    "Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else if cfg!(target_os = "macos") {
            let output = Command::new("launchctl")
                .args(["start", "com.scottidler.aka-daemon"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon started via LaunchD");
            } else {
                return Err(eyre::eyre!(
                    "Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
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
                .args(["--user", "start", "aka-daemon.service"])
                .output()?;

            if !output.status.success() {
                return Err(eyre::eyre!(
                    "Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else if cfg!(target_os = "macos") {
            let output = Command::new("launchctl")
                .args(["start", "com.scottidler.aka-daemon"])
                .output()?;

            if !output.status.success() {
                return Err(eyre::eyre!(
                    "Failed to start daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
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
                .args(["--user", "stop", "aka-daemon.service"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon stopped via SystemD");
            } else {
                return Err(eyre::eyre!(
                    "Failed to stop daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else if cfg!(target_os = "macos") {
            let output = Command::new("launchctl")
                .args(["stop", "com.scottidler.aka-daemon"])
                .output()?;

            if output.status.success() {
                println!("✅ Daemon stopped via LaunchD");
            } else {
                return Err(eyre::eyre!(
                    "Failed to stop daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        } else {
            println!("⚠️  Service management not supported on this platform");
            println!("   You can stop the daemon manually with: pkill aka-daemon");
        }

        // Clean up socket file after stopping daemon
        if let Ok(home_dir) = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory")) {
            if let Ok(socket_path) = determine_socket_path(&home_dir) {
                if socket_path.exists() {
                    use std::fs;
                    // Give daemon a moment to clean up
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if socket_path.exists() {
                        if let Err(e) = fs::remove_file(&socket_path) {
                            println!("⚠️  Failed to remove socket file: {e}");
                        } else {
                            println!("🧹 Removed daemon socket file");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn status(&self) -> Result<()> {
        println!("🔍 AKA Daemon Status Check");
        println!();

        // Check daemon binary
        let daemon_binary = self.get_daemon_binary_path();
        match daemon_binary {
            Ok(path) => println!("📦 Daemon binary: ✅ Found at {path:?}"),
            Err(_) => {
                println!("📦 Daemon binary: ❌ Not found in PATH");
                println!("   💡 Install with: cargo install --path .");
                return Ok(());
            }
        }

        // Check socket file
        let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory"))?;
        let socket_path = determine_socket_path(&home_dir)?;
        let socket_exists = socket_path.exists();
        if socket_exists {
            println!("🔌 Socket file: ✅ Found at {socket_path:?}");
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
            .args(["--user", "is-active", "aka-daemon.service"])
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
            _ => println!("🏗️  SystemD service: ❓ Unknown status: {status}"),
        }

        Ok(())
    }

    fn check_launchd_status(&self) -> Result<()> {
        use std::process::Command;

        let output = Command::new("launchctl")
            .args(["list", "com.scottidler.aka-daemon"])
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
            use std::fs;
            use std::process::Command;

            // Stop and disable service
            let _ = Command::new("systemctl")
                .args(["--user", "stop", "aka-daemon.service"])
                .status();
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "aka-daemon.service"])
                .status();

            // Remove service file
            let service_file = dirs::config_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine config directory"))?
                .join("systemd/user/aka-daemon.service");
            if service_file.exists() {
                fs::remove_file(&service_file)?;
            }

            let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
            println!("✅ SystemD service uninstalled");
        } else if cfg!(target_os = "macos") {
            use std::fs;
            use std::process::Command;

            // Unload service
            let _ = Command::new("launchctl")
                .args(["unload", "com.scottidler.aka-daemon"])
                .status();

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
        if let Ok(home_dir) = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory")) {
            if let Ok(socket_path) = determine_socket_path(&home_dir) {
                if socket_path.exists() {
                    use std::fs;
                    if let Err(e) = fs::remove_file(&socket_path) {
                        println!("⚠️  Failed to remove socket file: {e}");
                    } else {
                        println!("🧹 Removed stale socket file");
                    }
                }
            }
        }

        Ok(())
    }
}

fn print_daemon_legend() {
    println!("Daemon Status Legend:");
    println!("  ✅ - Daemon is healthy and config is synced");
    println!("  🔄 - Daemon is healthy but config is out of sync (reload needed)");
    println!("  ⚠️  - Stale socket (socket exists but process not running)");
    println!("  ❗ - Daemon not running (no socket, no process)");
    println!("  ❓ - Unknown/weird state (can't determine socket path, or process without socket)");
}

fn handle_daemon_reload() -> Result<()> {
    println!("🔄 Reloading daemon configuration...");

    // Send reload request to daemon
    let request = DaemonRequest::ReloadConfig;
    match DaemonClient::send_request(request) {
        Ok(DaemonResponse::ConfigReloaded { success, message }) => {
            if success {
                println!("✅ {message}");
            } else {
                println!("❌ Config reload failed: {message}");
                return Err(eyre::eyre!("Config reload failed"));
            }
        }
        Ok(DaemonResponse::Error { message }) => {
            println!("❌ Daemon error: {message}");
            return Err(eyre::eyre!("Daemon error: {}", message));
        }
        Ok(response) => {
            println!("❌ Unexpected response: {response:?}");
            return Err(eyre::eyre!("Unexpected daemon response"));
        }
        Err(e) => {
            println!("❌ Failed to communicate with daemon: {e}");
            println!("   Make sure the daemon is running with: aka daemon --status");
            return Err(eyre::eyre!("Daemon communication failed: {}", e));
        }
    }

    Ok(())
}

fn handle_daemon_command(daemon_opts: &DaemonOpts) -> Result<()> {
    let service_manager = ServiceManager::new();

    if daemon_opts.install {
        service_manager.install_service()?;
    } else if daemon_opts.uninstall {
        service_manager.uninstall_service()?;
    } else if daemon_opts.reinstall {
        service_manager.uninstall_service()?;
        std::thread::sleep(std::time::Duration::from_secs(1));
        service_manager.install_service()?;
    } else if daemon_opts.start {
        service_manager.start_service()?;
    } else if daemon_opts.stop {
        service_manager.stop_service()?;
    } else if daemon_opts.restart {
        service_manager.stop_service()?;
        std::thread::sleep(std::time::Duration::from_secs(1));
        service_manager.start_service()?;
    } else if daemon_opts.reload {
        handle_daemon_reload()?;
    } else if daemon_opts.status {
        service_manager.status()?;
    } else if daemon_opts.legend {
        print_daemon_legend();
    } else if daemon_opts.export_timing {
        match export_timing_csv() {
            Ok(csv) => {
                println!("{csv}");
            }
            Err(e) => {
                eprintln!("Error exporting timing data: {e}");
                return Err(e);
            }
        }
    } else if daemon_opts.timing_summary {
        match get_timing_summary() {
            Ok((daemon_avg, direct_avg, daemon_count, direct_count)) => {
                println!("📊 TIMING SUMMARY");
                println!("================");
                println!("👹 Daemon mode:");
                println!("   Average: {:.3}ms", daemon_avg.as_secs_f64() * 1000.0);
                println!("   Samples: {daemon_count}");
                println!("📥 Direct mode:");
                println!("   Average: {:.3}ms", direct_avg.as_secs_f64() * 1000.0);
                println!("   Samples: {direct_count}");
                if daemon_count > 0 && direct_count > 0 {
                    let improvement = direct_avg.as_secs_f64() - daemon_avg.as_secs_f64();
                    let percentage = (improvement / direct_avg.as_secs_f64()) * 100.0;
                    println!("⚡ Performance:");
                    println!(
                        "   Daemon is {:.3}ms faster ({:.1}% improvement)",
                        improvement * 1000.0,
                        percentage
                    );
                }
            }
            Err(e) => {
                eprintln!("Error getting timing summary: {e}");
                return Err(e);
            }
        }
    } else {
        println!("Usage: aka daemon [--install|--uninstall|--reinstall|--start|--stop|--restart|--reload|--status|--legend|--export-timing|--timing-summary]");
        return Ok(());
    }

    Ok(())
}

fn route_command_by_health_status(health_status: i32, opts: &AkaOpts) -> Result<i32> {
    match health_status {
        0 => {
            // Health check passed - daemon is healthy, use daemon
            debug!("✅ Health check passed (status=0), daemon is healthy");
            debug!("🔀 Routing to handle_command_via_daemon_with_fallback");
            handle_command_via_daemon_with_fallback(opts)
        }
        _ => {
            // Any non-zero status means fallback to direct mode
            debug!("⚠️ Health check returned status={health_status}, falling back to direct mode");
            debug!("🔀 Routing directly to handle_command_direct_timed");

            // Log the specific reason for fallback
            match health_status {
                1 => debug!("📋 Reason: Config file not found"),
                2 => debug!("📋 Reason: Config file invalid"),
                3 => debug!("📋 Reason: No aliases defined"),
                4 => debug!("📋 Reason: Stale socket detected"),
                _ => debug!("📋 Reason: Unknown health check status"),
            }

            let mut timing = TimingCollector::new(ProcessingMode::Direct);
            let result = handle_command_direct_timed(opts, &mut timing);
            let timing_data = timing.finalize();
            log_timing(timing_data);
            result
        }
    }
}

fn handle_regular_command(opts: &AkaOpts) -> Result<i32> {
    debug!("🎯 === STARTING REGULAR COMMAND PROCESSING ===");
    debug!("🔍 Command options: {opts:?}");

    // Handle explicit health check command
    if let Some(Command::HealthCheck) = &opts.command {
        debug!("🏥 Explicit health check command requested");
        let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory"))?;
        return execute_health_check(&home_dir, &opts.config);
    }

    // CRITICAL: If --config is specified, ALWAYS use direct mode
    // The daemon cannot handle custom configs, so we must process directly
    if opts.config.is_some() {
        debug!("🔧 Custom config specified (--config), forcing direct mode");
        debug!("🎯 Bypassing health check - daemon cannot handle custom configs");
        let mut timing = TimingCollector::new(ProcessingMode::Direct);
        let result = handle_command_direct_timed(opts, &mut timing);
        let timing_data = timing.finalize();
        log_timing(timing_data);
        return result;
    }

    // For all other commands, use health check to determine the best path
    debug!("🔍 Using health check to determine processing path");
    debug!("📋 About to run execute_health_check with config: {:?}", opts.config);

    // Run health check to determine system state
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory"))?;
    let health_status = execute_health_check(&home_dir, &opts.config)?;
    debug!("📊 Health check completed with status: {health_status}");

    route_command_by_health_status(health_status, opts)
}

fn handle_command_via_daemon_with_fallback(opts: &AkaOpts) -> Result<i32> {
    debug!("🎯 Processing command via daemon with fallback");
    debug!("🔍 Attempting daemon path first");

    // Start timing for daemon attempt
    let mut timing = TimingCollector::new(ProcessingMode::Daemon);

    // Quick check if daemon is available
    let home_dir = match dirs::home_dir() {
        Some(dir) => dir,
        None => {
            warn!("❌ Cannot determine home directory, using direct path");
            let mut direct_timing = TimingCollector::new(ProcessingMode::Direct);
            let result = handle_command_direct_timed(opts, &mut direct_timing);
            let timing_data = direct_timing.finalize();
            log_timing(timing_data);
            return result;
        }
    };
    match determine_socket_path(&home_dir) {
        Ok(socket_path) => {
            debug!("🔌 Socket path determined: {socket_path:?}");
            if socket_path.exists() {
                debug!("✅ Socket file exists, attempting daemon communication");

                // Try daemon approach with timing
                match handle_command_via_daemon_only_timed(opts, &mut timing) {
                    Ok(result) => {
                        debug!("✅ Daemon path successful");
                        debug!("🎯 === DAEMON-WITH-FALLBACK COMPLETE (DAEMON SUCCESS) ===");

                        // Log daemon timing
                        let timing_data = timing.finalize();
                        log_timing(timing_data);

                        return Ok(result);
                    }
                    Err(e) => {
                        warn!("⚠️ Daemon path failed: {e}, falling back to direct");
                        debug!("🔄 Daemon communication failed, will try direct path");
                    }
                }
            } else {
                debug!("❌ Socket file does not exist, using direct path");
                debug!("📁 No daemon socket, using direct path");
            }
        }
        Err(e) => {
            warn!("❌ Cannot determine socket path: {e}, using direct path");
        }
    }

    // Fallback to direct processing with timing
    info!("🔄 Falling back to direct config processing");
    debug!("🔀 Routing to handle_command_direct");

    let mut direct_timing = TimingCollector::new(ProcessingMode::Direct);
    let result = handle_command_direct_timed(opts, &mut direct_timing);

    // Log direct timing
    let timing_data = direct_timing.finalize();
    log_timing(timing_data);

    debug!("🎯 Direct fallback complete");
    result
}

fn handle_daemon_query_response(response: DaemonResponse, timing: &mut TimingCollector) -> Result<i32> {
    match response {
        DaemonResponse::Success { data } => {
            debug!("✅ Daemon query successful");
            println!("{data}");
            timing.end_processing();
            debug!("🎯 === DAEMON-ONLY COMPLETE (SUCCESS) ===");
            Ok(0)
        }
        DaemonResponse::Error { message } => {
            warn!("❌ Daemon returned error: {message}");
            eprintln!("Daemon error: {message}");
            timing.end_processing();
            debug!("🎯 === DAEMON-ONLY COMPLETE (DAEMON ERROR) ===");
            Ok(1)
        }
        DaemonResponse::VersionMismatch {
            daemon_version,
            client_version,
            message,
        } => {
            info!("🔄 Version mismatch detected");
            info!("   Daemon: {daemon_version} → Client: {client_version}");
            info!("   {message}");
            debug!("Daemon is restarting, fallback will handle retry");
            timing.end_processing();
            debug!("🎯 === DAEMON-ONLY COMPLETE (VERSION MISMATCH) ===");
            // Return error to trigger fallback to direct mode
            Err(eyre::eyre!("Daemon version mismatch - daemon restarting"))
        }
        _ => {
            warn!("❌ Daemon returned unexpected response: {response:?}");
            eprintln!("Unexpected daemon response");
            timing.end_processing();
            debug!("🎯 === DAEMON-ONLY COMPLETE (UNEXPECTED RESPONSE) ===");
            Ok(1)
        }
    }
}

fn handle_daemon_list_response(response: DaemonResponse, timing: &mut TimingCollector) -> Result<i32> {
    match response {
        DaemonResponse::Success { data } => {
            debug!("✅ Daemon list successful");
            println!("{data}");
            timing.end_processing();
            Ok(0)
        }
        DaemonResponse::Error { message } => {
            warn!("❌ Daemon returned error: {message}");
            eprintln!("Daemon error: {message}");
            timing.end_processing();
            Ok(1)
        }
        DaemonResponse::VersionMismatch {
            daemon_version,
            client_version,
            message,
        } => {
            info!("🔄 Version mismatch detected");
            info!("   Daemon: {daemon_version} → Client: {client_version}");
            info!("   {message}");
            debug!("Daemon is restarting, fallback will handle retry");
            timing.end_processing();
            // Return error to trigger fallback to direct mode
            Err(eyre::eyre!("Daemon version mismatch - daemon restarting"))
        }
        _ => {
            warn!("❌ Daemon returned unexpected response");
            eprintln!("Unexpected daemon response");
            timing.end_processing();
            Ok(1)
        }
    }
}

fn handle_command_via_daemon_only_timed(opts: &AkaOpts, timing: &mut TimingCollector) -> Result<i32> {
    debug!("🎯 Processing command via daemon only");
    debug!("🔍 Daemon-only handler - NO fallback to config loading");
    debug!("📋 Health check already confirmed daemon was healthy");

    timing.start_processing();

    if let Some(ref command) = &opts.command {
        debug!("🔍 Processing command: {command:?}");
        match command {
            Command::Query(query_opts) => {
                debug!("📤 Preparing daemon query request");
                let request = DaemonRequest::Query {
                    version: CLI_VERSION.to_string(),
                    cmdline: query_opts.cmdline.clone(),
                    eol: opts.eol,
                    config: opts.config.clone(),
                };
                debug!("📤 Sending daemon query: {}", query_opts.cmdline);

                match DaemonClient::send_request_timed(request, timing) {
                    Ok(response) => handle_daemon_query_response(response, timing),
                    Err(e) => {
                        warn!("❌ Daemon request failed: {e}");
                        debug!("🔄 Daemon communication failed, will fallback to direct mode");
                        timing.end_processing();
                        debug!("🎯 === DAEMON-ONLY COMPLETE (COMMUNICATION ERROR) ===");
                        Err(eyre::eyre!("Daemon communication failed: {}", e))
                    }
                }
            }
            Command::List(list_opts) => {
                let request = DaemonRequest::List {
                    version: CLI_VERSION.to_string(),
                    global: list_opts.global,
                    patterns: list_opts.patterns.clone(),
                    config: opts.config.clone(),
                };
                debug!("📤 Sending daemon list request");
                match DaemonClient::send_request_timed(request, timing) {
                    Ok(response) => handle_daemon_list_response(response, timing),
                    Err(e) => {
                        warn!("❌ Daemon request failed: {e}");
                        debug!("🔄 Daemon communication failed, will fallback to direct mode");
                        timing.end_processing();
                        Ok(1)
                    }
                }
            }
            Command::Freq(freq_opts) => {
                debug!("📤 Preparing daemon frequency request");
                let request = DaemonRequest::Freq {
                    version: CLI_VERSION.to_string(),
                    all: freq_opts.all,
                    config: opts.config.clone(),
                };
                debug!("📤 Sending daemon frequency request");
                match DaemonClient::send_request_timed(request, timing) {
                    Ok(response) => handle_daemon_query_response(response, timing),
                    Err(e) => {
                        warn!("❌ Daemon request failed: {e}");
                        debug!("🔄 Daemon communication failed, will fallback to direct mode");
                        timing.end_processing();
                        Ok(1)
                    }
                }
            }
            Command::CompleteAliases => {
                debug!("📤 Preparing daemon complete aliases request");
                let request = DaemonRequest::CompleteAliases {
                    version: CLI_VERSION.to_string(),
                    config: opts.config.clone(),
                };
                debug!("📤 Sending daemon complete aliases request");

                match DaemonClient::send_request_timed(request, timing) {
                    Ok(response) => handle_daemon_query_response(response, timing),
                    Err(e) => {
                        warn!("❌ Daemon request failed: {e}");
                        debug!("🔄 Daemon communication failed, will fallback to direct mode");
                        timing.end_processing();
                        debug!("🎯 === DAEMON-ONLY COMPLETE (COMMUNICATION ERROR) ===");
                        Err(eyre::eyre!("Daemon communication failed: {}", e))
                    }
                }
            }
            _ => {
                warn!("❌ Command not supported in daemon-only mode");
                eprintln!("Command not supported in daemon mode");
                timing.end_processing();
                Ok(1)
            }
        }
    } else {
        timing.end_processing();
        Ok(0)
    }
}

fn handle_command_direct_timed(opts: &AkaOpts, timing: &mut TimingCollector) -> Result<i32> {
    debug!("🎯 Processing command directly");
    debug!("🔍 Direct processing - loading config fresh");

    timing.start_config_load();

    // Get home directory - respect HOME environment variable for tests
    let home_dir = match std::env::var("HOME").ok().map(PathBuf::from).or_else(dirs::home_dir) {
        Some(dir) => dir,
        None => {
            warn!("❌ Cannot determine home directory");
            return Err(eyre::eyre!("Unable to determine home directory"));
        }
    };

    // Resolve config path with override support
    let config_path = get_config_path_with_override(&home_dir, &opts.config)?;

    // Create AKA instance (this loads config)
    let mut aka = match AKA::new(opts.eol, home_dir, config_path) {
        Ok(aka) => {
            debug!("✅ AKA instance created successfully");
            aka
        }
        Err(e) => {
            warn!("❌ Failed to create AKA instance: {e}");
            return Err(e);
        }
    };

    timing.end_config_load();
    timing.start_processing();

    if let Some(ref command) = &opts.command {
        debug!("🔍 Processing command: {command:?}");
        match command {
            Command::Query(query_opts) => {
                debug!("📤 Processing query: {}", query_opts.cmdline);
                match aka.replace_with_mode(&query_opts.cmdline, ProcessingMode::Direct) {
                    Ok(result) => {
                        debug!("✅ Query processed successfully");
                        println!("{result}");
                        timing.end_processing();
                        Ok(0)
                    }
                    Err(e) => {
                        warn!("❌ Query processing failed: {e}");
                        eprintln!("Error: {e}");
                        timing.end_processing();
                        Ok(1)
                    }
                }
            }
            Command::List(list_opts) => {
                debug!("📤 Processing list request");

                let output = aka_lib::format_aliases_efficiently(
                    aka.spec.aliases.values(),
                    false, // show_counts
                    true,  // show_all (ls always shows all)
                    list_opts.global,
                    &list_opts.patterns,
                );

                println!("{output}");

                debug!("✅ Listed aliases");
                timing.end_processing();
                Ok(0)
            }
            Command::Freq(freq_opts) => {
                debug!("📤 Processing frequency request");

                let output = aka_lib::format_aliases_efficiently(
                    aka.spec.aliases.values(),
                    true, // show_counts
                    freq_opts.all,
                    false, // global_only (freq doesn't filter by global)
                    &[],   // patterns (freq doesn't support patterns)
                );

                println!("{output}");

                debug!("✅ Showed frequency for aliases");
                timing.end_processing();
                Ok(0)
            }
            Command::CompleteAliases => {
                debug!("📤 Processing complete aliases request");
                let alias_names = aka_lib::get_alias_names_for_completion(&aka);
                for name in alias_names {
                    println!("{name}");
                }
                debug!("✅ Complete aliases processed successfully");
                timing.end_processing();
                Ok(0)
            }
            _ => {
                warn!("❌ Command not supported in direct mode");
                eprintln!("Command not supported in direct mode");
                timing.end_processing();
                Ok(1)
            }
        }
    } else {
        timing.end_processing();
        Ok(0)
    }
}

fn handle_shell_init(shell_opts: &ShellInitOpts) -> Result<i32> {
    match aka_lib::shell::generate_init_script(&shell_opts.shell) {
        Some(script) => {
            print!("{}", script);
            Ok(0)
        }
        None => {
            let supported = aka_lib::shell::supported_shells().join(", ");
            eprintln!(
                "Unsupported shell: '{}'. Supported shells: {}",
                shell_opts.shell, supported
            );
            Ok(1)
        }
    }
}

fn main() {
    let opts = AkaOpts::parse();

    // Set up logging
    let home_dir = match dirs::home_dir() {
        Some(dir) => dir,
        None => {
            eprintln!("Error: Unable to determine home directory");
            exit(1);
        }
    };
    if let Err(e) = setup_logging(&home_dir) {
        eprintln!("Warning: Failed to set up logging: {e}");
    }

    // Route commands - some bypass the regular command flow
    let result = match &opts.command {
        // shell-init doesn't need daemon or config loading
        Some(Command::ShellInit(shell_opts)) => match handle_shell_init(shell_opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        // daemon commands have their own handler
        Some(Command::Daemon(daemon_opts)) => match handle_daemon_command(daemon_opts) {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        // everything else goes through regular command handling
        _ => match handle_regular_command(&opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
    };

    exit(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_process_check() {
        // Test daemon process checking
        let result = check_daemon_process_simple();
        // Should return bool without panicking
        let _ = result; // Just verify it doesn't panic
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
        let home_dir = std::env::temp_dir();
        let result = determine_socket_path(&home_dir);
        // Should either succeed or fail gracefully
        if let Ok(path) = result {
            assert!(path.to_string_lossy().contains("aka"));
        }
        // Err case is acceptable in test environment
    }

    #[test]
    fn test_daemon_request_serialization() {
        // Test that daemon requests can be serialized
        let request = DaemonRequest::Health;
        let serialized = serde_json::to_string(&request);
        assert!(serialized.is_ok());

        let query_request = DaemonRequest::Query {
            version: "v0.5.0".to_string(),
            cmdline: "test".to_string(),
            eol: false,
            config: None,
        };
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

    // DaemonError tests
    #[test]
    fn test_daemon_error_display_connection_timeout() {
        let error = DaemonError::ConnectionTimeout;
        assert_eq!(error.to_string(), "Daemon connection timeout");
    }

    #[test]
    fn test_daemon_error_display_read_timeout() {
        let error = DaemonError::ReadTimeout;
        assert_eq!(error.to_string(), "Daemon read timeout");
    }

    #[test]
    fn test_daemon_error_display_write_timeout() {
        let error = DaemonError::WriteTimeout;
        assert_eq!(error.to_string(), "Daemon write timeout");
    }

    #[test]
    fn test_daemon_error_display_connection_refused() {
        let error = DaemonError::ConnectionRefused;
        assert_eq!(error.to_string(), "Daemon connection refused");
    }

    #[test]
    fn test_daemon_error_display_socket_not_found() {
        let error = DaemonError::SocketNotFound;
        assert_eq!(error.to_string(), "Daemon socket not found");
    }

    #[test]
    fn test_daemon_error_display_socket_permission_denied() {
        let error = DaemonError::SocketPermissionDenied;
        assert_eq!(error.to_string(), "Daemon socket permission denied");
    }

    #[test]
    fn test_daemon_error_display_protocol_error() {
        let error = DaemonError::ProtocolError("test error".to_string());
        assert_eq!(error.to_string(), "Daemon protocol error: test error");
    }

    #[test]
    fn test_daemon_error_display_daemon_shutdown() {
        let error = DaemonError::DaemonShutdown;
        assert_eq!(error.to_string(), "Daemon is shutting down");
    }

    #[test]
    fn test_daemon_error_display_total_operation_timeout() {
        let error = DaemonError::TotalOperationTimeout;
        assert_eq!(error.to_string(), "Total daemon operation timeout");
    }

    #[test]
    fn test_daemon_error_display_unknown_error() {
        let error = DaemonError::UnknownError("something happened".to_string());
        assert_eq!(error.to_string(), "Unknown daemon error: something happened");
    }

    #[test]
    fn test_should_retry_daemon_error() {
        // Should retry
        assert!(should_retry_daemon_error(&DaemonError::ConnectionTimeout));
        assert!(should_retry_daemon_error(&DaemonError::ConnectionRefused));

        // Should not retry
        assert!(!should_retry_daemon_error(&DaemonError::ReadTimeout));
        assert!(!should_retry_daemon_error(&DaemonError::WriteTimeout));
        assert!(!should_retry_daemon_error(&DaemonError::SocketNotFound));
        assert!(!should_retry_daemon_error(&DaemonError::SocketPermissionDenied));
        assert!(!should_retry_daemon_error(&DaemonError::ProtocolError(
            "test".to_string()
        )));
        assert!(!should_retry_daemon_error(&DaemonError::DaemonShutdown));
        assert!(!should_retry_daemon_error(&DaemonError::TotalOperationTimeout));
        assert!(!should_retry_daemon_error(&DaemonError::UnknownError(
            "test".to_string()
        )));
    }

    #[test]
    fn test_categorize_daemon_error() {
        use std::io::{Error, ErrorKind};

        let error = Error::new(ErrorKind::TimedOut, "timeout");
        assert!(matches!(
            categorize_daemon_error(&error),
            DaemonError::ConnectionTimeout
        ));

        let error = Error::new(ErrorKind::ConnectionRefused, "refused");
        assert!(matches!(
            categorize_daemon_error(&error),
            DaemonError::ConnectionRefused
        ));

        let error = Error::new(ErrorKind::NotFound, "not found");
        assert!(matches!(categorize_daemon_error(&error), DaemonError::SocketNotFound));

        let error = Error::new(ErrorKind::PermissionDenied, "denied");
        assert!(matches!(
            categorize_daemon_error(&error),
            DaemonError::SocketPermissionDenied
        ));

        let error = Error::new(ErrorKind::WouldBlock, "would block");
        assert!(matches!(categorize_daemon_error(&error), DaemonError::ReadTimeout));

        let error = Error::other("other");
        assert!(matches!(categorize_daemon_error(&error), DaemonError::UnknownError(_)));
    }

    #[test]
    fn test_validate_socket_path_not_found() {
        let socket_path = PathBuf::from("/nonexistent/path/to/socket.sock");
        let result = validate_socket_path(&socket_path);
        assert!(matches!(result, Err(DaemonError::SocketNotFound)));
    }

    #[test]
    fn test_validate_socket_path_not_socket() {
        use tempfile::NamedTempFile;

        // Create a regular file (not a socket)
        let temp_file = NamedTempFile::new().unwrap();
        let result = validate_socket_path(&temp_file.path().to_path_buf());

        // Should fail because it's not a socket
        assert!(result.is_err());
    }

    #[test]
    fn test_daemon_error_clone() {
        let error = DaemonError::ConnectionTimeout;
        let cloned = error.clone();
        assert_eq!(error.to_string(), cloned.to_string());
    }

    #[test]
    fn test_daemon_error_debug() {
        let error = DaemonError::ConnectionTimeout;
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("ConnectionTimeout"));
    }

    #[test]
    fn test_daemon_error_is_std_error() {
        let error: Box<dyn std::error::Error> = Box::new(DaemonError::ConnectionTimeout);
        let _ = error.to_string();
    }

    // Opts struct tests
    #[test]
    fn test_query_opts_debug() {
        let opts = QueryOpts {
            cmdline: "test command".to_string(),
        };
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("test command"));
    }

    #[test]
    fn test_list_opts_debug() {
        let opts = ListOpts {
            global: true,
            patterns: vec!["test".to_string()],
        };
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("global"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_freq_opts_debug() {
        let opts = FreqOpts { all: true };
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("all"));
    }

    #[test]
    fn test_shell_init_opts_debug() {
        let opts = ShellInitOpts {
            shell: "zsh".to_string(),
        };
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("zsh"));
    }

    #[test]
    fn test_daemon_opts_debug() {
        let opts = DaemonOpts {
            install: false,
            uninstall: false,
            reinstall: false,
            start: true,
            stop: false,
            restart: false,
            reload: false,
            status: false,
            legend: false,
            export_timing: false,
            timing_summary: false,
        };
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("start"));
    }

    #[test]
    fn test_daemon_request_list() {
        let request = DaemonRequest::List {
            version: "v0.5.0".to_string(),
            global: true,
            patterns: vec!["git".to_string()],
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("List"));
        assert!(serialized.contains("global"));
        assert!(serialized.contains("git"));
    }

    #[test]
    fn test_daemon_request_freq() {
        let request = DaemonRequest::Freq {
            version: "v0.5.0".to_string(),
            all: true,
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("Freq"));
        assert!(serialized.contains("all"));
    }

    #[test]
    fn test_daemon_request_reload_config() {
        let request = DaemonRequest::ReloadConfig;
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("ReloadConfig"));
    }

    #[test]
    fn test_daemon_request_shutdown() {
        let request = DaemonRequest::Shutdown;
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("Shutdown"));
    }

    #[test]
    fn test_daemon_request_complete_aliases() {
        let request = DaemonRequest::CompleteAliases {
            version: "v0.5.0".to_string(),
            config: None,
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(serialized.contains("CompleteAliases"));
    }

    #[test]
    fn test_daemon_response_success() {
        let response = DaemonResponse::Success {
            data: "test data".to_string(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("Success"));
        assert!(serialized.contains("test data"));
    }

    #[test]
    fn test_daemon_response_error() {
        let response = DaemonResponse::Error {
            message: "test error".to_string(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("Error"));
        assert!(serialized.contains("test error"));
    }

    #[test]
    fn test_daemon_response_config_reloaded() {
        let response = DaemonResponse::ConfigReloaded {
            success: true,
            message: "Config reloaded".to_string(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("ConfigReloaded"));
    }

    #[test]
    fn test_daemon_response_shutdown_ack() {
        let response = DaemonResponse::ShutdownAck;
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("ShutdownAck"));
    }

    #[test]
    fn test_daemon_response_version_mismatch() {
        let response = DaemonResponse::VersionMismatch {
            daemon_version: "v1.0.0".to_string(),
            client_version: "v0.9.0".to_string(),
            message: "Version mismatch".to_string(),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(serialized.contains("VersionMismatch"));
        assert!(serialized.contains("v1.0.0"));
        assert!(serialized.contains("v0.9.0"));
    }
}
