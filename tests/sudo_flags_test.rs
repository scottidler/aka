use aka_lib::*;
use tempfile::TempDir;
use std::fs;

#[test]
fn test_sudo_with_flags() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create a test config
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  ls:
    value: "eza"
    space: true
    global: false
  rmrf:
    value: "rkvr rmrf"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir.clone(), config_file).expect("Failed to create AKA instance");

    // Test various sudo flag combinations
    let test_cases = vec![
        ("sudo -E ls", "sudo -E $(which eza) "),
        ("sudo -i ls", "sudo -i -E $(which eza) "),
        ("sudo -E -i ls", "sudo -E -i $(which eza) "),
        ("sudo -u root ls", "sudo -u root -E $(which eza) "),
        ("sudo -E rmrf", "sudo -E $(which rkvr) rmrf "),
        ("sudo -i rmrf target", "sudo -i -E $(which rkvr) rmrf target "),
    ];

    for (input, expected) in test_cases {
        println!("Testing: {}", input);
        let result = aka.replace(input).expect("Should process input");
        println!("Expected: {}", expected);
        println!("Got:      {}", result);
        assert_eq!(result, expected, "Failed for input: {}", input);
        println!("✅ Passed\n");
    }
}

#[test]
fn test_sudo_flags_without_aliases() {
    // Test that sudo flags work even without aliases
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  dummy:
    value: "echo dummy"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    let mut aka = AKA::new(false, home_dir.clone(), config_file).expect("Failed to create AKA instance");

    // Test sudo flags with direct commands (no aliases)
    let test_cases = vec![
        ("sudo -E", "sudo -E "),
        ("sudo -i", "sudo -i "),
        ("sudo -E -i", "sudo -E -i "),
        ("sudo -u root", "sudo -u root "),
    ];

    for (input, expected) in test_cases {
        println!("Testing: {}", input);
        let result = aka.replace(input).expect("Should process input");
        println!("Expected: {}", expected);
        println!("Got:      {}", result);
        assert_eq!(result, expected, "Failed for input: {}", input);
        println!("✅ Passed\n");
    }
}

#[test]
fn test_sudo_flags_edge_cases() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

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

    let mut aka = AKA::new(false, home_dir.clone(), config_file).expect("Failed to create AKA instance");

    // Test edge cases
    let test_cases = vec![
        // Multiple flags
        ("sudo -E -i -u root ls", "sudo -E -i -u root $(which eza) "),
        // System commands (shouldn't be wrapped)
        ("sudo -E cat", "sudo -E cat "),
        ("sudo -i systemctl", "sudo -i systemctl "),
    ];

    for (input, expected) in test_cases {
        println!("Testing: {}", input);
        let result = aka.replace(input).expect("Should process input");
        println!("Expected: {}", expected);
        println!("Got:      {}", result);
        assert_eq!(result, expected, "Failed for input: {}", input);
        println!("✅ Passed\n");
    }
}
