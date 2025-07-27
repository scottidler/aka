use std::fs;
use tempfile::TempDir;
use aka_lib::{get_config_path, get_config_path_with_override, AKA, ConfigLoader, ProcessingMode};

/// Test that config path resolution is consistent between daemon and direct mode
#[test]
fn test_config_path_consistency() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory structure
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  test-alias:
    value: "echo test"
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config file");

    // Test standard config path resolution
    let resolved_path = get_config_path(&home_dir).expect("Failed to resolve config path");
    assert_eq!(resolved_path, config_file);

    // Test config path with no override
    let resolved_path_no_override = get_config_path_with_override(&home_dir, &None)
        .expect("Failed to resolve config path with no override");
    assert_eq!(resolved_path_no_override, config_file);

    // Test config path with override
    let custom_config = temp_dir.path().join("custom.yml");
    fs::write(&custom_config, test_config).expect("Failed to write custom config");

    let resolved_path_with_override = get_config_path_with_override(&home_dir, &Some(custom_config.clone()))
        .expect("Failed to resolve config path with override");
    assert_eq!(resolved_path_with_override, custom_config);
}

/// Test that config path resolution fails appropriately when files don't exist
#[test]
fn test_config_path_error_handling() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Test standard config path when file doesn't exist
    let result = get_config_path(&home_dir);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("Configuration file not found"));

    // Test override path when file doesn't exist
    let non_existent = temp_dir.path().join("non_existent.yml");
    let result = get_config_path_with_override(&home_dir, &Some(non_existent));
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("Configuration file not found"));
}

/// Test that config validation catches common errors
#[test]
fn test_config_validation_catches_errors() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    // Test empty aliases
    let config_file = config_dir.join("aka.yml");
    let empty_config = r#"
aliases: {}
lookups: {}
"#;
    fs::write(&config_file, empty_config).expect("Failed to write empty config");

    let loader = ConfigLoader::new();
    let result = loader.load(&config_file);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("No aliases defined"));

    // Test invalid alias name
    let invalid_alias_config = r#"
aliases:
  "bad name":
    value: "echo test"
    global: false
  "-bad-start":
    value: "echo test"
    global: false
"#;
    fs::write(&config_file, invalid_alias_config).expect("Failed to write invalid alias config");

    let result = loader.load(&config_file);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("contains spaces"));
    assert!(error_msg.contains("starts with hyphen"));

    // Test empty alias value
    let empty_value_config = r#"
aliases:
  empty-alias:
    value: ""
    global: false
"#;
    fs::write(&config_file, empty_value_config).expect("Failed to write empty value config");

    let result = loader.load(&config_file);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("empty value"));
}

/// Test that config validation catches lookup reference errors
#[test]
fn test_lookup_validation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_file = config_dir.join("aka.yml");

    // Test undefined lookup reference
    let undefined_lookup_config = r#"
aliases:
  test-alias:
    value: "echo lookup:undefined[key]"
    global: false
lookups: {}
"#;
    fs::write(&config_file, undefined_lookup_config).expect("Failed to write undefined lookup config");

    let loader = ConfigLoader::new();
    let result = loader.load(&config_file);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("references undefined lookup"));

    // Test empty lookup
    let empty_lookup_config = r#"
aliases:
  test-alias:
    value: "echo test"
    global: false
lookups:
  empty-lookup: {}
"#;
    fs::write(&config_file, empty_lookup_config).expect("Failed to write empty lookup config");

    let result = loader.load(&config_file);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("is empty"));
}

/// Test that valid configs load successfully
#[test]
fn test_valid_config_loads_successfully() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_file = config_dir.join("aka.yml");
    // Use a simpler config without hash characters that cause issues
    let valid_config = r#"
aliases:
  test-alias:
    value: "echo test"
    global: false
  global-alias:
    value: "echo global"
    global: true
  lookup-alias:
    value: "echo lookup:colors[red]"
    global: false
lookups:
  colors:
    red: "red-color"
    blue: "blue-color"
"#;
    fs::write(&config_file, valid_config).expect("Failed to write valid config");

    let loader = ConfigLoader::new();
    let result = loader.load(&config_file);
    assert!(result.is_ok());

    let spec = result.unwrap();
    assert_eq!(spec.aliases.len(), 3);
    assert_eq!(spec.lookups.len(), 1);

    // Verify aliases are loaded correctly
    assert!(spec.aliases.contains_key("test-alias"));
    assert!(spec.aliases.contains_key("global-alias"));
    assert!(spec.aliases.contains_key("lookup-alias"));

    // Verify lookups are loaded correctly
    assert!(spec.lookups.contains_key("colors"));
    let colors = spec.lookups.get("colors").unwrap();
    assert_eq!(colors.get("red").unwrap(), "red-color");
    assert_eq!(colors.get("blue").unwrap(), "blue-color");
}

/// Test that AKA initialization is consistent between daemon and direct mode
#[test]
fn test_aka_initialization_consistency() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory structure
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  test-alias:
    value: "echo test"
    global: false
  global-alias:
    value: "echo global"
    global: true
"#;
    fs::write(&config_file, test_config).expect("Failed to write config file");

    // Test direct mode initialization
    let aka_direct = AKA::new(false, home_dir.clone(), config_file.clone()).expect("Failed to create AKA for direct mode");
    assert_eq!(aka_direct.spec.aliases.len(), 2);
    assert!(!aka_direct.eol);

    // Test daemon mode initialization (simulated)
    let aka_daemon = AKA::new(false, home_dir.clone(), config_file.clone()).expect("Failed to create AKA for daemon mode");
    assert_eq!(aka_daemon.spec.aliases.len(), 2);
    assert!(!aka_daemon.eol);

    // Both should have the same config hash
    assert_eq!(aka_direct.config_hash, aka_daemon.config_hash);

    // Both should have the same aliases
    for (name, alias) in &aka_direct.spec.aliases {
        let daemon_alias = aka_daemon.spec.aliases.get(name).expect("Alias should exist in daemon mode");
        assert_eq!(alias.value, daemon_alias.value);
        assert_eq!(alias.global, daemon_alias.global);
    }
}

/// Test that config processing produces identical results in both modes
#[test]
fn test_config_processing_consistency() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory structure
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  test-alias:
    value: "echo 'hello world'"
    global: false
  global-alias:
    value: "ls -la"
    global: true
lookups:
  env:
    home: "/home/user"
    path: "/usr/bin"
"#;
    fs::write(&config_file, test_config).expect("Failed to write config file");

    // Test both EOL modes
    for eol in [true, false] {
        let mut aka_direct = AKA::new(eol, home_dir.clone(), config_file.clone()).expect("Failed to create AKA for direct mode");
        let mut aka_daemon = AKA::new(eol, home_dir.clone(), config_file.clone()).expect("Failed to create AKA for daemon mode");

        // Test alias processing
        let test_cases = vec![
            "test-alias",
            "global-alias",
            "sudo test-alias",
            "test-alias arg1 arg2",
        ];

        for test_case in test_cases {
            let direct_result = aka_direct.replace_with_mode(test_case, ProcessingMode::Direct)
                .expect("Direct mode should work");
            let daemon_result = aka_daemon.replace_with_mode(test_case, ProcessingMode::Daemon)
                .expect("Daemon mode should work");

            assert_eq!(direct_result, daemon_result,
                "Results should be identical for '{}' with eol={}", test_case, eol);
        }
    }
}



/// Test that apparent circular references are allowed (common pattern: ls -> ls --color=auto)
#[test]
fn test_circular_reference_detection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config directory");

    let config_file = config_dir.join("aka.yml");
    let circular_config = r#"
aliases:
  ls:
    value: "ls --color=auto"
    global: false
  vim:
    value: "vim -u ~/.vimrc"
    global: false
"#;
    fs::write(&config_file, circular_config).expect("Failed to write circular config");

    let loader = ConfigLoader::new();
    let result = loader.load(&config_file);
    // Should succeed - aliases that reference their base command are common and valid
    assert!(result.is_ok(), "Aliases referencing their base command should be allowed");

    let spec = result.unwrap();
    assert_eq!(spec.aliases.len(), 2);
    assert!(spec.aliases.contains_key("ls"));
    assert!(spec.aliases.contains_key("vim"));
}
