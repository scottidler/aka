use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Result type for aka command execution
pub struct AkaCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Get the path to the aka binary, building it if necessary
pub fn get_aka_binary_path() -> PathBuf {
    ensure_aka_binary_built();
    let mut path = std::env::current_dir().expect("Failed to get current dir");
    path.push("target");
    path.push("debug");
    path.push("aka");
    path
}

/// Ensure the aka binary is built
pub fn ensure_aka_binary_built() {
    let output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !output.status.success() {
        panic!(
            "Failed to build aka binary: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Run an aka command with flexible environment configuration
pub fn run_aka_command(args: &[&str], temp_dir: Option<&TempDir>, config_file: Option<&Path>) -> AkaCommandResult {
    let aka_binary = get_aka_binary_path();
    let mut cmd = Command::new(&aka_binary);

    // Add config file argument if specified
    if let Some(config) = config_file {
        cmd.args(&["--config", config.to_str().unwrap()]);
    }

    cmd.args(args);

    // Set environment variables based on temp_dir
    if let Some(temp) = temp_dir {
        cmd.env("HOME", temp.path());
        cmd.env("XDG_RUNTIME_DIR", temp.path().join("run"));
    } else {
        cmd.env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime");
    }

    cmd.env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log");
    cmd.env("AKA_CACHE_DIR", "/tmp/aka-test-cache");

    let output = cmd.output().expect("Failed to run aka command");
    let success = output.status.success();

    AkaCommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success,
    }
}
