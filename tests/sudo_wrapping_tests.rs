use aka_lib::*;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_sudo_wrapping_scenarios() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza"
    space: true
    global: false
  cat:
    value: "bat -p"
    space: true
    global: false
  sys:
    value: "systemctl"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test cases
    let test_cases = vec![
        // (input, should_not_double_wrap, description)
        ("sudo ls", true, "aliased ls command"),
        ("sudo cat", true, "aliased cat command"),
        ("sudo systemctl", true, "system command"),
        ("sudo echo", true, "basic system command"),
        ("sudo $(which ls)", true, "already wrapped command"),
        ("sudo ls -la", true, "command with arguments"),
    ];

    for (input, should_not_double_wrap, description) in test_cases {
        let result = aka.replace(input).expect("Should process command");

        if should_not_double_wrap {
            assert!(!result.contains("$(which $(which"),
                   "Should not double-wrap {}: '{}'", description, result);
        }

        // Should always contain sudo
        assert!(result.contains("sudo"),
               "Should contain sudo for {}: '{}'", description, result);

        println!("✅ {} -> {}", input, result);
    }
}

#[test]
fn test_sudo_wrapping_idempotent_behavior() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // First application
    let result1 = aka.replace("sudo ls").expect("First application should work");
    println!("First application: sudo ls -> {}", result1);

    // Second application should be idempotent
    let result2 = aka.replace(&result1.trim()).expect("Second application should work");
    println!("Second application: {} -> {}", result1.trim(), result2);

    // Should not double-wrap or add multiple sudos
    assert!(!result2.contains("$(which $(which"),
           "Should not double-wrap: '{}'", result2);
    assert!(!result2.contains("sudo sudo"),
           "Should not have multiple sudos: '{}'", result2);

    // Third application should also be idempotent
    let result3 = aka.replace(&result2.trim()).expect("Third application should work");
    println!("Third application: {} -> {}", result2.trim(), result3);

    assert!(!result3.contains("$(which $(which $(which"),
           "Should not triple-wrap: '{}'", result3);
    assert!(!result3.contains("sudo sudo sudo"),
           "Should not have triple sudos: '{}'", result3);
}

#[test]
fn test_sudo_wrapping_complex_commands() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create minimal config
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  dummy:
    value: "echo"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Complex commands that should not be wrapped
    let complex_commands = vec![
        "sudo ls -la",
        "sudo cat file.txt",
        "sudo grep pattern | less",
        "sudo echo hello > file.txt",
        "sudo command < input.txt",
        "sudo cmd1 && cmd2",
        "sudo cmd1; cmd2",
    ];

    for input in complex_commands {
        let result = aka.replace(input).expect("Should handle complex command");

        // Should not wrap the complex part
        assert!(!result.contains("$(which ls -la"),
               "Should not wrap complex command: '{}'", result);
        assert!(!result.contains("$(which cat file.txt"),
               "Should not wrap complex command: '{}'", result);
        assert!(!result.contains("$(which grep pattern"),
               "Should not wrap complex command: '{}'", result);

        // Should contain sudo
        assert!(result.contains("sudo"),
               "Should contain sudo: '{}'", result);

        println!("✅ Complex command: {} -> {}", input, result);
    }
}

#[test]
fn test_sudo_wrapping_nonexistent_commands() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create minimal config
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  dummy:
    value: "echo"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test with commands that definitely don't exist
    let nonexistent_commands = vec![
        "sudo nonexistent_command_12345",
        "sudo fake_binary_xyz_999",
        "sudo this_command_does_not_exist",
    ];

    for input in nonexistent_commands {
        let result = aka.replace(input).expect("Should handle nonexistent command");

        // Should not wrap commands that don't exist for the user
        assert!(!result.contains("$(which nonexistent"),
               "Should not wrap nonexistent command: '{}'", result);
        assert!(!result.contains("$(which fake_binary"),
               "Should not wrap nonexistent command: '{}'", result);
        assert!(!result.contains("$(which this_command"),
               "Should not wrap nonexistent command: '{}'", result);

        // Should still contain sudo
        assert!(result.contains("sudo"),
               "Should contain sudo: '{}'", result);

        println!("✅ Nonexistent command: {} -> {}", input, result);
    }
}

#[test]
fn test_sudo_wrapping_edge_cases() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create minimal config
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  dummy:
    value: "echo"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Edge cases
    let edge_cases = vec![
        ("sudo", "sudo "),  // Just sudo
        ("sudo ", "sudo "),  // Sudo with space
    ];

    for (input, expected) in edge_cases {
        let result = aka.replace(input).expect("Should handle edge case");
        assert_eq!(result, expected, "Edge case '{}' failed", input);

        // Should not create malformed commands
        assert!(!result.contains("$(which $(which"),
               "Should not create malformed wrapping: '{}'", result);

        println!("✅ Edge case: {} -> {}", input, result);
    }
}

#[test]
fn test_sudo_wrapping_preserves_alias_expansion() {
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
  cat:
    value: "bat -p"
    space: true
    global: false
  mygrep:
    value: "rg --color=always"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test that alias expansion still works with sudo
    let test_cases = vec![
        ("sudo ls", "eza -la"),
        ("sudo cat", "bat -p"),
        ("sudo mygrep", "rg --color=always"),
    ];

    for (input, expected_alias) in test_cases {
        let result = aka.replace(input).expect("Should expand alias");

        // Should contain sudo and the expanded alias
        assert!(result.contains("sudo"),
               "Should contain sudo: '{}'", result);
        assert!(result.contains(expected_alias),
               "Should contain expanded alias '{}': '{}'", expected_alias, result);

        // Should not double-wrap
        assert!(!result.contains("$(which $(which"),
               "Should not double-wrap: '{}'", result);

        println!("✅ Alias expansion: {} -> {}", input, result);
    }
}

#[test]
fn test_sudo_wrapping_with_arguments() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test that arguments are preserved
    let result = aka.replace("sudo ls -la --color").expect("Should preserve arguments");

    // Should contain sudo, the expanded alias, and the arguments
    assert!(result.contains("sudo"),
           "Should contain sudo: '{}'", result);
    assert!(result.contains("eza"),
           "Should contain expanded alias: '{}'", result);
    assert!(result.contains("-la"),
           "Should contain first argument: '{}'", result);
    assert!(result.contains("--color"),
           "Should contain second argument: '{}'", result);

    // Should not double-wrap
    assert!(!result.contains("$(which $(which"),
           "Should not double-wrap: '{}'", result);

    println!("✅ Arguments preserved: sudo ls -la --color -> {}", result);
}

#[test]
fn test_sudo_wrapping_consistency() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir).expect("Failed to create AKA");

    // Test that repeated applications are consistent
    let input = "sudo ls";
    let mut current = input.to_string();

    for i in 1..=5 {
        let result = aka.replace(&current).expect("Should process command");

        // Should not accumulate wrapping or sudos
        let which_count = result.matches("$(which").count();
        let sudo_count = result.matches("sudo").count();

        assert!(which_count <= 1,
               "Should not accumulate $(which) wrappers (iteration {}): '{}'", i, result);
        assert!(sudo_count <= 1,
               "Should not accumulate sudo commands (iteration {}): '{}'", i, result);

        current = result.trim().to_string();
        println!("Iteration {}: {}", i, current);
    }
}
