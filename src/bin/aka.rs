use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use eyre::Result;
use log::{debug, info, warn};
use std::path::PathBuf;
use std::process::exit;

// Import from the shared library
use aka_lib::daemon_client::{DaemonClient as LibDaemonClient, DaemonError};
use aka_lib::{
    determine_socket_path, execute_health_check, export_timing_csv, get_alias_cache_path, get_config_path,
    get_config_path_with_override, get_last_valid_config_path, get_timing_summary, load_alias_cache, log_file_path,
    log_timing, probe_daemon_health, setup_logging, xdg_config_dir, ConfigLoader, DaemonHealth, DaemonRequest,
    DaemonResponse, ProcessingMode, TimingCollector, AKA,
};

// Version constant for compatibility checking
const CLI_VERSION: &str = env!("GIT_DESCRIBE");

// Health status constants
const CACHE_FALLBACK: i32 = 5; // config broken, serving from last-valid cache

// Marker embedded in the top-level `after_help` so the help interceptor can
// distinguish top-level help (which gets the dynamic daemon-status line) from
// subcommand help (which clap renders without any `after_help`).
const AFTER_HELP_LOG_MARKER: &str = "Logs are written to:";

/// Thin CLI-side wrapper over the canonical `aka_lib::daemon_client::DaemonClient`.
/// The binary keeps this shim only to (a) resolve the socket path from the home
/// dir and (b) preserve `send_request_timed`'s `TimingCollector` bracketing; all
/// connect/retry/timeout/error taxonomy lives in the shared client.
struct DaemonClient;

impl DaemonClient {
    fn send_request(request: DaemonRequest) -> Result<DaemonResponse> {
        let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Unable to determine home directory"))?;
        let socket_path = determine_socket_path(&home_dir)?;
        debug!("DaemonClient::send_request: socket_path={socket_path:?}");
        LibDaemonClient::new()
            .send_request(request, &socket_path)
            .map_err(|e| eyre::eyre!("{e}"))
    }

    fn send_request_timed(request: DaemonRequest, timing: &mut TimingCollector) -> Result<DaemonResponse, DaemonError> {
        timing.start_ipc();
        let result = Self::send_request(request).map_err(|e| DaemonError::UnknownError(e.to_string()));
        timing.end_ipc();
        result
    }
}

/// Static `after_help`: the log path only, resolved from the same source the
/// logger uses so `--help` never lies about where logs go. Evaluated on every
/// invocation by clap's `command()` builder, so it must stay probe-free - the
/// daemon-status line is appended lazily by the help interceptor in `main`.
fn get_after_help() -> &'static str {
    let log_path = match dirs::home_dir() {
        Some(home) => log_file_path(&home).display().to_string(),
        None => "~/.local/share/aka/logs/aka.log".to_string(),
    };
    Box::leak(format!("{AFTER_HELP_LOG_MARKER} {log_path}").into_boxed_str())
}

/// Build the `Environment=` lines to bake into the generated systemd unit.
///
/// `systemd --user` starts with a minimal environment and does NOT inherit
/// `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` exported in the interactive shell. Left
/// unset, the daemon would resolve cache/log paths from the default home layout
/// while an interactive `aka` honored the shell's XDG overrides - the exact
/// split-brain the XDG migration exists to cure. We snapshot only *absolute*
/// values (the only shape the `xdg_*_dir` helpers honor). Returns lines that are
/// each newline-terminated, or an empty string when neither var is set.
fn xdg_environment_lines() -> String {
    debug!("xdg_environment_lines: snapshotting XDG_CONFIG_HOME/XDG_DATA_HOME");
    let mut lines = String::new();
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME"] {
        if let Ok(val) = std::env::var(key) {
            if PathBuf::from(&val).is_absolute() {
                lines.push_str(&format!("Environment={key}={val}\n"));
            }
        }
    }
    debug!("xdg_environment_lines: produced {} line(s)", lines.lines().count());
    lines
}

fn get_daemon_status_emoji() -> &'static str {
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
            // Daemon appears to be running, check config sync status via the
            // one bounded health probe (a wedged daemon can no longer hang
            // `--help` on an unbounded read).
            match probe_daemon_health(&socket_path) {
                DaemonHealth::Synced { .. } => "✅",                         // Healthy and synced
                DaemonHealth::Stale { .. } => "🔄",                          // Config out of sync
                DaemonHealth::Unhealthy | DaemonHealth::Unreachable => "⚠️", // Health check failed
            }
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

    #[clap(name = "check", about = "validate aka config and print errors")]
    Check(CheckOpts),

    #[clap(name = "restore", about = "restore last-valid config backup")]
    Restore(RestoreOpts),

    #[clap(name = "edit", about = "safely edit aka config (validate before applying)")]
    Edit,

    #[clap(name = "disable", about = "disable aka ZLE integration (create killswitch)")]
    Disable,

    #[clap(name = "enable", about = "enable aka ZLE integration (remove killswitch)")]
    Enable,

    #[clap(name = "prompt-status", about = "print status for shell prompt integration")]
    PromptStatus,
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

#[derive(Parser, Debug)]
struct CheckOpts {
    #[clap(long, help = "suppress output, exit code only")]
    quiet: bool,

    #[clap(long, help = "output results as JSON")]
    json: bool,
}

#[derive(Parser, Debug)]
struct RestoreOpts {
    #[clap(long, help = "show diff only, do not restore")]
    diff: bool,

    #[clap(long, help = "restore without confirmation prompt")]
    force: bool,
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
        let service_dir = xdg_config_dir()
            .ok_or_else(|| eyre::eyre!("Could not determine config directory"))?
            .join("systemd/user");
        fs::create_dir_all(&service_dir)?;

        // Get aka-daemon binary path
        let daemon_path = self.get_daemon_binary_path()?;

        // systemd --user does NOT inherit $XDG_CONFIG_HOME / $XDG_DATA_HOME exported
        // in the interactive shell (e.g. .zshrc). Snapshot whatever is set now into
        // Environment= lines so the daemon resolves cache/log paths identically to the
        // CLI. Changing these env vars later requires `aka daemon --reinstall`.
        let xdg_env_lines = xdg_environment_lines();

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
{}[Install]
WantedBy=default.target
"#,
            daemon_path.display(),
            dirs::home_dir()
                .ok_or_else(|| eyre::eyre!("Could not determine home directory"))?
                .join(".cargo/bin")
                .display(),
            xdg_env_lines,
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

        // Show the cache/log paths this CLI resolves to. The systemd-launched daemon
        // uses the XDG env snapshotted into its unit at install time, so if these were
        // changed since install they may differ - re-sync with `aka daemon --reinstall`.
        match get_alias_cache_path(&home_dir) {
            Ok(cache_path) => println!("🗃️  Cache file: {}", cache_path.display()),
            Err(e) => println!("🗃️  Cache file: ⚠️  could not resolve ({e})"),
        }
        println!("📝 Log file: {}", log_file_path(&home_dir).display());
        if std::env::var("XDG_CONFIG_HOME").is_ok() || std::env::var("XDG_DATA_HOME").is_ok() {
            println!("   ⚠️  XDG_CONFIG_HOME/XDG_DATA_HOME is set; run `aka daemon --reinstall` after changing it so the daemon stays in sync");
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
        let service_file = xdg_config_dir()
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
            let service_file = xdg_config_dir()
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
        CACHE_FALLBACK => {
            // Config is broken but cache has aliases - serve from cache
            debug!(
                "⚠️ Health check returned CACHE_FALLBACK (status={CACHE_FALLBACK}), config invalid but cache available"
            );
            debug!("🔀 Routing to handle_command_from_cache");
            handle_command_from_cache(opts)
        }
        _ => {
            // Any other non-zero status means fallback to direct mode
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

fn handle_command_from_cache(opts: &AkaOpts) -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let cache = load_alias_cache(&home_dir)?;

    if cache.aliases.is_empty() {
        return Ok(0);
    }

    let aka = AKA::from_cache(cache, home_dir);
    handle_command_direct_with_aka(aka, opts)
}

fn handle_command_direct_with_aka(mut aka: AKA, opts: &AkaOpts) -> Result<i32> {
    if let Some(ref command) = &opts.command {
        match command {
            Command::Query(query_opts) => match aka.replace_with_mode(&query_opts.cmdline, ProcessingMode::Direct) {
                Ok(result) => {
                    println!("{result}");
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    Ok(1)
                }
            },
            Command::List(list_opts) => {
                let output = aka_lib::format_aliases_efficiently(
                    aka.spec.aliases.values(),
                    false,
                    true,
                    list_opts.global,
                    &list_opts.patterns,
                );
                println!("{output}");
                Ok(0)
            }
            Command::Freq(freq_opts) => {
                let output =
                    aka_lib::format_aliases_efficiently(aka.spec.aliases.values(), true, freq_opts.all, false, &[]);
                println!("{output}");
                Ok(0)
            }
            Command::CompleteAliases => {
                let alias_names = aka_lib::get_alias_names_for_completion(&aka);
                for name in alias_names {
                    println!("{name}");
                }
                Ok(0)
            }
            _ => {
                eprintln!("Command not supported in cache fallback mode");
                Ok(1)
            }
        }
    } else {
        Ok(0)
    }
}

fn handle_check(opts: &CheckOpts) -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let config_path = get_config_path(&home_dir)?;

    if opts.quiet {
        let loader = ConfigLoader::new();
        return match loader.load(&config_path) {
            Ok(_) => Ok(0),
            Err(_) => Ok(1),
        };
    }

    let loader = ConfigLoader::new();
    match loader.load(&config_path) {
        Ok(spec) => {
            if opts.json {
                println!(
                    r#"{{"status":"valid","path":"{}","alias_count":{}}}"#,
                    config_path.display(),
                    spec.aliases.len()
                );
            } else {
                println!("Config: {}", config_path.display());
                println!("Status: VALID ({} aliases)", spec.aliases.len());
            }
            Ok(0)
        }
        Err(e) => {
            if opts.json {
                let msg = e.to_string().replace('"', "\\\"");
                println!(
                    r#"{{"status":"invalid","path":"{}","errors":["{msg}"]}}"#,
                    config_path.display()
                );
            } else {
                eprintln!("Config: {}", config_path.display());
                eprintln!("Status: INVALID");
                eprintln!();
                eprintln!("Errors:");
                for line in e.to_string().lines() {
                    eprintln!("  {line}");
                }
            }
            Ok(1)
        }
    }
}

fn handle_restore(opts: &RestoreOpts) -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let backup_path = get_last_valid_config_path(&home_dir)?;

    if !backup_path.exists() {
        eprintln!("No backup available yet. Run aka with a valid config first.");
        return Ok(1);
    }

    let current_path = get_config_path(&home_dir)?;

    if opts.diff {
        let _ = std::process::Command::new("git")
            .args(["diff", "--no-index", "--color=always"])
            .arg(&current_path)
            .arg(&backup_path)
            .status()
            .or_else(|_| {
                std::process::Command::new("diff")
                    .arg("-u")
                    .arg(&current_path)
                    .arg(&backup_path)
                    .status()
            });
        return Ok(0);
    }

    println!("Restoring from: {}", backup_path.display());

    if !opts.force {
        let _ = std::process::Command::new("git")
            .args(["diff", "--no-index", "--color=always"])
            .arg(&current_path)
            .arg(&backup_path)
            .status()
            .or_else(|_| {
                std::process::Command::new("diff")
                    .arg("-u")
                    .arg(&current_path)
                    .arg(&backup_path)
                    .status()
            });

        eprint!("Restore? [y/N]: ");
        use std::io::Write as _;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(0);
        }
    }

    std::fs::copy(&backup_path, &current_path)?;
    println!("Restored. Run 'aka check' to verify.");
    Ok(0)
}

fn handle_edit() -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let config_path = get_config_path(&home_dir)?;

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::copy(&config_path, tmp.path())?;

    loop {
        std::process::Command::new(&editor).arg(tmp.path()).status()?;

        let loader = ConfigLoader::new();
        match loader.load(tmp.path()) {
            Ok(_) => {
                std::fs::copy(tmp.path(), &config_path)?;
                println!("Config saved.");
                return Ok(0);
            }
            Err(e) => {
                eprintln!("Config invalid: {e}");
                eprint!("Re-edit? [Y/n]: ");
                use std::io::Write as _;
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim().eq_ignore_ascii_case("n") {
                    println!("Aborted. Original config unchanged.");
                    return Ok(1);
                }
            }
        }
    }
}

fn handle_disable() -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let killswitch = home_dir.join("aka-killswitch");
    if killswitch.exists() {
        println!("aka already disabled");
    } else {
        std::fs::write(&killswitch, "")?;
        println!("aka disabled (created {})", killswitch.display());
    }
    Ok(0)
}

fn handle_enable() -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let killswitch = home_dir.join("aka-killswitch");
    if killswitch.exists() {
        std::fs::remove_file(&killswitch)?;
        println!("aka enabled (removed {})", killswitch.display());
    } else {
        println!("aka already enabled");
    }
    Ok(0)
}

fn handle_prompt_status() -> Result<i32> {
    let home_dir = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    let config_path = get_config_path(&home_dir)?;
    let loader = ConfigLoader::new();
    match loader.load(&config_path) {
        Ok(_) => {
            print!("");
            Ok(0)
        }
        Err(_) => {
            print!("⚠aka");
            Ok(1)
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

/// Parse CLI args, intercepting help so the daemon-status probe runs ONLY when
/// help is actually requested - never on the per-keystroke `aka query` path.
///
/// Clap is the sole source of help resolution: we build the command with a
/// static `after_help` (log path only) and `try_get_matches`. Help/version and
/// parse errors surface as `clap::Error`; for the top-level help case we append
/// the (probe-backed) daemon-status line. Subcommand help carries no
/// `after_help` marker, so it prints exactly as clap rendered it.
fn parse_opts() -> AkaOpts {
    use clap::error::ErrorKind;

    debug!("parse_opts: intercepting help before daemon probe");
    let matches = match AkaOpts::command().try_get_matches() {
        Ok(matches) => matches,
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                let rendered = e.render().to_string();
                let _ = e.print();
                if is_top_level_help(&rendered) {
                    println!("\nDaemon status: {}", get_daemon_status_emoji());
                }
                exit(0);
            }
            // Version display and genuine parse errors: clap owns the stream and
            // exit code (stdout/0 for version, stderr/2 for usage errors).
            _ => e.exit(),
        },
    };

    match AkaOpts::from_arg_matches(&matches) {
        Ok(opts) => opts,
        Err(e) => e.exit(),
    }
}

/// True when a rendered clap help string is the top-level `aka` help (which
/// carries the `after_help` log-path marker) rather than a subcommand's help.
/// Only top-level help gets the appended daemon-status line.
fn is_top_level_help(rendered_help: &str) -> bool {
    rendered_help.contains(AFTER_HELP_LOG_MARKER)
}

fn main() {
    let opts = parse_opts();

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
        Some(Command::Check(check_opts)) => match handle_check(check_opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        Some(Command::Restore(restore_opts)) => match handle_restore(restore_opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        Some(Command::Edit) => match handle_edit() {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        Some(Command::Disable) => match handle_disable() {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        Some(Command::Enable) => match handle_enable() {
            Ok(code) => code,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        Some(Command::PromptStatus) => match handle_prompt_status() {
            Ok(code) => code,
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
    use std::sync::Mutex;

    /// Serializes the env-mutating tests in this module (env mutation is
    /// process-global and unsafe under parallel tests).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_xdg_environment_lines_snapshots_absolute_only() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_config = std::env::var("XDG_CONFIG_HOME").ok();
        let prior_data = std::env::var("XDG_DATA_HOME").ok();

        // Neither set -> no lines.
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        assert_eq!(xdg_environment_lines(), "");

        // Absolute values are snapshotted; a relative value is ignored.
        std::env::set_var("XDG_CONFIG_HOME", "/abs/config");
        std::env::set_var("XDG_DATA_HOME", "relative/data");
        assert_eq!(xdg_environment_lines(), "Environment=XDG_CONFIG_HOME=/abs/config\n");

        std::env::set_var("XDG_DATA_HOME", "/abs/data");
        assert_eq!(
            xdg_environment_lines(),
            "Environment=XDG_CONFIG_HOME=/abs/config\nEnvironment=XDG_DATA_HOME=/abs/data\n"
        );

        match prior_config {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match prior_data {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn test_daemon_process_check() {
        // Test daemon process checking
        let result = check_daemon_process_simple();
        // Should return bool without panicking
        let _ = result; // Just verify it doesn't panic
    }

    #[test]
    fn test_after_help_carries_log_marker() {
        // The static after_help must contain the log-path marker so the help
        // interceptor can recognize top-level help; it must NOT contain the
        // dynamic daemon-status line (that is appended lazily on the help path).
        let help = get_after_help();
        assert!(
            help.contains(AFTER_HELP_LOG_MARKER),
            "after_help missing log marker: {help}"
        );
        assert!(
            !help.contains("Daemon status:"),
            "after_help must stay probe-free (no daemon status): {help}"
        );
    }

    #[test]
    fn test_is_top_level_help() {
        // Top-level help renders the after_help marker; subcommand help does not.
        let top_level =
            format!("Usage: aka [OPTIONS]\n\n{AFTER_HELP_LOG_MARKER} /home/u/.local/share/aka/logs/aka.log");
        assert!(is_top_level_help(&top_level));

        let subcommand = "Usage: aka query [OPTIONS] <CMDLINE>\n\nquery for aka substitutions";
        assert!(!is_top_level_help(subcommand));
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
        let response_json = r#"{"type":"Health","status":"healthy:5:synced"}"#;
        let response: Result<DaemonResponse, _> = serde_json::from_str(response_json);
        assert!(response.is_ok());

        if let Ok(DaemonResponse::Health { status }) = response {
            assert_eq!(status, "healthy:5:synced");
        }
    }

    // NOTE: DaemonError display / should_retry / categorize / validate_socket_path
    // tests were removed here in Phase 2 - the ad-hoc client they exercised was
    // deleted and its canonical replacement lives in `src/daemon-client.rs`, whose
    // `#[cfg(test)] mod tests` already covers the identical taxonomy against
    // `MockSocketConnector`.

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
