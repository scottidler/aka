use std::fs;
use tempfile::TempDir;
use aka_lib::AKA;

/// Test that AKA::new respects custom config paths and doesn't use user's cache
#[test]
fn test_config_override_bypasses_cache() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create a test config with only 2 aliases
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  test1: "echo test1"
  test2: "echo test2"
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Create AKA instance with custom config
    let aka = AKA::new(false, home_dir, config_file).expect("Failed to create AKA instance");

    // Should have exactly 2 aliases from the config file
    assert_eq!(aka.spec.aliases.len(), 2);
    assert!(aka.spec.aliases.contains_key("test1"));
    assert!(aka.spec.aliases.contains_key("test2"));
    
    // Should not contain any of the user's real aliases
    assert!(!aka.spec.aliases.contains_key("ls"));
    assert!(!aka.spec.aliases.contains_key("cat"));
    assert!(!aka.spec.aliases.contains_key("grep"));
}

/// Test that AKA::new works with different config paths
#[test]
fn test_multiple_custom_configs() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create first config
    let config1 = temp_dir.path().join("config1.yml");
    let test_config1 = r#"
defaults:
  version: 1
aliases:
  first: "echo first"
"#;
    fs::write(&config1, test_config1).expect("Failed to write config1");

    // Create second config  
    let config2 = temp_dir.path().join("config2.yml");
    let test_config2 = r#"
defaults:
  version: 1
aliases:
  second: "echo second"
  third: "echo third"
"#;
    fs::write(&config2, test_config2).expect("Failed to write config2");

    // Test first config
    let aka1 = AKA::new(false, home_dir.clone(), config1).expect("Failed to create AKA instance 1");
    assert_eq!(aka1.spec.aliases.len(), 1);
    assert!(aka1.spec.aliases.contains_key("first"));

    // Test second config
    let aka2 = AKA::new(false, home_dir.clone(), config2).expect("Failed to create AKA instance 2");
    assert_eq!(aka2.spec.aliases.len(), 2);
    assert!(aka2.spec.aliases.contains_key("second"));
    assert!(aka2.spec.aliases.contains_key("third"));
}

/// Test that get_alias_names_for_completion works with custom configs
#[test]
fn test_completion_with_custom_config() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create a test config
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
defaults:
  version: 1
aliases:
  alpha: "echo alpha"
  beta: "echo beta"
  gamma: "echo gamma"
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Create AKA instance
    let aka = AKA::new(false, home_dir, config_file).expect("Failed to create AKA instance");

    // Test completion function
    let alias_names = aka_lib::get_alias_names_for_completion(&aka);
    
    // Should be sorted alphabetically and contain only the 3 test aliases
    assert_eq!(alias_names, vec!["alpha", "beta", "gamma"]);
} 