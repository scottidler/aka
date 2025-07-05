use clap::{Parser, Subcommand};
use eyre::Result;
use log::debug;
use std::path::PathBuf;
use std::process::exit;

// Import from the shared library
use aka_lib::{
    setup_logging, execute_health_check, determine_socket_path,
    AKA, print_alias
};

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/git_describe.rs"));
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
        (true, true) => "✅",   // Healthy
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

#[derive(Parser)]
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

#[derive(Subcommand)]
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

#[derive(Parser)]
struct QueryOpts {
    cmdline: String,
}

#[derive(Parser)]
struct ListOpts {
    #[clap(short, long, help = "list global aliases only")]
    global: bool,

    patterns: Vec<String>,
}

#[derive(Parser)]
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
    println!("  ✅ - Daemon is healthy (socket exists + process running)");
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
    } else if daemon_opts.status {
        service_manager.status()?;
    } else if daemon_opts.legend {
        print_daemon_legend();
    } else {
        println!("Usage: aka daemon [--install|--uninstall|--start|--stop|--restart|--status|--legend]");
        return Ok(());
    }

    Ok(())
}

fn handle_regular_command(opts: &AkaOpts) -> Result<i32> {
    // Check if daemon is available (socket file exists)
    let socket_path = determine_socket_path()?;
    if socket_path.exists() {
        debug!("Daemon detected at {:?}, would use fast path (fallback for now)", socket_path);
        // Daemon is available - in real implementation, this would send requests to daemon
        // For now, just fall back to existing implementation silently
    } else {
        debug!("No daemon detected, using direct implementation");
    }

    // Handle health check first, before trying to create AKA instance
    if let Some(ref command) = &opts.command {
        if let Command::HealthCheck = command {
            return execute_health_check(&opts.config);
        }
    }

    let aka = AKA::new(opts.eol, &opts.config)?;
    if let Some(ref command) = opts.command {
        match command {
            Command::Query(query_opts) => {
                let result = aka.replace(&query_opts.cmdline)?;
                println!("{result}");
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