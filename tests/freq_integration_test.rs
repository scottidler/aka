use std::fs;
use tempfile::TempDir;
use std::path::PathBuf;

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
    assert!(result.stdout.contains("No aliases found."), "Should show 'No aliases found.' when no aliases are used");
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
    assert_eq!(lines.len(), 6, "Should have 4 aliases + empty line + count line with --all");

    // Check that lines are properly formatted (count alias -> value)
    // Skip the last 2 lines (empty line and count line)
    for line in &lines[..lines.len()-2] {
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert!(parts.len() >= 4, "Each line should have at least 4 parts: count, alias, ->, value");
        assert_eq!(parts[0], "0", "Count should be 0 for unused aliases");
        assert_eq!(parts[2], "->", "Should have -> separator");
    }

    // Check that the last line is the count line
    assert!(lines[lines.len()-1].starts_with("count: "), "Last line should be count line");
    assert!(lines[lines.len()-1].contains("4"), "Count should be 4 for 4 aliases");
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
    assert!(result.stdout.contains("No aliases found."), "Should show 'No aliases found.' when no aliases are used");
}

#[test]
fn test_freq_command_help() {
    let result = run_aka_command(&["freq", "--help"], None, None);

    if !result.success {
        panic!("aka freq --help failed: {}", result.stderr);
    }

    // Should contain help information
    assert!(result.stdout.contains("show alias usage frequency statistics"), "Should contain description");
    assert!(result.stdout.contains("--all"), "Should contain --all option");
    assert!(result.stdout.contains("show all aliases including unused ones"), "Should contain --all description");
}

#[test]
fn test_freq_command_in_main_help() {
    let result = run_aka_command(&["--help"], None, None);

    if !result.success {
        panic!("aka --help failed: {}", result.stderr);
    }

    // Should contain the freq command in the main help
    assert!(result.stdout.contains("freq"), "Should contain 'freq' command in main help");
    assert!(result.stdout.contains("show alias usage frequency statistics"), "Should contain freq description");
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

    // Use the alias a few times to increment its count
    for _ in 0..3 {
        let result = run_aka_command(&["query", "test-alias"], Some(&temp_dir), None);

        if !result.success {
            panic!("aka query failed: {}", result.stderr);
        }
    }

    // Now run freq to see the usage count
    let result = run_aka_command(&["freq"], Some(&temp_dir), None);

    if !result.success {
        panic!("aka freq failed: {}", result.stderr);
    }

    // Should show the alias with count 3
    assert!(result.stdout.contains("test-alias"), "Should contain test-alias");
    assert!(result.stdout.contains("3"), "Should show count of 3");
    assert!(result.stdout.contains("echo \"test\""), "Should show the alias value");
}
