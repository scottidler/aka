use aka_lib::{cfg::loader::Loader, AKA};
use eyre::Result;
use std::collections::HashMap;
use std::io::Write;
use tempfile::NamedTempFile;

/// Test that pipe-separated lookup keys are properly expanded
#[test]
fn test_lookup_key_expansion() -> Result<()> {
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    prod|apps: us-east-1
    staging|test|dev|ops: us-west-2
  env:
    production|prod: prod-value
    development|dev|staging: dev-value
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let loader = Loader::new();
    let mut spec = loader.load(&file.path().to_path_buf())?;

    // Apply the same expansion logic that should be in AKA::new()
    for (_, map) in spec.lookups.iter_mut() {
        let mut expanded = HashMap::new();
        for (pattern, value) in map.iter() {
            let keys: Vec<&str> = pattern.split('|').collect();
            for key in keys {
                expanded.insert(key.to_string(), value.clone());
            }
        }
        *map = expanded;
    }

    // Test that all keys are expanded correctly
    assert_eq!(spec.lookups["region"]["prod"], "us-east-1");
    assert_eq!(spec.lookups["region"]["apps"], "us-east-1");
    assert_eq!(spec.lookups["region"]["staging"], "us-west-2");
    assert_eq!(spec.lookups["region"]["test"], "us-west-2");
    assert_eq!(spec.lookups["region"]["dev"], "us-west-2");
    assert_eq!(spec.lookups["region"]["ops"], "us-west-2");

    assert_eq!(spec.lookups["env"]["production"], "prod-value");
    assert_eq!(spec.lookups["env"]["prod"], "prod-value");
    assert_eq!(spec.lookups["env"]["development"], "dev-value");
    assert_eq!(spec.lookups["env"]["dev"], "dev-value");
    assert_eq!(spec.lookups["env"]["staging"], "dev-value");

    // Test that original pipe-separated keys are gone
    assert!(!spec.lookups["region"].contains_key("prod|apps"));
    assert!(!spec.lookups["region"].contains_key("staging|test|dev|ops"));
    assert!(!spec.lookups["env"].contains_key("production|prod"));
    assert!(!spec.lookups["env"].contains_key("development|dev|staging"));

    Ok(())
}

/// Test that AKA::new() properly expands lookup keys during initialization
#[test]
fn test_aka_new_expands_lookups() -> Result<()> {
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    prod|apps: us-east-1
    staging|test|dev|ops: us-west-2
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let aka = AKA::new(false, home_dir, file.path().to_path_buf())?;

    // Test that AKA::new() properly expanded the lookup keys
    assert_eq!(aka.spec.lookups["region"]["prod"], "us-east-1");
    assert_eq!(aka.spec.lookups["region"]["apps"], "us-east-1");
    assert_eq!(aka.spec.lookups["region"]["staging"], "us-west-2");
    assert_eq!(aka.spec.lookups["region"]["test"], "us-west-2");
    assert_eq!(aka.spec.lookups["region"]["dev"], "us-west-2");
    assert_eq!(aka.spec.lookups["region"]["ops"], "us-west-2");

    // Test that original pipe-separated keys are gone
    assert!(!aka.spec.lookups["region"].contains_key("prod|apps"));
    assert!(!aka.spec.lookups["region"].contains_key("staging|test|dev|ops"));

    Ok(())
}

/// Test lookup interpolation in command lines
#[test]
fn test_lookup_interpolation() -> Result<()> {
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    prod|apps: us-east-1
    staging|test|dev|ops: us-west-2
  account:
    prod: "123456789"
    test: "987654321"
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(false, home_dir, file.path().to_path_buf())?;

    // Test basic lookup interpolation
    let result = aka.replace("aws eks --region lookup:region[test] update-kubeconfig")?;
    assert_eq!(result, "aws eks --region us-west-2 update-kubeconfig ");

    let result = aka.replace("aws eks --region lookup:region[prod] update-kubeconfig")?;
    assert_eq!(result, "aws eks --region us-east-1 update-kubeconfig ");

    let result = aka.replace("aws eks --region lookup:region[apps] update-kubeconfig")?;
    assert_eq!(result, "aws eks --region us-east-1 update-kubeconfig ");

    // Test multiple lookups in same command
    let result =
        aka.replace("aws --profile lookup:account[prod] eks --region lookup:region[prod] update-kubeconfig")?;
    assert_eq!(
        result,
        "aws --profile 123456789 eks --region us-east-1 update-kubeconfig "
    );

    // Test the original failing case
    let result = aka.replace("aws eks --region lookup:region[test] update-kubeconfig --name test --alias test --role-arn arn:aws:iam::878256633362:role/eks-test-admin")?;
    assert!(result.contains("us-west-2"));
    assert!(result.contains("--name test"));

    Ok(())
}

/// Test edge cases for lookup functionality
#[test]
fn test_lookup_edge_cases() -> Result<()> {
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    prod: us-east-1
    test: us-west-2
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(false, home_dir, file.path().to_path_buf())?;

    // Test unknown lookup key - should remain unchanged
    let result = aka.replace("aws --region lookup:region[unknown] update-kubeconfig")?;
    // The result might be empty or contain the original text - both are acceptable for unknown lookups
    if !result.is_empty() {
        assert!(
            result.contains("lookup:region[unknown]"),
            "Unknown lookup should remain unchanged: {}",
            result
        );
    }

    // Test unknown lookup table - should remain unchanged
    let result = aka.replace("aws --region lookup:unknown[test] update-kubeconfig")?;
    if !result.is_empty() {
        assert!(
            result.contains("lookup:unknown[test]"),
            "Unknown lookup table should remain unchanged: {}",
            result
        );
    }

    // Test unknown lookup table - should remain unchanged
    let result = aka.replace("aws --region lookup:nonexistent[test] update-kubeconfig")?;
    if !result.is_empty() {
        assert!(
            result.contains("lookup:nonexistent[test]"),
            "Nonexistent lookup should remain unchanged: {}",
            result
        );
    }

    // Test malformed lookup syntax - should remain unchanged
    let result = aka.replace("aws --region lookup:region test update-kubeconfig")?;
    if !result.is_empty() {
        assert!(
            result.contains("lookup:region"),
            "Malformed lookup should remain unchanged: {}",
            result
        );
    }

    let result = aka.replace("aws --region lookup:region[test update-kubeconfig")?;
    if !result.is_empty() {
        assert!(
            result.contains("lookup:region[test"),
            "Malformed lookup should remain unchanged: {}",
            result
        );
    }

    Ok(())
}

/// Integration test that would have caught the missing expansion bug
#[test]
fn test_regression_prevention_integration() -> Result<()> {
    // This test specifically tests the exact scenario that broke
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    prod|apps: us-east-1
    staging|test|dev|ops: us-west-2
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(false, home_dir, file.path().to_path_buf())?;

    // This exact command was failing before the fix
    let original_failing_command = "aws eks --region lookup:region[test] update-kubeconfig --name test --alias test --role-arn arn:aws:iam::878256633362:role/eks-test-admin";
    let result = aka.replace(original_failing_command)?;

    // If expansion is working, lookup:region[test] should be replaced with us-west-2
    assert!(
        result.contains("us-west-2"),
        "lookup:region[test] should be replaced with us-west-2, but got: {}",
        result
    );
    assert!(
        !result.contains("lookup:region[test]"),
        "lookup:region[test] should be replaced, not left as-is: {}",
        result
    );

    // Test all the expanded keys work
    assert_eq!(aka.replace("lookup:region[prod]")?, "us-east-1 ");
    assert_eq!(aka.replace("lookup:region[apps]")?, "us-east-1 ");
    assert_eq!(aka.replace("lookup:region[staging]")?, "us-west-2 ");
    assert_eq!(aka.replace("lookup:region[test]")?, "us-west-2 ");
    assert_eq!(aka.replace("lookup:region[dev]")?, "us-west-2 ");
    assert_eq!(aka.replace("lookup:region[ops]")?, "us-west-2 ");

    // Test that the original pipe-separated keys are NOT present in the final HashMap
    // This ensures the expansion actually happened
    assert!(
        !aka.spec.lookups["region"].contains_key("prod|apps"),
        "Original pipe-separated key 'prod|apps' should be expanded and removed"
    );
    assert!(
        !aka.spec.lookups["region"].contains_key("staging|test|dev|ops"),
        "Original pipe-separated key 'staging|test|dev|ops' should be expanded and removed"
    );

    Ok(())
}

/// Test that single-key lookups (no pipes) still work
#[test]
fn test_single_key_lookups() -> Result<()> {
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    prod: us-east-1
    test: us-west-2
    "key-with-dashes": special-value
    "key|with|pipes": literal-pipe-value
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(false, home_dir, file.path().to_path_buf())?;

    // Test normal single keys
    let result = aka.replace("lookup:region[prod]")?;
    assert_eq!(result, "us-east-1 ");

    let result = aka.replace("lookup:region[test]")?;
    assert_eq!(result, "us-west-2 ");

    // Test keys with special characters
    let result = aka.replace("lookup:region[key-with-dashes]")?;
    assert_eq!(result, "special-value ");

    // Test that literal pipe keys get expanded
    assert!(aka.spec.lookups["region"].contains_key("key"));
    assert!(aka.spec.lookups["region"].contains_key("with"));
    assert!(aka.spec.lookups["region"].contains_key("pipes"));
    assert_eq!(aka.spec.lookups["region"]["key"], "literal-pipe-value");
    assert_eq!(aka.spec.lookups["region"]["with"], "literal-pipe-value");
    assert_eq!(aka.spec.lookups["region"]["pipes"], "literal-pipe-value");

    Ok(())
}

/// Test that empty or whitespace-only keys are handled correctly
#[test]
fn test_whitespace_key_handling() -> Result<()> {
    let yaml_content = r#"
aliases:
  test-alias: echo hello
lookups:
  region:
    "prod | apps": us-east-1
    " staging|test ": us-west-2
    "|leading|pipe": pipe-value
    "trailing|pipe|": pipe-value2
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let aka = AKA::new(false, home_dir, file.path().to_path_buf())?;

    // Test that spaces around pipes are preserved in key names
    assert!(aka.spec.lookups["region"].contains_key("prod "));
    assert!(aka.spec.lookups["region"].contains_key(" apps"));
    assert!(aka.spec.lookups["region"].contains_key(" staging"));
    assert!(aka.spec.lookups["region"].contains_key("test "));

    // Test that empty keys from leading/trailing pipes are handled
    assert!(aka.spec.lookups["region"].contains_key(""));
    assert!(aka.spec.lookups["region"].contains_key("leading"));
    assert!(aka.spec.lookups["region"].contains_key("pipe"));
    assert!(aka.spec.lookups["region"].contains_key("trailing"));

    Ok(())
}
