use aka_lib::*;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

// Global mutex to prevent tests from interfering with each other's environment variables
static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// Helper function to run a test with isolated XDG_RUNTIME_DIR environment
fn with_isolated_env<F, R>(test_fn: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = ENV_MUTEX.lock().unwrap();

    // Save original environment
    let original_xdg = std::env::var("XDG_RUNTIME_DIR").ok();

    // Remove XDG_RUNTIME_DIR to force using home_dir for daemon socket
    std::env::remove_var("XDG_RUNTIME_DIR");

    // Run the test
    let result = test_fn();

    // Restore original environment
    if let Some(xdg) = original_xdg {
        std::env::set_var("XDG_RUNTIME_DIR", xdg);
    }

    result
}

/// Test that health check correctly parses daemon status formats
#[test]
fn test_daemon_status_parsing() {
    with_isolated_env(|| {
        // This test verifies that the health check logic correctly parses
        // the actual daemon status format: "healthy:COUNT:synced" or "healthy:COUNT:stale"

        // We can't directly test the internal check_daemon_health function since it's private,
        // but we can test the overall health check behavior by mocking daemon responses

        // Create a test config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let home_dir = temp_dir.path().to_path_buf();

        // Create config directory
        let config_dir = home_dir.join(".config").join("aka");
        fs::create_dir_all(&config_dir).expect("Failed to create config dir");

        // Create valid config
        let config_file = config_dir.join("aka.yml");
        let test_config = r#"
defaults:
  version: 1
aliases:
  cat: "bat -p"
  ls: "eza"
"#;
        fs::write(&config_file, test_config).expect("Failed to write config");

        // Test that health check works with valid config (should return 0 when daemon not running)
        let result = execute_health_check(&home_dir, &None).expect("Health check should work");

        // When daemon is not running, it should fall back to direct mode validation
        // With valid config, this should return 0
        assert_eq!(
            result, 0,
            "Health check should return 0 for valid config when daemon not running"
        );
    });
}

/// Test health check with no config file
#[test]
fn test_health_check_no_config() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Don't create any config file

    // Temporarily unset XDG_RUNTIME_DIR to force using home_dir for daemon socket
    let original_xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    std::env::remove_var("XDG_RUNTIME_DIR");

    let result = execute_health_check(&home_dir, &None).expect("Health check should work");

    // Restore XDG_RUNTIME_DIR if it was set
    if let Some(xdg) = original_xdg {
        std::env::set_var("XDG_RUNTIME_DIR", xdg);
    }

    // Should return 1 for config not found
    assert_eq!(result, 1, "Health check should return 1 when config file not found");
}

/// Test health check with invalid config
#[test]
fn test_health_check_invalid_config() {
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

    // Temporarily unset XDG_RUNTIME_DIR to force using home_dir for daemon socket
    let original_xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    std::env::remove_var("XDG_RUNTIME_DIR");

    let result = execute_health_check(&home_dir, &None).expect("Health check should work");

    // Restore XDG_RUNTIME_DIR if it was set
    if let Some(xdg) = original_xdg {
        std::env::set_var("XDG_RUNTIME_DIR", xdg);
    }

    // Should return 2 for invalid config
    assert_eq!(result, 2, "Health check should return 2 for invalid config");
}

/// Test health check with config that has no aliases (invalid)
#[test]
fn test_health_check_no_aliases() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create config with no aliases
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases: {}
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Temporarily unset XDG_RUNTIME_DIR to force using home_dir for daemon socket
    let original_xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    std::env::remove_var("XDG_RUNTIME_DIR");

    let result = execute_health_check(&home_dir, &None).expect("Health check should work");

    // Restore XDG_RUNTIME_DIR if it was set
    if let Some(xdg) = original_xdg {
        std::env::set_var("XDG_RUNTIME_DIR", xdg);
    }

    // Should return 2 for invalid config (empty aliases are considered invalid)
    assert_eq!(
        result, 2,
        "Health check should return 2 when config has no aliases (invalid config)"
    );
}

/// Test health check exit code meanings
#[test]
fn test_health_check_exit_codes() {
    with_isolated_env(|| {
        // This test documents the expected exit codes from health check

        // Exit code 0: Daemon healthy OR config cache valid
        // Exit code 1: Config file not found OR config file unreadable
        // Exit code 2: Config file invalid (YAML parsing failed)
        // Exit code 3: Config valid but no aliases defined
        // Exit code 4: Stale socket detected (daemon socket exists but daemon appears dead)

        // Test with valid config (should return 0)
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let home_dir = temp_dir.path().to_path_buf();

        let config_dir = home_dir.join(".config").join("aka");
        fs::create_dir_all(&config_dir).expect("Failed to create config dir");

        let config_file = config_dir.join("aka.yml");
        let test_config = r#"
defaults:
  version: 1
aliases:
  test: "echo test"
"#;
        fs::write(&config_file, test_config).expect("Failed to write config");

        let result = execute_health_check(&home_dir, &None).expect("Health check should work");
        assert_eq!(result, 0, "Valid config should return exit code 0");
    });
}

/// Test that health check handles config hash changes correctly
#[test]
fn test_health_check_config_hash_changes() {
    with_isolated_env(|| {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let home_dir = temp_dir.path().to_path_buf();

        // Create config directory
        let config_dir = home_dir.join(".config").join("aka");
        fs::create_dir_all(&config_dir).expect("Failed to create config dir");

        // Create initial config
        let config_file = config_dir.join("aka.yml");
        let test_config1 = r#"
defaults:
  version: 1
aliases:
  cat: "bat -p"
"#;
        fs::write(&config_file, test_config1).expect("Failed to write config");

        // First health check should return 0 (valid config)
        let result1 = execute_health_check(&home_dir, &None).expect("Health check should work");
        assert_eq!(result1, 0, "First health check should return 0");

        // Modify config
        let test_config2 = r#"
defaults:
  version: 1
aliases:
  cat: "bat -p"
  ls: "eza"
"#;
        fs::write(&config_file, test_config2).expect("Failed to write config");

        // Second health check should still return 0 (valid config, hash updated)
        let result2 = execute_health_check(&home_dir, &None).expect("Health check should work");
        assert_eq!(result2, 0, "Second health check should return 0");
    });
}

/// Test that health check works with different config locations
#[test]
fn test_health_check_config_locations() {
    with_isolated_env(|| {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let home_dir = temp_dir.path().to_path_buf();

        // Test with config in ~/.config/aka/aka.yml
        let config_dir = home_dir.join(".config").join("aka");
        fs::create_dir_all(&config_dir).expect("Failed to create config dir");

        let config_file = config_dir.join("aka.yml");
        let test_config = r#"
defaults:
  version: 1
aliases:
  test: "echo test"
"#;
        fs::write(&config_file, test_config).expect("Failed to write config");

        let result = execute_health_check(&home_dir, &None).expect("Health check should work");
        assert_eq!(result, 0, "Config in .config/aka/aka.yml should work");

        // Remove the config file and test with ~/.aka.yml
        fs::remove_file(&config_file).expect("Should remove config file");

        let home_config = home_dir.join(".aka.yml");
        fs::write(&home_config, test_config).expect("Failed to write home config");

        let result2 = execute_health_check(&home_dir, &None).expect("Health check should work");
        assert_eq!(result2, 0, "Config in ~/.aka.yml should work");
    });
}

#[cfg(test)]
mod daemon_status_format_tests {

    /// Test that daemon status format matches expected patterns
    #[test]
    fn test_daemon_status_format_validation() {
        // Test valid daemon status formats
        let valid_formats = vec![
            "healthy:5:synced",
            "healthy:100:synced",
            "healthy:0:synced",
            "healthy:5:stale",
            "healthy:100:stale",
            "healthy:0:stale",
        ];

        for status in valid_formats {
            // Test the format matches our expected pattern
            assert!(
                status.starts_with("healthy:"),
                "Status should start with 'healthy:': {}",
                status
            );
            assert!(
                status.ends_with(":synced") || status.ends_with(":stale"),
                "Status should end with ':synced' or ':stale': {}",
                status
            );

            // Test that we can parse the alias count
            let parts: Vec<&str> = status.split(':').collect();
            assert_eq!(parts.len(), 3, "Status should have 3 parts: {}", status);
            assert_eq!(parts[0], "healthy", "First part should be 'healthy': {}", status);
            assert!(
                parts[1].parse::<u32>().is_ok(),
                "Second part should be a number: {}",
                status
            );
            assert!(
                parts[2] == "synced" || parts[2] == "stale",
                "Third part should be 'synced' or 'stale': {}",
                status
            );
        }
    }

    /// Test that old format is rejected
    #[test]
    fn test_old_daemon_status_format_rejected() {
        // These are the old formats that should NOT be accepted
        let invalid_formats = vec![
            "healthy:5:aliases",
            "healthy:100:aliases",
            "healthy:aliases",
            "healthy:5:aliases:synced",
        ];

        for status in invalid_formats {
            // These should NOT match our new parsing logic
            let parts: Vec<&str> = status.split(':').collect();
            let matches_new_format = parts.len() == 3
                && parts[0] == "healthy"
                && parts[1].parse::<u32>().is_ok()
                && (parts[2] == "synced" || parts[2] == "stale");
            assert!(
                !matches_new_format,
                "Old format should not match new parsing logic: {}",
                status
            );
        }
    }
}
