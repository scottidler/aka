use std::fs;
use std::process::Command;
use tempfile::TempDir;
use std::path::PathBuf;

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

fn get_aka_binary_path() -> PathBuf {
    let mut path = std::env::current_dir().expect("Failed to get current dir");
    path.push("target");
    path.push("debug");
    path.push("aka");
    path
}

#[test]
fn test_freq_command_basic() {
    let (temp_dir, _config_file) = setup_test_environment_with_usage();
    let aka_binary = get_aka_binary_path();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !build_output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&build_output.stderr));
    }

    // Set HOME to our temp directory and ensure no daemon socket exists
    let output = Command::new(&aka_binary)
        .args(&["freq"])
        .env("HOME", temp_dir.path())
        .env("XDG_RUNTIME_DIR", temp_dir.path().join("run"))
        .output()
        .expect("Failed to run aka freq");

    if !output.status.success() {
        panic!("aka freq failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // By default, should only show used aliases (count > 0)
    // Since all aliases have count 0, should show "No aliases found."
    assert!(stdout.contains("No aliases found."), "Should show 'No aliases found.' when no aliases are used");
}

#[test]
fn test_freq_command_with_all_option() {
    let (temp_dir, _config_file) = setup_test_environment_with_usage();
    let aka_binary = get_aka_binary_path();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !build_output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&build_output.stderr));
    }

    // Test with --all to show all aliases including unused ones
    let output = Command::new(&aka_binary)
        .args(&["freq", "--all"])
        .env("HOME", temp_dir.path())
        .env("XDG_RUNTIME_DIR", temp_dir.path().join("run"))
        .output()
        .expect("Failed to run aka freq --all");

    if !output.status.success() {
        panic!("aka freq --all failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain all aliases with count 0 (sorted alphabetically)
    assert!(stdout.contains("test-high"));
    assert!(stdout.contains("test-medium"));
    assert!(stdout.contains("test-low"));
    assert!(stdout.contains("test-unused"));

    // All should have count 0
    assert!(stdout.contains("0"));

    // Should be formatted with proper spacing
    let lines: Vec<&str> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 4, "Should have 4 aliases with --all");

    // Check that lines are properly formatted (count alias -> value)
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert!(parts.len() >= 4, "Each line should have at least 4 parts: count, alias, ->, value");
        assert_eq!(parts[0], "0", "Count should be 0 for unused aliases");
        assert_eq!(parts[2], "->", "Should have -> separator");
    }
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

    let aka_binary = get_aka_binary_path();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !build_output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&build_output.stderr));
    }

    let output = Command::new(&aka_binary)
        .args(&["freq"])
        .env("HOME", temp_dir.path())
        .env("XDG_RUNTIME_DIR", temp_dir.path().join("run"))
        .output()
        .expect("Failed to run aka freq");

    if !output.status.success() {
        panic!("aka freq failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // By default, should only show used aliases (count > 0)
    // Since dummy alias has count 0, should show "No aliases found."
    assert!(stdout.contains("No aliases found."), "Should show 'No aliases found.' when no aliases are used");
}

#[test]
fn test_freq_command_help() {
    let aka_binary = get_aka_binary_path();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !build_output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&build_output.stderr));
    }

    let output = Command::new(&aka_binary)
        .args(&["freq", "--help"])
        .output()
        .expect("Failed to run aka freq --help");

    if !output.status.success() {
        panic!("aka freq --help failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain help information
    assert!(stdout.contains("show alias usage frequency statistics"), "Should contain description");
    assert!(stdout.contains("--all"), "Should contain --all option");
    assert!(stdout.contains("show all aliases including unused ones"), "Should contain --all description");
}

#[test]
fn test_freq_command_in_main_help() {
    let aka_binary = get_aka_binary_path();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !build_output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&build_output.stderr));
    }

    let output = Command::new(&aka_binary)
        .args(&["--help"])
        .output()
        .expect("Failed to run aka --help");

    if !output.status.success() {
        panic!("aka --help failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain the freq command in the main help
    assert!(stdout.contains("freq"), "Should contain 'freq' command in main help");
    assert!(stdout.contains("show alias usage frequency statistics"), "Should contain freq description");
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

    let aka_binary = get_aka_binary_path();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--bin", "aka"])
        .output()
        .expect("Failed to build aka binary");

    if !build_output.status.success() {
        panic!("Failed to build aka binary: {}", String::from_utf8_lossy(&build_output.stderr));
    }

    // Use the alias a few times to increment its count
    for _ in 0..3 {
        let output = Command::new(&aka_binary)
            .args(&["query", "test-alias"])
            .env("HOME", temp_dir.path())
            .env("XDG_RUNTIME_DIR", temp_dir.path().join("run"))
            .output()
            .expect("Failed to run aka query");

        if !output.status.success() {
            panic!("aka query failed: {}", String::from_utf8_lossy(&output.stderr));
        }
    }

    // Now run freq to see the usage count
    let output = Command::new(&aka_binary)
        .args(&["freq"])
        .env("HOME", temp_dir.path())
        .env("XDG_RUNTIME_DIR", temp_dir.path().join("run"))
        .output()
        .expect("Failed to run aka freq");

    if !output.status.success() {
        panic!("aka freq failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show the alias with count 3
    assert!(stdout.contains("test-alias"), "Should contain test-alias");
    assert!(stdout.contains("3"), "Should show count of 3");
    assert!(stdout.contains("echo \"test\""), "Should show the alias value");
}
