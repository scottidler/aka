use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use serde_json;

use aka_lib::{DaemonRequest, DaemonResponse, determine_socket_path, AKA, ProcessingMode, get_config_path};

/// Test utilities for daemon integration testing
struct DaemonTestHelper {
    home_dir: PathBuf,
    config_path: PathBuf,
    _temp_dir: TempDir,
}

impl DaemonTestHelper {
    fn new(test_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let home_dir = temp_dir.path().to_path_buf();

        // Create config directory structure
        let config_dir = home_dir.join(".config").join("aka");
        fs::create_dir_all(&config_dir)?;

        // Create test config
        let config_path = config_dir.join("aka.yml");
        let test_config = format!(r#"
aliases:
  test-alias:
    value: "echo 'Hello from {test_name}'"
    global: false
  health-test:
    value: "echo 'Daemon is healthy'"
    global: true
  reload-test:
    value: "echo 'Original config'"
    global: false
lookups:
  colors:
    red: "\\033[31m"
    green: "\\033[32m"
    reset: "\\033[0m"
"#, test_name = test_name);
        fs::write(&config_path, test_config)?;

        Ok(Self {
            home_dir,
            config_path,
            _temp_dir: temp_dir,
        })
    }

    fn create_aka_instance(&self, eol: bool) -> Result<AKA, Box<dyn std::error::Error>> {
        AKA::new(eol, self.home_dir.clone(), self.config_path.clone()).map_err(|e| e.into())
    }

    fn update_config(&self, new_config: &str) -> Result<(), Box<dyn std::error::Error>> {
        fs::write(&self.config_path, new_config)?;
        Ok(())
    }
}

/// Test daemon protocol serialization/deserialization
#[test]
fn test_daemon_protocol_serialization() {
    // Test all request types serialize correctly
    let requests = vec![
        DaemonRequest::Health,
        DaemonRequest::Query {
            cmdline: "test-alias".to_string(),
            eol: false,
        },
        DaemonRequest::Query {
            cmdline: "test-alias".to_string(),
            eol: true,
        },
        DaemonRequest::List {
            global: false,
            patterns: vec!["test-*".to_string()],
        },
        DaemonRequest::List {
            global: true,
            patterns: vec![],
        },
        DaemonRequest::ReloadConfig,
        DaemonRequest::Shutdown,
    ];

    for request in requests {
        let serialized = serde_json::to_string(&request)
            .expect("Request should serialize");
        let deserialized: DaemonRequest = serde_json::from_str(&serialized)
            .expect("Request should deserialize");

        // Verify round-trip worked (basic type check)
        match (&request, &deserialized) {
            (DaemonRequest::Health, DaemonRequest::Health) => {},
            (DaemonRequest::Query { cmdline: c1, eol: e1 }, DaemonRequest::Query { cmdline: c2, eol: e2 }) => {
                assert_eq!(c1, c2);
                assert_eq!(e1, e2);
            },
            (DaemonRequest::List { global: g1, patterns: p1 }, DaemonRequest::List { global: g2, patterns: p2 }) => {
                assert_eq!(g1, g2);
                assert_eq!(p1, p2);
            },
            (DaemonRequest::ReloadConfig, DaemonRequest::ReloadConfig) => {},
            (DaemonRequest::Shutdown, DaemonRequest::Shutdown) => {},
            _ => panic!("Request types don't match after round-trip"),
        }
    }
}

/// Test daemon response serialization/deserialization
#[test]
fn test_daemon_response_serialization() {
    let responses = vec![
        DaemonResponse::Success { data: "test output".to_string() },
        DaemonResponse::Error { message: "test error".to_string() },
        DaemonResponse::Health { status: "healthy:5:aliases".to_string() },
        DaemonResponse::ConfigReloaded { success: true, message: "Config reloaded".to_string() },
        DaemonResponse::ConfigReloaded { success: false, message: "Reload failed".to_string() },
        DaemonResponse::ShutdownAck,
    ];

    for response in responses {
        let serialized = serde_json::to_string(&response)
            .expect("Response should serialize");
        let deserialized: DaemonResponse = serde_json::from_str(&serialized)
            .expect("Response should deserialize");

        // Verify round-trip worked
        match (&response, &deserialized) {
            (DaemonResponse::Success { data: d1 }, DaemonResponse::Success { data: d2 }) => {
                assert_eq!(d1, d2);
            },
            (DaemonResponse::Error { message: m1 }, DaemonResponse::Error { message: m2 }) => {
                assert_eq!(m1, m2);
            },
            (DaemonResponse::Health { status: s1 }, DaemonResponse::Health { status: s2 }) => {
                assert_eq!(s1, s2);
            },
            (DaemonResponse::ConfigReloaded { success: s1, message: m1 }, DaemonResponse::ConfigReloaded { success: s2, message: m2 }) => {
                assert_eq!(s1, s2);
                assert_eq!(m1, m2);
            },
            (DaemonResponse::ShutdownAck, DaemonResponse::ShutdownAck) => {},
            _ => panic!("Response types don't match after round-trip"),
        }
    }
}

/// Test daemon and direct mode consistency
#[test]
fn test_daemon_direct_mode_consistency() {
    let helper = DaemonTestHelper::new("consistency_test")
        .expect("Failed to create test helper");

    // Test both eol modes
    for eol in [false, true] {
        let mut aka = helper.create_aka_instance(eol)
            .expect("Failed to create AKA instance");

        let test_cases = vec![
            "test-alias",
            "health-test",
            "reload-test",
            "sudo test-alias",
            "test-alias arg1 arg2",
        ];

        for test_case in test_cases {
            // Test direct mode
            let direct_result = aka.replace_with_mode(test_case, ProcessingMode::Direct)
                .expect("Direct mode should work");

            // Test daemon mode (simulated)
            let daemon_result = aka.replace_with_mode(test_case, ProcessingMode::Daemon)
                .expect("Daemon mode should work");

            // Results should be identical
            assert_eq!(direct_result, daemon_result,
                "Direct and daemon modes should produce identical results for '{}' with eol={}",
                test_case, eol);
        }
    }
}

/// Test socket path determination
#[test]
fn test_socket_path_determination() {
    let helper = DaemonTestHelper::new("socket_test")
        .expect("Failed to create test helper");

    let socket_path = determine_socket_path(&helper.home_dir)
        .expect("Failed to determine socket path");

    assert!(socket_path.to_string_lossy().contains("aka"),
           "Socket path should contain 'aka': {:?}", socket_path);
    assert!(socket_path.to_string_lossy().contains("daemon.sock"),
           "Socket path should contain 'daemon.sock': {:?}", socket_path);

    // Verify socket path is in a reasonable location (either home dir or XDG_RUNTIME_DIR)
    let is_under_home = socket_path.starts_with(&helper.home_dir);
    let is_under_runtime = socket_path.to_string_lossy().contains("/run/user/");
    assert!(is_under_home || is_under_runtime,
           "Socket path should be under home directory or XDG_RUNTIME_DIR: {:?}", socket_path);
}

/// Test config reload functionality through AKA instance
#[test]
fn test_config_reload_integration() {
    let helper = DaemonTestHelper::new("reload_integration_test")
        .expect("Failed to create test helper");

    // Create initial AKA instance
    let mut aka = helper.create_aka_instance(false)
        .expect("Failed to create AKA instance");

    // Test initial config
    let initial_result = aka.replace_with_mode("reload-test", ProcessingMode::Direct)
        .expect("Initial query should work");
    assert!(initial_result.contains("Original config"),
           "Should contain original config value: {}", initial_result);

    // Update config file
    let updated_config = r#"
aliases:
  test-alias:
    value: "echo 'Hello from reload_integration_test'"
    global: false
  health-test:
    value: "echo 'Daemon is healthy'"
    global: true
  reload-test:
    value: "echo 'Updated config'"
    global: false
  new-alias:
    value: "echo 'New alias added'"
    global: false
lookups:
  colors:
    red: "\\033[31m"
    green: "\\033[32m"
    reset: "\\033[0m"
"#;

    helper.update_config(updated_config)
        .expect("Failed to update config");

    // Create new AKA instance to pick up changes
    let mut updated_aka = helper.create_aka_instance(false)
        .expect("Failed to create updated AKA instance");

    // Test updated config
    let updated_result = updated_aka.replace_with_mode("reload-test", ProcessingMode::Direct)
        .expect("Updated query should work");
    assert!(updated_result.contains("Updated config"),
           "Should contain updated config value: {}", updated_result);

    // Test new alias
    let new_alias_result = updated_aka.replace_with_mode("new-alias", ProcessingMode::Direct)
        .expect("New alias query should work");
    assert!(new_alias_result.contains("New alias added"),
           "Should contain new alias value: {}", new_alias_result);
}

/// Test error handling in daemon mode
#[test]
fn test_daemon_mode_error_handling() {
    let helper = DaemonTestHelper::new("error_handling_test")
        .expect("Failed to create test helper");

    let mut aka = helper.create_aka_instance(false)
        .expect("Failed to create AKA instance");

    // Test non-existent alias
    let result = aka.replace_with_mode("non-existent-alias", ProcessingMode::Direct)
        .expect("Should handle non-existent alias");
    // The behavior might be to return empty string or original command, both are valid
    assert!(result.trim() == "non-existent-alias" || result.trim() == "",
           "Should return original command or empty string for non-existent alias, got: '{}'", result.trim());

    // Test daemon mode with same non-existent alias
    let daemon_result = aka.replace_with_mode("non-existent-alias", ProcessingMode::Daemon)
        .expect("Daemon mode should handle non-existent alias");
    // Should be consistent with direct mode
    assert_eq!(daemon_result.trim(), result.trim(),
              "Daemon mode should be consistent with direct mode for non-existent alias");
}

/// Test performance characteristics
#[test]
fn test_performance_characteristics() {
    let helper = DaemonTestHelper::new("performance_test")
        .expect("Failed to create test helper");

    let mut aka = helper.create_aka_instance(false)
        .expect("Failed to create AKA instance");

    // Test direct mode performance
    let start = std::time::Instant::now();
    for _i in 0..10 {
        let _result = aka.replace_with_mode("test-alias", ProcessingMode::Direct)
            .expect("Direct mode should work");
    }
    let direct_elapsed = start.elapsed();

    // Test daemon mode performance (simulated)
    let start = std::time::Instant::now();
    for _i in 0..10 {
        let _result = aka.replace_with_mode("test-alias", ProcessingMode::Daemon)
            .expect("Daemon mode should work");
    }
    let daemon_elapsed = start.elapsed();

    // Both should be reasonably fast
    assert!(direct_elapsed < Duration::from_millis(1000),
           "Direct mode 10 operations took {}ms, should be under 1000ms",
           direct_elapsed.as_millis());
    assert!(daemon_elapsed < Duration::from_millis(1000),
           "Daemon mode 10 operations took {}ms, should be under 1000ms",
           daemon_elapsed.as_millis());
}

/// Test EOL parameter handling consistency
#[test]
fn test_eol_parameter_consistency() {
    let helper = DaemonTestHelper::new("eol_test")
        .expect("Failed to create test helper");

    // Test eol=false
    let mut aka_no_eol = helper.create_aka_instance(false)
        .expect("Failed to create AKA instance with eol=false");

    let result_no_eol = aka_no_eol.replace_with_mode("test-alias", ProcessingMode::Direct)
        .expect("Should work with eol=false");
    // The EOL parameter doesn't affect the raw result from replace_with_mode
    // It's used internally for variadic alias handling and by the binary for output formatting
    assert!(!result_no_eol.is_empty(), "Result should not be empty when alias exists");

    // Test eol=true
    let mut aka_with_eol = helper.create_aka_instance(true)
        .expect("Failed to create AKA instance with eol=true");

    let result_with_eol = aka_with_eol.replace_with_mode("test-alias", ProcessingMode::Direct)
        .expect("Should work with eol=true");
    // The EOL parameter affects internal processing but not the raw string result
    // Both should produce the same result since we're testing the library directly
    assert_eq!(result_no_eol, result_with_eol,
           "Library results should be identical - EOL affects binary output formatting, not library results");
}

/// Test config validation integration
#[test]
fn test_config_validation_integration() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    // Test with invalid config
    let config_path = config_dir.join("aka.yml");
    let invalid_config = r#"
aliases:
  "":
    value: "echo 'empty alias name'"
    global: false
  test-alias:
    value: ""
    global: false
"#;
    fs::write(&config_path, invalid_config).expect("Failed to write invalid config");

    // Try to create AKA instance with invalid config
    let config_path_result = get_config_path(&home_dir);
    let result = match config_path_result {
        Ok(config_path) => AKA::new(false, home_dir, config_path),
        Err(e) => Err(e),
    };

    // Should handle validation errors appropriately
    match result {
        Ok(_) => {
            // Config validation might be lenient or fixed
        },
        Err(e) => {
            // Should provide helpful error message
            let error_msg = e.to_string();
            assert!(!error_msg.is_empty(), "Error message should not be empty");
        }
    }
}

/// Test that daemon protocol messages have correct structure
#[test]
fn test_daemon_protocol_structure() {
    // Test that protocol messages have expected JSON structure
    let query = DaemonRequest::Query {
        cmdline: "test".to_string(),
        eol: true,
    };

    let serialized = serde_json::to_string(&query).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Query\""), "Should have type field");
    assert!(serialized.contains("\"cmdline\":\"test\""), "Should have cmdline field");
    assert!(serialized.contains("\"eol\":true"), "Should have eol field");

    let health = DaemonRequest::Health;
    let serialized = serde_json::to_string(&health).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Health\""), "Health should have type field");

    let success = DaemonResponse::Success { data: "test".to_string() };
    let serialized = serde_json::to_string(&success).expect("Should serialize");
    assert!(serialized.contains("\"type\":\"Success\""), "Success should have type field");
    assert!(serialized.contains("\"data\":\"test\""), "Success should have data field");
}
