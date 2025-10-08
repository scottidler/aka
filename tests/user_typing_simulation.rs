use aka_lib::*;
use std::fs;
use tempfile::TempDir;

/// Test that simulates exactly what happens when a user types commands
/// This tests the critical difference between space-expansion and enter-expansion
#[test]
fn test_user_typing_simulation() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    // Create config directory
    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    // Create a test config with various alias types
    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  # Simple alias
  ls:
    value: "eza"
    space: true
    global: false

  # Alias without space
  gc:
    value: "git commit -m\""
    space: false
    global: false

  # Variadic alias (uses $@)
  echo_all:
    value: "echo $@"
    space: true
    global: false

  # Positional alias
  greet:
    value: "echo Hello $1"
    space: true
    global: false

  # Alias that expands to user-installed tool
  rmrf:
    value: "rkvr rmrf"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Get config path once for all tests
    let config_path = get_config_path(&home_dir).expect("Failed to get config path");

    // Test 1: User typing "ls" and pressing space (mid-typing)
    println!("=== TEST 1: User types 'ls' and presses SPACE ===");
    let mut aka_space = AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_space.replace("ls").expect("Should process ls");
    println!("Input: 'ls' (space pressed, eol=false)");
    println!("Output: '{}'", result);
    assert_eq!(result, "eza ", "Simple alias should expand on space");

    // Test 2: User typing "ls" and pressing enter (command execution)
    println!("\n=== TEST 2: User types 'ls' and presses ENTER ===");
    let mut aka_enter = AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_enter.replace("ls").expect("Should process ls");
    println!("Input: 'ls' (enter pressed, eol=true)");
    println!("Output: '{}'", result);
    assert_eq!(result, "eza ", "Simple alias should expand on enter");

    // Test 3: User typing "echo_all hello" and pressing space (mid-typing)
    println!("\n=== TEST 3: User types 'echo_all hello' and presses SPACE ===");
    let mut aka_space = AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_space
        .replace("echo_all hello")
        .expect("Should process echo_all hello");
    println!("Input: 'echo_all hello' (space pressed, eol=false)");
    println!("Output: '{}'", result);
    // Variadic aliases should NOT expand on space because user might still be typing
    assert_eq!(result, "", "Variadic alias should NOT expand on space (still typing)");

    // Test 4: User typing "echo_all hello" and pressing enter (command execution)
    println!("\n=== TEST 4: User types 'echo_all hello' and presses ENTER ===");
    let mut aka_enter = AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_enter
        .replace("echo_all hello")
        .expect("Should process echo_all hello");
    println!("Input: 'echo_all hello' (enter pressed, eol=true)");
    println!("Output: '{}'", result);
    assert_eq!(result, "echo hello ", "Variadic alias should expand on enter");

    // Test 5: User typing "echo_all hello world" and pressing space (mid-typing)
    println!("\n=== TEST 5: User types 'echo_all hello world' and presses SPACE ===");
    let mut aka_space = AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_space
        .replace("echo_all hello world")
        .expect("Should process echo_all hello world");
    println!("Input: 'echo_all hello world' (space pressed, eol=false)");
    println!("Output: '{}'", result);
    assert_eq!(result, "", "Variadic alias should NOT expand on space (still typing)");

    // Test 6: User typing "echo_all hello world" and pressing enter (command execution)
    println!("\n=== TEST 6: User types 'echo_all hello world' and presses ENTER ===");
    let mut aka_enter = AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_enter
        .replace("echo_all hello world")
        .expect("Should process echo_all hello world");
    println!("Input: 'echo_all hello world' (enter pressed, eol=true)");
    println!("Output: '{}'", result);
    assert_eq!(result, "echo hello world ", "Variadic alias should expand on enter");

    // Test 7: User typing "greet World" and pressing space (mid-typing)
    println!("\n=== TEST 7: User types 'greet World' and presses SPACE ===");
    let mut aka_space = AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_space.replace("greet World").expect("Should process greet World");
    println!("Input: 'greet World' (space pressed, eol=false)");
    println!("Output: '{}'", result);
    assert_eq!(result, "echo Hello World ", "Positional alias should expand on space");

    // Test 8: User typing "greet World" and pressing enter (command execution)
    println!("\n=== TEST 8: User types 'greet World' and presses ENTER ===");
    let mut aka_enter = AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_enter.replace("greet World").expect("Should process greet World");
    println!("Input: 'greet World' (enter pressed, eol=true)");
    println!("Output: '{}'", result);
    assert_eq!(result, "echo Hello World ", "Positional alias should expand on enter");

    // Test 9: User typing "gc" and pressing space (mid-typing) - no space alias
    println!("\n=== TEST 9: User types 'gc' and presses SPACE (no space alias) ===");
    let mut aka_space = AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_space.replace("gc").expect("Should process gc");
    println!("Input: 'gc' (space pressed, eol=false)");
    println!("Output: '{}'", result);
    assert_eq!(
        result, "git commit -m\"",
        "No-space alias should expand without trailing space"
    );

    // Test 10: User typing "gc" and pressing enter (command execution) - no space alias
    println!("\n=== TEST 10: User types 'gc' and presses ENTER (no space alias) ===");
    let mut aka_enter = AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
    let result = aka_enter.replace("gc").expect("Should process gc");
    println!("Input: 'gc' (enter pressed, eol=true)");
    println!("Output: '{}'", result);
    assert_eq!(
        result, "git commit -m\"",
        "No-space alias should expand without trailing space"
    );
}

/// Test progressive typing simulation - like a user typing character by character
#[test]
fn test_progressive_typing_simulation() {
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

  echo_all:
    value: "echo $@"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Get config path for all tests
    let config_path = get_config_path(&home_dir).expect("Failed to get config path");

    println!("\n=== PROGRESSIVE TYPING SIMULATION ===");

    // Simulate user typing "echo_all hello world" character by character
    let typing_sequence = vec![
        "e",
        "ec",
        "ech",
        "echo",
        "echo_",
        "echo_a",
        "echo_al",
        "echo_all",
        "echo_all ",
        "echo_all h",
        "echo_all he",
        "echo_all hel",
        "echo_all hell",
        "echo_all hello",
        "echo_all hello ",
        "echo_all hello w",
        "echo_all hello wo",
        "echo_all hello wor",
        "echo_all hello worl",
        "echo_all hello world",
    ];

    for (i, input) in typing_sequence.iter().enumerate() {
        println!("\nStep {}: User typed '{}'", i + 1, input);

        // Test with space press (eol=false)
        let mut aka_space =
            AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_space = aka_space.replace(input).expect("Should process input");
        println!("  Space press (eol=false): '{}'", result_space);

        // Test with enter press (eol=true)
        let mut aka_enter =
            AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_enter = aka_enter.replace(input).expect("Should process input");
        println!("  Enter press (eol=true): '{}'", result_enter);

        // Key assertions
        if *input == "echo_all hello world" {
            assert_eq!(result_space, "", "Variadic should not expand on space");
            assert_eq!(result_enter, "echo hello world ", "Variadic should expand on enter");
        }
    }
}

/// Test sudo command typing simulation
#[test]
fn test_sudo_typing_simulation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

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

    // Get config path for all tests
    let config_path = get_config_path(&home_dir).expect("Failed to get config path");

    println!("\n=== SUDO TYPING SIMULATION ===");

    // Simulate user typing sudo commands
    let sudo_sequence = vec![
        ("touch", ""),
        ("touch target", ""),
        ("sudo", "sudo "),
        ("sudo rmrf", "sudo -E $(which rkvr) rmrf "),
        ("sudo rkvr", "sudo -E $(which rkvr) "),
        ("sudo rkvr rmrf", "sudo -E $(which rkvr) rmrf "),
        ("sudo rkvr rmrf target", "sudo -E $(which rkvr) rmrf target "),
    ];

    for (input, expected) in sudo_sequence {
        println!("\nUser types: '{}'", input);

        // Test with space press (eol=false)
        let mut aka_space =
            AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_space = aka_space.replace(input).expect("Should process input");
        println!("  Space press result: '{}'", result_space);

        // Test with enter press (eol=true)
        let mut aka_enter =
            AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_enter = aka_enter.replace(input).expect("Should process input");
        println!("  Enter press result: '{}'", result_enter);

        // Both should be the same for sudo commands (no variadic aliases involved)
        assert_eq!(
            result_space, expected,
            "Space result should match expected for '{}'",
            input
        );
        assert_eq!(
            result_enter, expected,
            "Enter result should match expected for '{}'",
            input
        );
    }
}

/// Test mixed variadic and non-variadic aliases
#[test]
fn test_mixed_alias_types_simulation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

    let config_dir = home_dir.join(".config").join("aka");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");

    let config_file = config_dir.join("aka.yml");
    let test_config = r#"
aliases:
  # Non-variadic - should always expand
  ls:
    value: "eza"
    space: true
    global: false

  # Variadic - should only expand on enter
  echo_all:
    value: "echo $@"
    space: true
    global: false

  # Positional - should always expand
  greet:
    value: "echo Hello $1"
    space: true
    global: false
"#;
    fs::write(&config_file, test_config).expect("Failed to write config");

    // Get config path for all tests
    let config_path = get_config_path(&home_dir).expect("Failed to get config path");

    println!("\n=== MIXED ALIAS TYPES SIMULATION ===");

    let test_cases = vec![
        // (input, expected_space, expected_enter, description)
        (
            "ls",
            "eza ",
            "eza ",
            "Simple alias - should expand on both space and enter",
        ),
        (
            "ls -la",
            "eza -la ",
            "eza -la ",
            "Simple alias with args - should expand on both",
        ),
        (
            "greet World",
            "echo Hello World ",
            "echo Hello World ",
            "Positional alias - should expand on both",
        ),
        (
            "echo_all hello",
            "",
            "echo hello ",
            "Variadic alias - should only expand on enter",
        ),
        (
            "echo_all hello world",
            "",
            "echo hello world ",
            "Variadic alias with multiple args - should only expand on enter",
        ),
    ];

    for (input, expected_space, expected_enter, description) in test_cases {
        println!("\n{}", description);
        println!("Input: '{}'", input);

        // Test with space press (eol=false)
        let mut aka_space =
            AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_space = aka_space.replace(input).expect("Should process input");
        println!("  Space press: '{}'", result_space);
        assert_eq!(result_space, expected_space, "Space result mismatch for '{}'", input);

        // Test with enter press (eol=true)
        let mut aka_enter =
            AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_enter = aka_enter.replace(input).expect("Should process input");
        println!("  Enter press: '{}'", result_enter);
        assert_eq!(result_enter, expected_enter, "Enter result mismatch for '{}'", input);
    }
}

/// Test the exact sequence from the original user complaint
#[test]
fn test_original_user_sequence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let home_dir = temp_dir.path().to_path_buf();

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

    // Get config path for all tests
    let config_path = get_config_path(&home_dir).expect("Failed to get config path");

    println!("\n=== ORIGINAL USER SEQUENCE SIMULATION ===");

    // The exact sequence the user described
    let sequence = vec![
        "touch",
        "touch target",
        "sudo",
        "sudo rmrf",
        "sudo rkvr rmrf",
        "sudo $(which rkvr) rmrf",
        "sudo $(which rkvr) rmrf target",
    ];

    for input in sequence {
        println!("\nUser types: '{}'", input);

        // Test with space press (eol=false) - what happens when user presses space
        let mut aka_space =
            AKA::new(false, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_space = aka_space.replace(input).expect("Should process input");
        println!("  Space press (mid-typing): '{}'", result_space);

        // Test with enter press (eol=true) - what happens when user presses enter
        let mut aka_enter =
            AKA::new(true, home_dir.clone(), config_path.clone()).expect("Failed to create AKA instance");
        let result_enter = aka_enter.replace(input).expect("Should process input");
        println!("  Enter press (execute): '{}'", result_enter);

        // Specific assertions for key transitions
        match input {
            "touch" => {
                assert_eq!(result_space, "", "No alias for 'touch'");
                assert_eq!(result_enter, "", "No alias for 'touch'");
            }
            "touch target" => {
                assert_eq!(result_space, "", "No alias for 'touch target'");
                assert_eq!(result_enter, "", "No alias for 'touch target'");
            }
            "sudo" => {
                assert_eq!(result_space, "sudo ", "Just 'sudo' should return 'sudo '");
                assert_eq!(result_enter, "sudo ", "Just 'sudo' should return 'sudo '");
            }
            "sudo rmrf" => {
                // This is where the magic happens - rmrf expands to rkvr rmrf
                assert!(result_space.contains("sudo"), "Should contain sudo");
                assert!(result_space.contains("rkvr"), "Should contain rkvr");
                assert!(result_space.contains("rmrf"), "Should contain rmrf");
                assert_eq!(
                    result_space, result_enter,
                    "Sudo results should be same for space/enter"
                );
            }
            "sudo rkvr rmrf" => {
                // Direct rkvr command - should wrap with $(which)
                assert!(result_space.contains("sudo"), "Should contain sudo");
                assert!(result_space.contains("$(which rkvr)"), "Should wrap with $(which)");
                assert_eq!(
                    result_space, result_enter,
                    "Sudo results should be same for space/enter"
                );
            }
            _ => {
                // For other cases, just ensure no crashes
                assert!(!result_space.contains("error"), "Should not contain error");
                assert!(!result_enter.contains("error"), "Should not contain error");
            }
        }
    }
}
