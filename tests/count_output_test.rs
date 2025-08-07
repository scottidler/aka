use std::fs;
use std::process::Command;
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

#[test]
fn test_ls_command_shows_count() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");

    // Write test config with 3 aliases
    let config_content = r#"
aliases:
  test1:
    value: "echo test1"
    global: false
  test2:
    value: "echo test2"
    global: false
  test3:
    value: "echo test3"
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    let aka_binary = get_aka_binary_path();
    let output = Command::new(&aka_binary)
        .args(&["--config", config_file.to_str().unwrap(), "ls"])
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")
        .output()
        .expect("Failed to run aka ls");

    if !output.status.success() {
        panic!("aka ls failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().split('\n').collect();

    // Should have 3 alias lines + 1 empty line + 1 count line = 5 lines total
    assert_eq!(lines.len(), 5, "Should have 3 aliases + empty line + count line");

    // Check that all 3 aliases are present
    assert!(stdout.contains("test1 -> echo test1"));
    assert!(stdout.contains("test2 -> echo test2"));
    assert!(stdout.contains("test3 -> echo test3"));

    // Check that the last line is the count line
    assert!(lines[lines.len()-1].starts_with("count: "), "Last line should be count line");
    assert!(lines[lines.len()-1].contains("3"), "Count should be 3");

    // Check that there's an empty line before the count
    assert_eq!(lines[lines.len()-2], "", "Second to last line should be empty");
}

#[test]
fn test_freq_command_shows_count() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");

    // Write test config with 2 aliases
    let config_content = r#"
aliases:
  freq1:
    value: "echo freq1"
    global: false
  freq2:
    value: "echo freq2"
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    let aka_binary = get_aka_binary_path();
    let output = Command::new(&aka_binary)
        .args(&["--config", config_file.to_str().unwrap(), "freq", "--all"])
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")
        .output()
        .expect("Failed to run aka freq");

    if !output.status.success() {
        panic!("aka freq failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().split('\n').collect();

    // Should have 2 alias lines + 1 empty line + 1 count line = 4 lines total
    assert_eq!(lines.len(), 4, "Should have 2 aliases + empty line + count line");

    // Check that both aliases are present with count 0
    assert!(stdout.contains("0 freq1 -> echo freq1"));
    assert!(stdout.contains("0 freq2 -> echo freq2"));

    // Check that the last line is the count line
    assert!(lines[lines.len()-1].starts_with("count: "), "Last line should be count line");
    assert!(lines[lines.len()-1].contains("2"), "Count should be 2");

    // Check that there's an empty line before the count
    assert_eq!(lines[lines.len()-2], "", "Second to last line should be empty");
}

#[test]
fn test_count_output_consistency_between_modes() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");

    // Write test config
    let config_content = r#"
aliases:
  consistency:
    value: "echo consistency"
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    let aka_binary = get_aka_binary_path();

    // Test ls command
    let ls_output = Command::new(&aka_binary)
        .args(&["--config", config_file.to_str().unwrap(), "ls"])
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")
        .output()
        .expect("Failed to run aka ls");

    // Test freq command
    let freq_output = Command::new(&aka_binary)
        .args(&["--config", config_file.to_str().unwrap(), "freq", "--all"])
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime")
        .output()
        .expect("Failed to run aka freq");

    if !ls_output.status.success() {
        panic!("aka ls failed: {}", String::from_utf8_lossy(&ls_output.stderr));
    }

    if !freq_output.status.success() {
        panic!("aka freq failed: {}", String::from_utf8_lossy(&freq_output.stderr));
    }

    let ls_stdout = String::from_utf8_lossy(&ls_output.stdout);
    let freq_stdout = String::from_utf8_lossy(&freq_output.stdout);

    // Both should end with the same count
    assert!(ls_stdout.ends_with("count: 1\n"), "ls should end with count: 1");
    assert!(freq_stdout.ends_with("count: 1\n"), "freq should end with count: 1");

    // Both should have empty line before count
    assert!(ls_stdout.contains("\n\ncount: 1"), "ls should have empty line before count");
    assert!(freq_stdout.contains("\n\ncount: 1"), "freq should have empty line before count");
}