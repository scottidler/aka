use std::fs;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

fn get_aka_binary_path() -> String {
    let output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&output.stderr));
    }

    "target/debug/aka".to_string()
}

fn start_daemon_with_config(config_path: &str) -> std::process::Child {
    let daemon_output = Command::new("cargo")
        .args(&["build", "--bin", "aka-daemon"])
        .output()
        .expect("Failed to build aka-daemon binary");

    if !daemon_output.status.success() {
        panic!("Failed to build aka-daemon binary: {}", String::from_utf8_lossy(&daemon_output.stderr));
    }

    Command::new("target/debug/aka-daemon")
        .args(&["--config", config_path, "--foreground"])
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")  // Direct test logs to /tmp
        .env("AKA_CACHE_DIR", "/tmp/aka-test-cache")  // Direct test cache to /tmp
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")   // Isolate daemon socket
        .spawn()
        .expect("Failed to start daemon")
}

fn stop_daemon() {
    let _ = Command::new("pkill")
        .arg("aka-daemon")
        .output();

    // Wait a bit for cleanup
    std::thread::sleep(Duration::from_millis(100));
}

fn run_aka_command(config_path: &str, args: &[&str]) -> (String, String, i32) {
    let aka_binary = get_aka_binary_path();
    let mut cmd = Command::new(&aka_binary);
    cmd.args(&["--config", config_path]);
    cmd.args(args);
    cmd.env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log");  // Direct test logs to /tmp
    cmd.env("AKA_CACHE_DIR", "/tmp/aka-test-cache");  // Direct test cache to /tmp
    cmd.env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime");   // Isolate daemon socket

    let output = cmd.output().expect("Failed to run aka command");

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1)
    )
}

fn create_test_config() -> String {
    r#"
aliases:
  simple:
    value: "echo simple"
    global: false
  with_args:
    value: "echo hello"
    global: false
  positional:
    value: "echo Hello $1"
    global: false
  variadic:
    value: "echo $@"
    global: false
  global_alias:
    value: "echo global"
    global: true
  freq_test1:
    value: "echo freq1"
    global: false
  freq_test2:
    value: "echo freq2"
    global: false
"#.to_string()
}

#[test]
fn test_query_command_equivalence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");
    fs::write(&config_file, create_test_config()).expect("Failed to write config");
    let config_path = config_file.to_str().unwrap();

    // Test cases for query command
    let test_cases = vec![
        "simple",
        "with_args world",
        "positional World",
        "variadic hello world",
        "nonexistent",
        "",
    ];

    // Stop any existing daemon
    stop_daemon();

    for test_case in &test_cases {
        println!("Testing query: '{}'", test_case);

        // Test direct mode (no daemon)
        let (direct_stdout, _, direct_code) = run_aka_command(config_path, &["query", test_case]);

        // Start daemon and test daemon mode
        let mut daemon = start_daemon_with_config(config_path);
        std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start

        let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, &["query", test_case]);

        // Stop daemon
        daemon.kill().expect("Failed to kill daemon");
        stop_daemon();

        // Assert equivalence
        assert_eq!(direct_stdout, daemon_stdout,
            "Query '{}' stdout differs between direct and daemon mode", test_case);
        assert_eq!(direct_code, daemon_code,
            "Query '{}' exit code differs between direct and daemon mode", test_case);
    }
}

#[test]
fn test_list_command_equivalence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");
    fs::write(&config_file, create_test_config()).expect("Failed to write config");
    let config_path = config_file.to_str().unwrap();

    // Test cases for list command
    let test_cases = vec![
        vec!["ls"],
        vec!["ls", "--global"],
        vec!["ls", "simple"],
        vec!["ls", "freq"],
        vec!["ls", "nonexistent"],
    ];

    // Stop any existing daemon
    stop_daemon();

    for test_case in &test_cases {
        println!("Testing list: {:?}", test_case);

        // Test direct mode (no daemon)
        let (direct_stdout, _, direct_code) = run_aka_command(config_path, test_case);

        // Start daemon and test daemon mode
        let mut daemon = start_daemon_with_config(config_path);
        std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start

        let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, test_case);

        // Stop daemon
        daemon.kill().expect("Failed to kill daemon");
        stop_daemon();

        // Assert equivalence
        assert_eq!(direct_stdout, daemon_stdout,
            "List {:?} stdout differs between direct and daemon mode", test_case);
        assert_eq!(direct_code, daemon_code,
            "List {:?} exit code differs between direct and daemon mode", test_case);
    }
}

#[test]
fn test_freq_command_equivalence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");
    fs::write(&config_file, create_test_config()).expect("Failed to write config");
    let config_path = config_file.to_str().unwrap();

    // Test cases for freq command
    let test_cases = vec![
        vec!["freq"],
        vec!["freq", "--all"],
    ];

    // Stop any existing daemon
    stop_daemon();

    for test_case in &test_cases {
        println!("Testing freq: {:?}", test_case);

        // Test direct mode (no daemon)
        let (direct_stdout, _, direct_code) = run_aka_command(config_path, test_case);

        // Start daemon and test daemon mode
        let mut daemon = start_daemon_with_config(config_path);
        std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start

        let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, test_case);

        // Stop daemon
        daemon.kill().expect("Failed to kill daemon");
        stop_daemon();

        // Assert equivalence
        assert_eq!(direct_stdout, daemon_stdout,
            "Freq {:?} stdout differs between direct and daemon mode", test_case);
        assert_eq!(direct_code, daemon_code,
            "Freq {:?} exit code differs between direct and daemon mode", test_case);
    }
}

#[test]
fn test_eol_parameter_equivalence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");
    fs::write(&config_file, create_test_config()).expect("Failed to write config");
    let config_path = config_file.to_str().unwrap();

    // Test EOL parameter with query command
    let test_cases = vec![
        ("simple", false),
        ("simple", true),
        ("positional World", false),
        ("positional World", true),
    ];

    // Stop any existing daemon
    stop_daemon();

    for (query, eol) in &test_cases {
        println!("Testing query '{}' with eol={}", query, eol);

        let mut direct_args = vec!["query"];
        if *eol {
            direct_args.push("--eol");
        }
        direct_args.push(query);

        // Test direct mode (no daemon)
        let (direct_stdout, _, direct_code) = run_aka_command(config_path, &direct_args);

        // Start daemon and test daemon mode
        let mut daemon = start_daemon_with_config(config_path);
        std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start

        let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, &direct_args);

        // Stop daemon
        daemon.kill().expect("Failed to kill daemon");
        stop_daemon();

        // Assert equivalence
        assert_eq!(direct_stdout, daemon_stdout,
            "Query '{}' with eol={} stdout differs between direct and daemon mode", query, eol);
        assert_eq!(direct_code, daemon_code,
            "Query '{}' with eol={} exit code differs between direct and daemon mode", query, eol);
    }
}

#[test]
fn test_custom_config_equivalence() {
    // Create two different config files
    let temp_dir1 = TempDir::new().expect("Failed to create temp dir 1");
    let config_file1 = temp_dir1.path().join("aka1.yml");
    fs::write(&config_file1, r#"
aliases:
  test1:
    value: "echo config1"
    global: false
"#).expect("Failed to write config 1");

    let temp_dir2 = TempDir::new().expect("Failed to create temp dir 2");
    let config_file2 = temp_dir2.path().join("aka2.yml");
    fs::write(&config_file2, r#"
aliases:
  test2:
    value: "echo config2"
    global: false
"#).expect("Failed to write config 2");

    let config_path1 = config_file1.to_str().unwrap();
    let config_path2 = config_file2.to_str().unwrap();

    // Test that different configs produce different results but are consistent between modes
    let test_cases = vec![
        (config_path1, "test1"),
        (config_path1, "test2"), // Should fail
        (config_path2, "test1"), // Should fail
        (config_path2, "test2"),
    ];

    // Stop any existing daemon
    stop_daemon();

    for (config_path, query) in &test_cases {
        println!("Testing config '{}' with query '{}'", config_path, query);

        // Test direct mode (no daemon)
        let (direct_stdout, _, direct_code) = run_aka_command(config_path, &["query", query]);

        // Start daemon and test daemon mode
        let mut daemon = start_daemon_with_config(config_path);
        std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start

        let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, &["query", query]);

        // Stop daemon
        daemon.kill().expect("Failed to kill daemon");
        stop_daemon();

        // Assert equivalence
        assert_eq!(direct_stdout, daemon_stdout,
            "Config '{}' query '{}' stdout differs between direct and daemon mode", config_path, query);
        assert_eq!(direct_code, daemon_code,
            "Config '{}' query '{}' exit code differs between direct and daemon mode", config_path, query);
    }
}

#[test]
fn test_error_handling_equivalence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");
    fs::write(&config_file, "invalid yaml content [[[").expect("Failed to write config");
    let config_path = config_file.to_str().unwrap();

    // Stop any existing daemon
    stop_daemon();

    println!("Testing error handling with invalid config");

    // Test direct mode (no daemon)
    let (_, _, direct_code) = run_aka_command(config_path, &["query", "test"]);

    // Start daemon and test daemon mode (daemon should fail to start or handle gracefully)
    let mut daemon = start_daemon_with_config(config_path);
    std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start (or fail)

    let (_, _, daemon_code) = run_aka_command(config_path, &["query", "test"]);

    // Stop daemon
    daemon.kill().expect("Failed to kill daemon");
    stop_daemon();

    // Both should fail, but consistently
    assert!(direct_code != 0, "Direct mode should fail with invalid config");
    assert!(daemon_code != 0, "Daemon mode should fail with invalid config");

    // The exact error messages might differ slightly, but both should indicate failure
    println!("Direct mode error code: {}", direct_code);
    println!("Daemon mode error code: {}", daemon_code);
}

#[test]
fn test_performance_consistency() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");

    // Create a larger config for performance testing
    let mut large_config = String::from("aliases:\n");
    for i in 0..100 {
        large_config.push_str(&format!(
            "  alias{}:\n    value: \"echo alias{}\"\n    global: false\n",
            i, i
        ));
    }

    fs::write(&config_file, large_config).expect("Failed to write config");
    let config_path = config_file.to_str().unwrap();

    // Stop any existing daemon
    stop_daemon();

    // Test multiple operations to ensure consistency
    let operations = vec![
        vec!["ls"],
        vec!["freq", "--all"],
        vec!["query", "alias50"],
    ];

    for operation in &operations {
        println!("Testing performance consistency for {:?}", operation);

        // Test direct mode
        let start = std::time::Instant::now();
        let (direct_stdout, _, direct_code) = run_aka_command(config_path, operation);
        let direct_duration = start.elapsed();

        // Start daemon and test daemon mode
        let mut daemon = start_daemon_with_config(config_path);
        std::thread::sleep(Duration::from_millis(500)); // Wait for daemon to start

        let start = std::time::Instant::now();
        let (daemon_stdout, _, daemon_code) = run_aka_command(config_path, operation);
        let daemon_duration = start.elapsed();

        // Stop daemon
        daemon.kill().expect("Failed to kill daemon");
        stop_daemon();

        // Assert functional equivalence (performance can differ)
        assert_eq!(direct_stdout, daemon_stdout,
            "Operation {:?} stdout differs between direct and daemon mode", operation);
        assert_eq!(direct_code, daemon_code,
            "Operation {:?} exit code differs between direct and daemon mode", operation);

        println!("Direct mode: {:?}, Daemon mode: {:?}", direct_duration, daemon_duration);
    }
}