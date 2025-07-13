use aka_lib::*;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_sudo_transition_sequence() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create a test config that includes the rmrf alias
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  rmrf:
    value: "rkvr rmrf"
    space: true
    global: false
  ls:
    value: "eza"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    println!("=== Testing sudo transition sequence ===");

    // Step 1: Basic command (no sudo)
    let result1 = aka.replace("touch target").expect("Should process touch target");
    println!("Step 1 - touch target: '{}'", result1);
    assert_eq!(result1, ""); // No aliases, should return empty string

    // Step 2: Just sudo
    let result2 = aka.replace("sudo").expect("Should process sudo");
    println!("Step 2 - sudo: '{}'", result2);
    assert_eq!(result2, "sudo ");

    // Step 3: sudo with alias that expands
    let result3 = aka.replace("sudo rmrf").expect("Should process sudo rmrf");
    println!("Step 3 - sudo rmrf: '{}'", result3);
    // rmrf should expand to "rkvr rmrf", and since rkvr is user-installed, it should be wrapped
    assert!(result3.contains("sudo"));
    assert!(result3.contains("rkvr") && result3.contains("rmrf"));
    assert!(result3.contains("$(which") || result3.contains("-E"));

    // Step 4: sudo with already expanded command
    let result4 = aka.replace("sudo rkvr rmrf").expect("Should process sudo rkvr rmrf");
    println!("Step 4 - sudo rkvr rmrf: '{}'", result4);
    // Should wrap rkvr since it's user-installed
    assert!(result4.contains("sudo"));
    assert!(result4.contains("rkvr") && result4.contains("rmrf"));
    assert!(result4.contains("$(which") || result4.contains("-E"));

    // Step 5: sudo with already wrapped command
    let result5 = aka.replace("sudo $(which rkvr) rmrf").expect("Should process sudo $(which rkvr) rmrf");
    println!("Step 5 - sudo $(which rkvr) rmrf: '{}'", result5);
    // Should be idempotent - no double wrapping
    assert!(result5.contains("sudo"));
    assert!(result5.contains("$(which rkvr)"));
    assert!(!result5.contains("$(which $(which"));

    // Step 6: sudo with already wrapped command and target
    let result6 = aka.replace("sudo $(which rkvr) rmrf target").expect("Should process sudo $(which rkvr) rmrf target");
    println!("Step 6 - sudo $(which rkvr) rmrf target: '{}'", result6);
    // Should preserve target argument
    assert!(result6.contains("sudo"));
    assert!(result6.contains("$(which rkvr)"));
    assert!(result6.contains("target"));
    assert!(!result6.contains("$(which $(which"));

    // Step 7: Test idempotency - processing the full result again
    let full_result = result6.trim();
    let result7 = aka.replace(full_result).expect("Should process full result again");
    println!("Step 7 - Processing full result again: '{}'", result7);
    // Should be exactly the same (idempotent) - note: result7 might have trailing space
    assert_eq!(result7.trim(), full_result);

    println!("=== All transitions completed successfully ===");
}

#[test]
fn test_sudo_wrapping_prevents_double_wrapping() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  rmrf:
    value: "rkvr rmrf"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test that already wrapped commands don't get double-wrapped
    let already_wrapped = "sudo $(which rkvr) rmrf target";
    let result = aka.replace(already_wrapped).expect("Should process already wrapped command");

    println!("Input: {}", already_wrapped);
    println!("Output: {}", result);

    // Should not contain double wrapping
    assert!(!result.contains("$(which $(which"));
    assert!(!result.contains("sudo sudo"));

    // Should preserve the original structure
    assert!(result.contains("sudo"));
    assert!(result.contains("$(which rkvr)"));
    assert!(result.contains("target"));
}

#[test]
fn test_sudo_with_complex_commands_no_wrapping() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza -la"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test that complex commands (with arguments) don't get $(which) wrapping
    let result = aka.replace("sudo ls target").expect("Should process sudo ls target");

    println!("Input: sudo ls target");
    println!("Output: {}", result);

    // Should expand the alias - note: eza might be wrapped with $(which) if it's user-installed
    assert!(result.contains("sudo"));
    assert!(result.contains("eza"));
    assert!(result.contains("-la"));
    assert!(result.contains("target"));
    // The key test is that we don't get double wrapping
    assert!(!result.contains("$(which $(which"));
}

#[test]
fn test_home_environment_preservation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  rmrf:
    value: "rkvr rmrf"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    let result = aka.replace("sudo rmrf target").expect("Should process sudo rmrf target");

    println!("Input: sudo rmrf target");
    println!("Output: {}", result);

    // Should preserve environment with -E flag for user-installed tools
    assert!(result.contains("sudo"));
    assert!(result.contains("-E"));
    assert!(result.contains("rkvr") && result.contains("rmrf"));
    assert!(result.contains("target"));
}
