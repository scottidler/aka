use std::fs;
use tempfile::TempDir;
use aka_lib::{AKA, get_config_path, test_config};

fn setup_test_environment(_test_name: &str) -> (TempDir, TempDir) {
    // Create temp directory for config file
    let config_temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create temp directory for cache files
    let cache_temp_dir = TempDir::new().expect("Failed to create temp directory");

    (config_temp_dir, cache_temp_dir)
}

#[test]
fn test_aka_library_can_be_instantiated() {
    let (config_temp_dir, cache_temp_dir) = setup_test_environment("instantiation");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create a minimal config file
    let config_file = config_temp_dir.path().join("aka.yml");
    let minimal_config = r#"
lookups: {}
aliases:
  test-alias:
    value: echo "test"
    global: true
"#;
    fs::write(&config_file, minimal_config).expect("Failed to write test config");

    // Test that AKA can be instantiated with the config
    let result = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path));
    assert!(result.is_ok(), "AKA should be instantiable with valid config");

    let aka = result.unwrap();
    assert_eq!(aka.spec.aliases.len(), 1);
    assert!(aka.spec.aliases.contains_key("test-alias"));
}

#[test]
fn test_config_path_resolution() {
    let (_config_temp_dir, _cache_temp_dir) = setup_test_environment("config_path");

    // Test that get_config_path works - it should succeed if real config exists
    let config_path_result = get_config_path();

    // The result depends on whether a real config file exists
    // This test just verifies the function doesn't panic
    match config_path_result {
        Ok(path) => {
            println!("Config path found: {:?}", path);
            assert!(path.to_string_lossy().contains("aka.yml"), "Config path should contain aka.yml");
        }
        Err(e) => {
            println!("Config path not found (expected in test environment): {}", e);
            // This is also acceptable in test environment
        }
    }
}

#[test]
fn test_config_validation_function() {
    let (config_temp_dir, _cache_temp_dir) = setup_test_environment("config_validation");

    // Test with valid config file
    let config_file = config_temp_dir.path().join("aka.yml");
    let valid_config = r#"
lookups: {}
aliases:
  test-alias:
    value: echo "test"
    global: true
"#;
    fs::write(&config_file, valid_config).expect("Failed to write valid config");

    let result = test_config(&config_file);
    assert!(result.is_ok(), "test_config should succeed with valid config file");

    // Test with non-existent config file
    let non_existent = config_temp_dir.path().join("nonexistent.yml");
    let result = test_config(&non_existent);
    assert!(result.is_err(), "test_config should fail with non-existent config file");
}

#[test]
fn test_alias_replacement_basic() {
    let (config_temp_dir, cache_temp_dir) = setup_test_environment("alias_replacement");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create config with test alias
    let config_file = config_temp_dir.path().join("aka.yml");
    let config_content = r#"
lookups: {}
aliases:
  test-alias:
    value: echo "hello world"
    global: true
  local-alias:
    value: ls -la
    global: false
"#;
    fs::write(&config_file, config_content).expect("Failed to write config");

    // Test alias replacement
    let mut aka = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path)).expect("Failed to create AKA");

    // Test global alias
    let result = aka.replace_with_mode("test-alias", aka_lib::ProcessingMode::Direct).expect("Failed to replace alias");
    assert_eq!(result.trim(), "echo \"hello world\"");

    // Test local alias
    let result = aka.replace_with_mode("local-alias", aka_lib::ProcessingMode::Direct).expect("Failed to replace alias");
    assert_eq!(result.trim(), "ls -la");

    // Test non-existent alias
    let result = aka.replace_with_mode("non-existent", aka_lib::ProcessingMode::Direct).expect("Failed to replace alias");
    assert_eq!(result, "");
}
