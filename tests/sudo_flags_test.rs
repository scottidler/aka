use aka_lib::*;
use std::fs;
use tempfile::TempDir;

/// The `$(which <tool>)` expectations below only hold where the tool is on the user's PATH
/// but not root's (a dev box with cargo-installed tools). On a bare CI runner they are
/// absent, so aka correctly leaves the command unwrapped and these cases do not apply.
fn is_user_only_tool(tool: &str) -> bool {
    let quiet = std::process::Stdio::null;
    let user_has = std::process::Command::new("which")
        .arg(tool)
        .stdout(quiet())
        .stderr(quiet())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let root_has = std::process::Command::new("sudo")
        .args(["-n", "which", tool])
        .stdout(quiet())
        .stderr(quiet())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    user_has && !root_has
}

macro_rules! require_user_only_tools {
    ($($tool:expr),+) => {
        $(
            if !is_user_only_tool($tool) {
                eprintln!("skipping: {} is not a user-only tool on this machine", $tool);
                return;
            }
        )+
    };
}

#[test]
fn test_sudo_with_flags() {
    require_user_only_tools!("eza", "rkvr");
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
    require_user_only_tools!("eza");
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
        // Multiple flags with aliased command
        ("sudo -E -i -u root ls", "sudo -E -i -u root $(which eza) "),
    ];

    for (input, expected) in test_cases {
        println!("Testing: {}", input);
        let result = aka.replace(input).expect("Should process input");
        println!("Expected: {}", expected);
        println!("Got:      {}", result);
        assert_eq!(result, expected, "Failed for input: {}", input);
        println!("✅ Passed\n");
    }

    // Test system commands - these may or may not be wrapped depending on system configuration
    // (whether sudo -n which cat succeeds). The key is that flags are preserved correctly.
    let system_cmd_cases = vec![
        ("sudo -E cat", vec!["sudo -E cat ", "sudo -E $(which cat) "]),
        (
            "sudo -i systemctl",
            vec!["sudo -i systemctl ", "sudo -i $(which systemctl) "],
        ),
    ];

    for (input, valid_outputs) in system_cmd_cases {
        println!("Testing: {}", input);
        let result = aka.replace(input).expect("Should process input");
        println!("Got:      {}", result);
        assert!(
            valid_outputs.contains(&result.as_str()),
            "Failed for input: {}, got: {}, valid outputs: {:?}",
            input,
            result,
            valid_outputs
        );
        println!("✅ Passed\n");
    }
}
