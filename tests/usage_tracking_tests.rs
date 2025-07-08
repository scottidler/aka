use std::fs;
use tempfile::TempDir;
use std::path::PathBuf;
use aka_lib::{AKA, ProcessingMode, load_alias_cache_with_base};

fn setup_test_environment(test_name: &str) -> (TempDir, PathBuf, TempDir) {
    // Create temp directory for config file
    let config_temp_dir = TempDir::new().expect("Failed to create temp dir for config");
    let config_file = config_temp_dir.path().join("aka.yml");

    // Create temp directory for cache files
    let cache_temp_dir = TempDir::new().expect("Failed to create temp dir for cache");

    // Write test config
    let config_content = format!(r#"
# Test config for {}
lookups: {{}}

aliases:
  test-alias:
    value: echo "test command"
    global: true
  another-alias:
    value: echo "another command"
    global: false
"#, test_name);

    fs::write(&config_file, config_content).expect("Failed to write config");

    (config_temp_dir, config_file, cache_temp_dir)
}

#[test]
fn test_usage_count_initialization() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("initialization");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance with temp cache directory
    let aka = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path)).expect("Failed to create AKA instance");

    // Check that aliases are initialized with count = 0
    for (name, alias) in &aka.spec.aliases {
        assert_eq!(alias.count, 0, "Alias '{}' should have count 0", name);
    }
}

#[test]
fn test_usage_count_increment() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("increment");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance with temp cache directory
    let mut aka = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path)).expect("Failed to create AKA instance");

    // Use an alias
    let result = aka.replace_with_mode("test-alias", ProcessingMode::Direct).expect("Failed to replace");
    assert_eq!(result.trim(), "echo \"test command\"");

    // Check that count was incremented
    let test_alias = aka.spec.aliases.get("test-alias").expect("test-alias should exist");
    assert_eq!(test_alias.count, 1, "test-alias should have count 1 after use");

    // Use the same alias again
    let result = aka.replace_with_mode("test-alias", ProcessingMode::Direct).expect("Failed to replace");
    assert_eq!(result.trim(), "echo \"test command\"");

    // Check that count was incremented again
    let test_alias = aka.spec.aliases.get("test-alias").expect("test-alias should exist");
    assert_eq!(test_alias.count, 2, "test-alias should have count 2 after second use");
}

#[test]
fn test_usage_count_persistence() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("persistence");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance and use an alias
    {
        let mut aka = AKA::new_with_cache_dir(false, &Some(config_file.clone()), Some(&cache_path)).expect("Failed to create AKA instance");
        let result = aka.replace_with_mode("test-alias", ProcessingMode::Direct).expect("Failed to replace");
        assert_eq!(result.trim(), "echo \"test command\"");

        // Check that count was incremented
        let test_alias = aka.spec.aliases.get("test-alias").expect("test-alias should exist");
        assert_eq!(test_alias.count, 1, "test-alias should have count 1 after use");
    }

    // Create a new AKA instance (simulating restart)
    {
        let aka = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path)).expect("Failed to create AKA instance");

        // Check that count was persisted
        let test_alias = aka.spec.aliases.get("test-alias").expect("test-alias should exist");
        assert_eq!(test_alias.count, 1, "test-alias should have count 1 after restart");
    }
}

#[test]
fn test_no_count_increment_for_unused_aliases() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("no_increment");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance with temp cache directory
    let mut aka = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path)).expect("Failed to create AKA instance");

    // Try to use a non-existent alias
    let result = aka.replace_with_mode("non-existent-alias", ProcessingMode::Direct).expect("Failed to replace");
    assert_eq!(result, "", "Non-existent alias should return empty string");

    // Check that existing aliases still have count 0
    for (name, alias) in &aka.spec.aliases {
        assert_eq!(alias.count, 0, "Alias '{}' should still have count 0", name);
    }
}

#[test]
fn test_usage_count_with_daemon_mode() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("daemon_mode");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance with temp cache directory
    let mut aka = AKA::new_with_cache_dir(false, &Some(config_file), Some(&cache_path)).expect("Failed to create AKA instance");

    // Use an alias with daemon mode
    let result = aka.replace_with_mode("test-alias", ProcessingMode::Daemon).expect("Failed to replace");
    assert_eq!(result.trim(), "echo \"test command\"");

    // Check that count was incremented
    let test_alias = aka.spec.aliases.get("test-alias").expect("test-alias should exist");
    assert_eq!(test_alias.count, 1, "test-alias should have count 1 after daemon use");
}

#[test]
fn test_cache_loading_directly() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("cache_loading");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance and use an alias to create cache
    {
        let mut aka = AKA::new_with_cache_dir(false, &Some(config_file.clone()), Some(&cache_path)).expect("Failed to create AKA instance");
        let result = aka.replace_with_mode("test-alias", ProcessingMode::Direct).expect("Failed to replace");
        assert_eq!(result.trim(), "echo \"test command\"");
    }

    // Load cache directly using the config hash
    let hash = aka_lib::hash_config_file(&config_file).expect("Failed to hash config");

    let loaded_cache = load_alias_cache_with_base(&hash, Some(&cache_path)).expect("Failed to load cache");
    assert!(loaded_cache.is_some(), "Cache should be loaded");

    let aliases = loaded_cache.unwrap();
    let test_alias = aliases.get("test-alias").expect("test-alias should exist in cache");
    assert_eq!(test_alias.count, 1, "test-alias should have count 1 in cache");
}

#[test]
fn test_cache_debug() {
    let (_config_temp_dir, config_file, cache_temp_dir) = setup_test_environment("cache_debug");
    let cache_path = cache_temp_dir.path().to_path_buf();

    // Create AKA instance and use aliases
    {
        let mut aka = AKA::new_with_cache_dir(false, &Some(config_file.clone()), Some(&cache_path)).expect("Failed to create AKA instance");

        // Use test-alias 3 times
        for i in 1..=3 {
            let result = aka.replace_with_mode("test-alias", ProcessingMode::Direct).expect("Failed to replace");
            assert_eq!(result.trim(), "echo \"test command\"");

            let test_alias = aka.spec.aliases.get("test-alias").expect("test-alias should exist");
            assert_eq!(test_alias.count, i, "test-alias should have count {} after {} uses", i, i);
        }

        // Use another-alias 2 times
        for i in 1..=2 {
            let result = aka.replace_with_mode("another-alias", ProcessingMode::Direct).expect("Failed to replace");
            assert_eq!(result.trim(), "echo \"another command\"");

            let another_alias = aka.spec.aliases.get("another-alias").expect("another-alias should exist");
            assert_eq!(another_alias.count, i, "another-alias should have count {} after {} uses", i, i);
        }
    }

    // Load cache and verify counts
    let hash = aka_lib::hash_config_file(&config_file).expect("Failed to hash config");
    let loaded_cache = load_alias_cache_with_base(&hash, Some(&cache_path)).expect("Failed to load cache");
    assert!(loaded_cache.is_some(), "Cache should be loaded");

    let aliases = loaded_cache.unwrap();

    let test_alias = aliases.get("test-alias").expect("test-alias should exist in cache");
    assert_eq!(test_alias.count, 3, "test-alias should have count 3 in cache");

    let another_alias = aliases.get("another-alias").expect("another-alias should exist in cache");
    assert_eq!(another_alias.count, 2, "another-alias should have count 2 in cache");

    println!("Cache debug complete: test-alias={}, another-alias={}", test_alias.count, another_alias.count);
}
