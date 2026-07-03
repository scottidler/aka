use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

mod common;
use common::*;

fn setup_test_environment_with_usage() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("aka.yml");

    // Write test config
    let config_content = r#"
lookups: {}

aliases:
  test-high:
    value: echo "high usage"
    global: true
  test-medium:
    value: echo "medium usage"
    global: false
  test-low:
    value: echo "low usage"
    global: true
  test-unused:
    value: echo "unused"
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    (temp_dir, config_file)
}

#[test]
fn test_freq_command_basic() {
    let (temp_dir, _config_file) = setup_test_environment_with_usage();

    // Set HOME to our temp directory and ensure no daemon socket exists
    let result = run_aka_command(&["freq"], Some(&temp_dir), None);

    if !result.success {
        panic!("aka freq failed: {}", result.stderr);
    }

    // By default, should only show used aliases (count > 0)
    // Since all aliases have count 0, should show "No aliases found."
    assert!(
        result.stdout.contains("No aliases found."),
        "Should show 'No aliases found.' when no aliases are used"
    );
}

#[test]
fn test_freq_command_with_all_option() {
    let (temp_dir, _config_file) = setup_test_environment_with_usage();

    // Test with --all to show all aliases including unused ones
    let result = run_aka_command(&["freq", "--all"], Some(&temp_dir), None);

    if !result.success {
        panic!("aka freq --all failed: {}", result.stderr);
    }

    // Should contain all aliases with count 0 (sorted alphabetically)
    assert!(result.stdout.contains("test-high"));
    assert!(result.stdout.contains("test-medium"));
    assert!(result.stdout.contains("test-low"));
    assert!(result.stdout.contains("test-unused"));

    // All should have count 0
    assert!(result.stdout.contains("0"));

    // Should be formatted with proper spacing
    let lines: Vec<&str> = result.stdout.trim().split('\n').collect();
    assert_eq!(
        lines.len(),
        6,
        "Should have 4 aliases + empty line + count line with --all"
    );

    // Check that lines are properly formatted (count alias -> value)
    // Skip the last 2 lines (empty line and count line)
    for line in &lines[..lines.len() - 2] {
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert!(
            parts.len() >= 4,
            "Each line should have at least 4 parts: count, alias, ->, value"
        );
        assert_eq!(parts[0], "0", "Count should be 0 for unused aliases");
        assert_eq!(parts[2], "->", "Should have -> separator");
    }

    // Check that the last line is the count line
    assert!(
        lines[lines.len() - 1].starts_with("count: "),
        "Last line should be count line"
    );
    assert!(lines[lines.len() - 1].contains("4"), "Count should be 4 for 4 aliases");
}

#[test]
fn test_freq_command_empty_config() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("aka.yml");

    // Write minimal valid config with one alias (to satisfy validation)
    let config_content = r#"
lookups: {}
aliases:
  dummy:
    value: "echo dummy"
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    let result = run_aka_command(&["freq"], Some(&temp_dir), None);

    if !result.success {
        panic!("aka freq failed: {}", result.stderr);
    }

    // By default, should only show used aliases (count > 0)
    // Since dummy alias has count 0, should show "No aliases found."
    assert!(
        result.stdout.contains("No aliases found."),
        "Should show 'No aliases found.' when no aliases are used"
    );
}

#[test]
fn test_freq_command_help() {
    let result = run_aka_command(&["freq", "--help"], None, None);

    if !result.success {
        panic!("aka freq --help failed: {}", result.stderr);
    }

    // Should contain help information
    assert!(
        result.stdout.contains("show alias usage frequency statistics"),
        "Should contain description"
    );
    assert!(result.stdout.contains("--all"), "Should contain --all option");
    assert!(
        result.stdout.contains("show all aliases including unused ones"),
        "Should contain --all description"
    );
}

#[test]
fn test_freq_command_in_main_help() {
    let result = run_aka_command(&["--help"], None, None);

    if !result.success {
        panic!("aka --help failed: {}", result.stderr);
    }

    // Should contain the freq command in the main help
    assert!(
        result.stdout.contains("freq"),
        "Should contain 'freq' command in main help"
    );
    assert!(
        result.stdout.contains("show alias usage frequency statistics"),
        "Should contain freq description"
    );
}

#[test]
fn test_freq_command_with_simulated_usage() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("aka.yml");

    // Write test config
    let config_content = r#"
lookups: {}

aliases:
  test-alias:
    value: echo "test"
    global: true
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    // Create a unique cache directory for this specific test run
    let unique_cache_dir = temp_dir.path().join("isolated-cache");
    std::fs::create_dir_all(&unique_cache_dir).expect("Failed to create cache dir");

    // Use the alias a few times to increment its count
    for i in 1..=3 {
        let mut cmd = std::process::Command::new(common::get_aka_binary_path());
        cmd.args(["query", "test-alias"])
            .env("HOME", temp_dir.path())
            .env("AKA_CACHE_DIR", &unique_cache_dir)
            .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
            .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime");

        let output = cmd.output().expect("Failed to run aka command");

        if !output.status.success() {
            panic!(
                "aka query failed on iteration {}: {}",
                i,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    // Now run freq to see the usage count
    let mut cmd = std::process::Command::new(common::get_aka_binary_path());
    cmd.args(["freq"])
        .env("HOME", temp_dir.path())
        .env("AKA_CACHE_DIR", &unique_cache_dir)
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime");

    let output = cmd.output().expect("Failed to run aka command");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        panic!("aka freq failed: {}", stderr);
    }

    // Should show the alias with count 3 - format is "   3 test-alias -> echo "test""
    assert!(stdout.contains("test-alias"), "Should contain test-alias");
    assert!(
        stdout.contains("   3 test-alias"),
        "Should show count of 3 in the correct format"
    );
    assert!(stdout.contains("echo \"test\""), "Should show the alias value");
}

// ---------------------------------------------------------------------------
// Phase 5: daemon-side debounced cache flush end-to-end.
//
// These spin up a real `aka-daemon` against a temp HOME so the daemon's
// default-config path (ProcessingMode::Daemon) is exercised: usage counts are
// buffered in memory and only reach disk on a flush trigger. The client talks
// to the daemon directly via aka_lib::daemon_client so it can control the
// protocol version (needed for the version-mismatch shutdown flush) and issue
// Freq without shelling back through the CLI.
// ---------------------------------------------------------------------------

use aka_lib::daemon_client::DaemonClient;
use aka_lib::{DaemonRequest, DaemonResponse};
use std::time::Duration;

// Client-side protocol version; identical to what the daemon was built with,
// since both come from the same crate build.
const TEST_CLIENT_VERSION: &str = env!("GIT_DESCRIBE");

/// Kills the spawned daemon when it goes out of scope so a failed assertion
/// never leaks a background process.
struct DaemonGuard(std::process::Child);
impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn build_daemon_binary() {
    let output = std::process::Command::new("cargo")
        .args(["build", "--bin", "aka-daemon"])
        .output()
        .expect("Failed to build aka-daemon binary");
    assert!(
        output.status.success(),
        "aka-daemon build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Write a config with one simple expanding alias under `home/.config/aka/`.
fn write_daemon_test_config(home: &std::path::Path) {
    let config_dir = home.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_content = r#"
lookups: {}

aliases:
  gs:
    value: git status
    global: false
"#;
    fs::write(config_dir.join("aka.yml"), config_content).expect("Failed to write config");
}

/// Spawn the daemon against `home` with isolated runtime/cache/log dirs and no
/// `--config` (so it resolves the default HOME config -> Daemon mode). Returns
/// the guard plus the resolved socket and cache-base paths.
fn spawn_daemon(home: &std::path::Path) -> (DaemonGuard, PathBuf, PathBuf) {
    build_daemon_binary();

    let runtime_dir = home.join("run");
    let cache_dir = home.join("cache");
    fs::create_dir_all(&runtime_dir).expect("Failed to create runtime dir");
    fs::create_dir_all(&cache_dir).expect("Failed to create cache dir");

    let child = std::process::Command::new("target/debug/aka-daemon")
        .arg("--foreground")
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env("AKA_CACHE_DIR", &cache_dir)
        .env("AKA_LOG_FILE", home.join("aka.log"))
        // Ensure ambient XDG_* from CI does not redirect config resolution away
        // from the temp HOME.
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .spawn()
        .expect("Failed to spawn aka-daemon");

    let socket_path = runtime_dir.join("aka").join("daemon.sock");

    // Wait for the daemon to bind its socket.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !socket_path.exists() {
        if std::time::Instant::now() > deadline {
            panic!("daemon socket never appeared at {socket_path:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    (DaemonGuard(child), socket_path, cache_dir)
}

fn query(socket: &std::path::Path, version: &str, cmdline: &str) -> Result<DaemonResponse, String> {
    DaemonClient::new()
        .send_request(
            DaemonRequest::Query {
                version: version.to_string(),
                cmdline: cmdline.to_string(),
                eol: true,
                config: None,
            },
            socket,
        )
        .map_err(|e| e.to_string())
}

/// Poll the cache file for `gs` reaching `expected` count, up to a deadline.
fn wait_for_count(cache_dir: &PathBuf, alias: &str, expected: u64) -> u64 {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let count = aka_lib::load_alias_cache_with_base(Some(cache_dir))
            .ok()
            .and_then(|c| c.aliases.get(alias).map(|a| a.count))
            .unwrap_or(0);
        if count >= expected || std::time::Instant::now() > deadline {
            return count;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn test_daemon_persists_counts_on_freq() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home = temp_dir.path();
    write_daemon_test_config(home);

    let (_guard, socket, cache_dir) = spawn_daemon(home);

    // A daemon-mode query bumps the in-memory count but debounces the disk write.
    let resp = query(&socket, TEST_CLIENT_VERSION, "gs").expect("query failed");
    match resp {
        DaemonResponse::Success { data } => assert_eq!(data.trim(), "git status"),
        other => panic!("unexpected query response: {other:?}"),
    }

    // The pre-Freq flush must persist the buffered count to the cache file.
    let freq = DaemonClient::new()
        .send_request(
            DaemonRequest::Freq {
                version: TEST_CLIENT_VERSION.to_string(),
                all: true,
                config: None,
            },
            &socket,
        )
        .expect("freq request failed");
    assert!(matches!(freq, DaemonResponse::Success { .. }), "freq should succeed");

    let count = wait_for_count(&cache_dir, "gs", 1);
    assert_eq!(count, 1, "pre-Freq flush should persist gs count to the cache file");
}

#[test]
fn test_daemon_flushes_counts_on_version_mismatch_shutdown() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home = temp_dir.path();
    write_daemon_test_config(home);

    let (_guard, socket, cache_dir) = spawn_daemon(home);

    // Buffer a count with a correctly-versioned query.
    let resp = query(&socket, TEST_CLIENT_VERSION, "gs").expect("query failed");
    assert!(matches!(resp, DaemonResponse::Success { .. }), "query should succeed");

    // A wrong-version query triggers the graceful shutdown path, which flushes
    // buffered counts before signalling shutdown (flush-before-reconstruction:
    // the restarted daemon would otherwise reload a cache missing this count).
    let mismatch = query(&socket, "v0.0.0-wrong-version", "gs");
    // Response is VersionMismatch (or a transport error as the daemon tears down);
    // either way the flush already ran synchronously inside the handler.
    if let Ok(resp) = mismatch {
        assert!(
            matches!(resp, DaemonResponse::VersionMismatch { .. }),
            "expected VersionMismatch, got {resp:?}"
        );
    }

    let count = wait_for_count(&cache_dir, "gs", 1);
    assert_eq!(
        count, 1,
        "version-mismatch shutdown should flush the buffered gs count to disk"
    );
}
