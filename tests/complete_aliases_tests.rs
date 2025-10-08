use aka_lib::*;
use std::fs;
use tempfile::TempDir;

/// Test the get_alias_names_for_completion function directly
#[test]
fn test_get_alias_names_for_completion() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create test config with various aliases
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  zz: "eza -la"
  cat: "bat -p"
  ls: "eza"
  grep: "rg"
  aa: "echo first"
  bb: "echo second"
lookups:
  region:
    prod: us-east-1
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Create AKA instance
    let aka = AKA::new(false, home_dir, config_file).expect("Failed to create AKA instance");

    // Test the function
    let alias_names = get_alias_names_for_completion(&aka);

    // Should be sorted alphabetically
    let expected = vec!["aa", "bb", "cat", "grep", "ls", "zz"];
    assert_eq!(alias_names, expected);
}

/// Test that empty config returns empty list
#[test]
fn test_get_alias_names_for_completion_empty() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create config with one alias that we'll ignore
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  dummy: "echo dummy"
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Create AKA instance and clear aliases manually to test empty case
    let mut aka = AKA::new(false, home_dir, config_file).expect("Failed to create AKA instance");
    aka.spec.aliases.clear();

    // Test the function
    let alias_names = get_alias_names_for_completion(&aka);

    // Should be empty
    assert_eq!(alias_names, Vec::<String>::new());
}

/// Test that function handles special characters in alias names
#[test]
fn test_get_alias_names_for_completion_special_chars() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create config with special characters
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  "ls-la": "eza -la"
  "git-log": "git log --oneline"
  "|c": "| xclip -sel clip"
  "!!": "sudo !!"
  "...": "cd ../.."
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Create AKA instance
    let aka = AKA::new(false, home_dir, config_file).expect("Failed to create AKA instance");

    // Test the function
    let alias_names = get_alias_names_for_completion(&aka);

    // Should be sorted alphabetically, including special characters
    let expected = vec!["!!", "...", "git-log", "ls-la", "|c"];
    assert_eq!(alias_names, expected);
}

/// Integration test for direct mode __complete_aliases command
#[test]
fn test_complete_aliases_direct_mode_integration() {
    use std::process::Command;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create test config
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  cat: "bat -p"
  ls: "eza"
  grep: "rg"
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Set environment variable to use our test config
    let output = Command::new("cargo")
        .args(&[
            "run",
            "-q",
            "--",
            "-c",
            config_file.to_str().unwrap(),
            "__complete_aliases",
        ])
        .env("HOME", home_dir.to_str().unwrap())
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("AKA_CACHE_DIR", "/tmp/aka-test-cache")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime") // Isolate daemon socket
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success(), "Command should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Output should be valid UTF-8");
    let lines: Vec<&str> = stdout.trim().split('\n').collect();

    // Should have 3 aliases, sorted alphabetically
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "cat");
    assert_eq!(lines[1], "grep");
    assert_eq!(lines[2], "ls");
}

/// Test that __complete_aliases works with no aliases
#[test]
fn test_complete_aliases_no_aliases() {
    use std::process::Command;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create config with one alias that we'll use for testing
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  dummy: "echo dummy"
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Set environment variable to use our test config
    let output = Command::new("cargo")
        .args(&[
            "run",
            "-q",
            "--",
            "-c",
            config_file.to_str().unwrap(),
            "__complete_aliases",
        ])
        .env("HOME", home_dir.to_str().unwrap())
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("AKA_CACHE_DIR", "/tmp/aka-test-cache")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime") // Isolate daemon socket
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success(), "Command should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Output should be valid UTF-8");

    // Should have one alias
    assert_eq!(stdout.trim(), "dummy");
}

/// Test that __complete_aliases handles invalid config gracefully
#[test]
fn test_complete_aliases_invalid_config() {
    use std::process::Command;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create invalid config
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  cat: "bat -p"
  # Invalid YAML - missing closing quote
  ls: "eza
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Set environment variable to use our test config
    let output = Command::new("cargo")
        .args(&[
            "run",
            "-q",
            "--",
            "-c",
            config_file.to_str().unwrap(),
            "__complete_aliases",
        ])
        .env("HOME", home_dir.to_str().unwrap())
        .env("AKA_LOG_FILE", "/tmp/aka-test-logs/aka.log")
        .env("AKA_CACHE_DIR", "/tmp/aka-test-cache")
        .env("XDG_RUNTIME_DIR", "/tmp/aka-test-runtime") // Isolate daemon socket
        .output()
        .expect("Failed to execute command");

    // Should fail with invalid config
    assert!(!output.status.success(), "Command should fail with invalid config");

    let stderr = String::from_utf8(output.stderr).expect("Error output should be valid UTF-8");
    assert!(stderr.contains("Error"), "Should contain error message");
}

#[cfg(test)]
mod daemon_tests {
    use aka_lib::protocol::DaemonRequest;
    use serde_json;

    /// Test that CompleteAliases request can be serialized/deserialized
    #[test]
    fn test_complete_aliases_protocol_serialization() {
        let request = DaemonRequest::CompleteAliases {
            version: "v0.5.0".to_string(),
            config: None,
        };

        // Test serialization
        let serialized = serde_json::to_string(&request).expect("Should serialize");
        assert!(serialized.contains("CompleteAliases"));

        // Test deserialization
        let deserialized: DaemonRequest = serde_json::from_str(&serialized).expect("Should deserialize");
        match deserialized {
            DaemonRequest::CompleteAliases { .. } => {} // Success
            _ => panic!("Wrong variant deserialized"),
        }
    }

    /// Test the protocol message format
    #[test]
    fn test_complete_aliases_protocol_format() {
        let request = DaemonRequest::CompleteAliases {
            version: "v0.5.0".to_string(),
            config: None,
        };
        let json = serde_json::to_string(&request).expect("Should serialize");

        // Should contain the type tag and version field
        assert!(json.contains("\"type\":\"CompleteAliases\""));
        assert!(json.contains("\"version\":\"v0.5.0\""));
    }
}
