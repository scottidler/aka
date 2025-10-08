use std::fs;
use tempfile::TempDir;

mod common;
use common::*;

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

    let result = run_aka_command(&["ls"], Some(&temp_dir), Some(&config_file));

    if !result.success {
        panic!("aka ls failed: {}", result.stderr);
    }

    let lines: Vec<&str> = result.stdout.trim().split('\n').collect();

    // Should have 3 alias lines + 1 empty line + 1 count line = 5 lines total
    assert_eq!(lines.len(), 5, "Should have 3 aliases + empty line + count line");

    // Check that all 3 aliases are present
    assert!(result.stdout.contains("test1 -> echo test1"));
    assert!(result.stdout.contains("test2 -> echo test2"));
    assert!(result.stdout.contains("test3 -> echo test3"));

    // Check that the last line is the count line
    assert!(
        lines[lines.len() - 1].starts_with("count: "),
        "Last line should be count line"
    );
    assert!(lines[lines.len() - 1].contains("3"), "Count should be 3 for 3 aliases");
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

    let result = run_aka_command(&["freq", "--all"], Some(&temp_dir), Some(&config_file));

    if !result.success {
        panic!("aka freq --all failed: {}", result.stderr);
    }

    let lines: Vec<&str> = result.stdout.trim().split('\n').collect();

    // Should have 2 alias lines + 1 empty line + 1 count line = 4 lines total
    assert_eq!(lines.len(), 4, "Should have 2 aliases + empty line + count line");

    // Check that both aliases are present
    assert!(result.stdout.contains("freq1"));
    assert!(result.stdout.contains("freq2"));

    // Check that the last line is the count line
    assert!(
        lines[lines.len() - 1].starts_with("count: "),
        "Last line should be count line"
    );
    assert!(lines[lines.len() - 1].contains("2"), "Count should be 2 for 2 aliases");
}

#[test]
fn test_commands_have_consistent_count_formatting() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("aka.yml");

    // Write test config with known number of aliases
    let config_content = r#"
aliases:
  consistent1:
    value: "echo consistent1"
    global: false
  consistent2:
    value: "echo consistent2"
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    // Test ls command count
    let ls_result = run_aka_command(&["ls"], Some(&temp_dir), Some(&config_file));

    if !ls_result.success {
        panic!("aka ls failed: {}", ls_result.stderr);
    }

    // Test freq command count
    let freq_result = run_aka_command(&["freq", "--all"], Some(&temp_dir), Some(&config_file));

    if !freq_result.success {
        panic!("aka freq --all failed: {}", freq_result.stderr);
    }

    // Both should have the same count line format and value
    let ls_lines: Vec<&str> = ls_result.stdout.trim().split('\n').collect();
    let freq_lines: Vec<&str> = freq_result.stdout.trim().split('\n').collect();

    let ls_count_line = ls_lines.last().unwrap();
    let freq_count_line = freq_lines.last().unwrap();

    // Both should start with "count: " and contain "2"
    assert!(
        ls_count_line.starts_with("count: "),
        "ls count line should start with 'count: '"
    );
    assert!(
        freq_count_line.starts_with("count: "),
        "freq count line should start with 'count: '"
    );
    assert!(ls_count_line.contains("2"), "ls count should be 2");
    assert!(freq_count_line.contains("2"), "freq count should be 2");
}
