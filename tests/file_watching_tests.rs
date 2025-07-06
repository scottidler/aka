use std::fs;
use std::time::Duration;
use std::thread;
use tempfile::TempDir;
use aka_lib::{AKA, get_config_path};

#[cfg(test)]
mod file_watching_tests {
    use super::*;

    const TEST_CONFIG_INITIAL: &str = r#"
lookups: {}

aliases:
  test-initial:
    value: echo "initial test"
    global: true

  test-local:
    value: echo "local test"
    global: false
"#;

    const TEST_CONFIG_UPDATED: &str = r#"
lookups: {}

aliases:
  test-initial:
    value: echo "initial test"
    global: true

  test-local:
    value: echo "local test"
    global: false

  test-new-alias:
    value: echo "new alias added"
    global: true

  test-another:
    value: echo "another new alias"
    global: false
"#;

    #[test]
    fn test_manual_config_reload() {
        // Create temporary config file
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("aka.yml");
        fs::write(&config_file, TEST_CONFIG_INITIAL).expect("Failed to write initial config");

        // Load initial config
        let mut aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to load initial config");
        assert_eq!(aka.spec.aliases.len(), 2);
        assert!(aka.spec.aliases.contains_key("test-initial"));
        assert!(aka.spec.aliases.contains_key("test-local"));

        // Update config file
        fs::write(&config_file, TEST_CONFIG_UPDATED).expect("Failed to write updated config");

        // Manually reload config
        aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to reload config");
        assert_eq!(aka.spec.aliases.len(), 4);
        assert!(aka.spec.aliases.contains_key("test-initial"));
        assert!(aka.spec.aliases.contains_key("test-local"));
        assert!(aka.spec.aliases.contains_key("test-new-alias"));
        assert!(aka.spec.aliases.contains_key("test-another"));
    }

    #[test]
    fn test_config_file_modification_detection() {
        // Create temporary config file
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("aka.yml");
        fs::write(&config_file, TEST_CONFIG_INITIAL).expect("Failed to write initial config");

        // Get initial modification time
        let initial_metadata = fs::metadata(&config_file).expect("Failed to get file metadata");
        let initial_modified = initial_metadata.modified().expect("Failed to get modification time");

        // Wait a bit to ensure different timestamp
        thread::sleep(Duration::from_millis(10));

        // Update config file
        fs::write(&config_file, TEST_CONFIG_UPDATED).expect("Failed to write updated config");

        // Check that modification time changed
        let updated_metadata = fs::metadata(&config_file).expect("Failed to get updated file metadata");
        let updated_modified = updated_metadata.modified().expect("Failed to get updated modification time");

        assert!(updated_modified > initial_modified, "File modification time should have changed");
    }

    #[test]
    fn test_config_validation_after_reload() {
        // Test that reloaded config is properly validated
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("aka.yml");

        // Write invalid config
        let invalid_config = r#"
invalid_yaml: [
  - missing_closing_bracket
"#;
        fs::write(&config_file, invalid_config).expect("Failed to write invalid config");

        // Attempt to load invalid config should fail
        let result = AKA::new(false, &Some(config_file.clone()));
        assert!(result.is_err(), "Loading invalid config should fail");

        // Write valid config
        fs::write(&config_file, TEST_CONFIG_INITIAL).expect("Failed to write valid config");

        // Should now load successfully
        let aka = AKA::new(false, &Some(config_file.clone()));
        assert!(aka.is_ok(), "Loading valid config should succeed");

        let aka = aka.expect("Config should load successfully after writing valid config");
        assert_eq!(aka.spec.aliases.len(), 2);
    }

    #[test]
    fn test_alias_functionality_after_reload() {
        // Test that aliases work correctly after config reload
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("aka.yml");
        fs::write(&config_file, TEST_CONFIG_INITIAL).expect("Failed to write initial config");

        // Load initial config and test alias
        let aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to load initial config");
        let result = aka.replace_with_mode("test-initial", aka_lib::ProcessingMode::Direct).expect("Failed to process alias");
        assert_eq!(result.trim(), "echo \"initial test\"");

        // Update config file with new alias
        fs::write(&config_file, TEST_CONFIG_UPDATED).expect("Failed to write updated config");

        // Reload config
        let aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to reload config");

        // Test that old alias still works
        let result = aka.replace_with_mode("test-initial", aka_lib::ProcessingMode::Direct).expect("Failed to process old alias");
        assert_eq!(result.trim(), "echo \"initial test\"");

        // Test that new alias works
        let result = aka.replace_with_mode("test-new-alias", aka_lib::ProcessingMode::Direct).expect("Failed to process new alias");
        assert_eq!(result.trim(), "echo \"new alias added\"");
    }

    #[test]
    fn test_get_config_path_function() {
        // Test that get_config_path function works
        let config_path = get_config_path();
        assert!(config_path.is_ok(), "get_config_path should succeed");

        let path = config_path.expect("get_config_path should return a valid path");
        assert!(path.to_string_lossy().contains("aka.yml"), "Config path should contain aka.yml");
    }

    #[test]
    fn test_alias_count_tracking() {
        // Test that alias count is properly tracked during reloads
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("aka.yml");

        // Initial config with 2 aliases
        fs::write(&config_file, TEST_CONFIG_INITIAL).expect("Failed to write initial config");
        let aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to load initial config");
        assert_eq!(aka.spec.aliases.len(), 2);

        // Updated config with 4 aliases
        fs::write(&config_file, TEST_CONFIG_UPDATED).expect("Failed to write updated config");
        let aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to reload config");
        assert_eq!(aka.spec.aliases.len(), 4);

        // Verify specific aliases exist
        assert!(aka.spec.aliases.contains_key("test-initial"));
        assert!(aka.spec.aliases.contains_key("test-local"));
        assert!(aka.spec.aliases.contains_key("test-new-alias"));
        assert!(aka.spec.aliases.contains_key("test-another"));
    }

    #[test]
    fn test_global_vs_local_aliases_after_reload() {
        // Test that global/local distinction is preserved after reload
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_file = temp_dir.path().join("aka.yml");
        fs::write(&config_file, TEST_CONFIG_UPDATED).expect("Failed to write config");

        let aka = AKA::new(false, &Some(config_file.clone())).expect("Failed to load config");

        // Check global aliases
        let global_aliases: Vec<_> = aka.spec.aliases.values()
            .filter(|alias| alias.global)
            .collect();
        assert_eq!(global_aliases.len(), 2); // test-initial and test-new-alias

        // Check local aliases
        let local_aliases: Vec<_> = aka.spec.aliases.values()
            .filter(|alias| !alias.global)
            .collect();
        assert_eq!(local_aliases.len(), 2); // test-local and test-another
    }
}

#[cfg(test)]
mod daemon_ipc_tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug)]
    #[serde(tag = "type")]
    enum TestRequest {
        ReloadConfig,
        Health,
    }

    #[derive(Serialize, Deserialize, Debug)]
    #[serde(tag = "type")]
    enum TestResponse {
        ConfigReloaded { success: bool, aliases_count: usize, message: String },
        Health { status: String },
        Error { message: String },
    }

    #[test]
    fn test_reload_config_request_serialization() {
        // Test that ReloadConfig request can be serialized/deserialized
        let request = TestRequest::ReloadConfig;
        let serialized = serde_json::to_string(&request).expect("Failed to serialize request");
        assert!(serialized.contains("ReloadConfig"));

        let deserialized: TestRequest = serde_json::from_str(&serialized).expect("Failed to deserialize request");
        match deserialized {
            TestRequest::ReloadConfig => {}, // Success
            _ => panic!("Wrong request type after deserialization"),
        }
    }

    #[test]
    fn test_config_reloaded_response_serialization() {
        // Test that ConfigReloaded response can be serialized/deserialized
        let response = TestResponse::ConfigReloaded {
            success: true,
            aliases_count: 42,
            message: "Config reloaded successfully".to_string(),
        };

        let serialized = serde_json::to_string(&response).expect("Failed to serialize response");
        assert!(serialized.contains("ConfigReloaded"));
        assert!(serialized.contains("42"));
        assert!(serialized.contains("Config reloaded successfully"));

        let deserialized: TestResponse = serde_json::from_str(&serialized).expect("Failed to deserialize response");
        match deserialized {
            TestResponse::ConfigReloaded { success, aliases_count, message } => {
                assert!(success);
                assert_eq!(aliases_count, 42);
                assert_eq!(message, "Config reloaded successfully");
            },
            _ => panic!("Wrong response type after deserialization"),
        }
    }

    #[test]
    fn test_health_request_response_cycle() {
        // Test health request/response serialization
        let request = TestRequest::Health;
        let serialized_request = serde_json::to_string(&request).expect("Failed to serialize health request");

        let response = TestResponse::Health { status: "healthy:10:aliases".to_string() };
        let serialized_response = serde_json::to_string(&response).expect("Failed to serialize health response");

        // Verify both can be deserialized
        let _: TestRequest = serde_json::from_str(&serialized_request).expect("Failed to deserialize health request");
        let _: TestResponse = serde_json::from_str(&serialized_response).expect("Failed to deserialize health response");
    }

    #[test]
    fn test_error_response_serialization() {
        // Test error response serialization
        let response = TestResponse::Error { message: "Test error message".to_string() };
        let serialized = serde_json::to_string(&response).expect("Failed to serialize error response");

        let deserialized: TestResponse = serde_json::from_str(&serialized).expect("Failed to deserialize error response");
        match deserialized {
            TestResponse::Error { message } => {
                assert_eq!(message, "Test error message");
            },
            _ => panic!("Wrong response type after deserialization"),
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    #[test]
    fn test_daemon_binary_exists() {
        // Test that the daemon binary can be built
        let output = Command::new("cargo")
            .args(&["build", "--bin", "aka-daemon"])
            .output();

        assert!(output.is_ok(), "Should be able to build aka-daemon");
        let output = output.expect("Should be able to execute cargo build command");
        assert!(output.status.success(), "aka-daemon binary should build successfully");
    }

    #[test]
    fn test_cli_reload_command_exists() {
        // Test that the CLI has the reload command
        let output = Command::new("cargo")
            .args(&["run", "--bin", "aka", "--", "daemon", "--help"])
            .output();

        assert!(output.is_ok(), "Should be able to run 'cargo run --bin aka daemon --help'");
        let output = output.expect("Should be able to execute cargo run command");
        let help_text = String::from_utf8_lossy(&output.stdout);
        assert!(help_text.contains("reload"), "Help text should mention reload option");
    }

    #[test]
    fn test_file_watcher_data_structures() {
        // Test that we can create the data structures needed for file watching
        let shutdown = Arc::new(AtomicBool::new(false));
        let aka_data = Arc::new(Mutex::new("test data"));

        // Simulate what the file watcher does
        assert!(!shutdown.load(Ordering::Relaxed));

        // Simulate updating the data
        {
            let mut data = aka_data.lock().expect("Should be able to acquire mutex lock");
            *data = "updated data";
        }

        // Verify the data was updated
        {
            let data = aka_data.lock().expect("Should be able to acquire mutex lock");
            assert_eq!(*data, "updated data");
        }

        // Simulate shutdown
        shutdown.store(true, Ordering::Relaxed);
        assert!(shutdown.load(Ordering::Relaxed));
    }
}
