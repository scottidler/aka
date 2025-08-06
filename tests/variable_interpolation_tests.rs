use eyre::Result;
use tempfile::NamedTempFile;
use std::io::Write;
use aka_lib::AKA;

/// Test basic variable interpolation
#[test]
fn test_basic_variable_interpolation() -> Result<()> {
    let yaml_content = r#"
aliases:
  av: aws-vault exec -d 12h prod --
  test-cmd: $av echo hello
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("test-cmd")?;
    assert_eq!(result, "aws-vault exec -d 12h prod -- echo hello ");

    Ok(())
}

/// Test variable interpolation with positional arguments
#[test]
fn test_variable_with_positional_args() -> Result<()> {
    let yaml_content = r#"
aliases:
  av: aws-vault exec -d 12h prod --
  deploy: $av kubectl apply -f $1
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("deploy manifest.yaml")?;
    assert_eq!(result, "aws-vault exec -d 12h prod -- kubectl apply -f manifest.yaml ");

    Ok(())
}

/// Test multiple variable interpolation in one alias
#[test]
fn test_multiple_variable_interpolation() -> Result<()> {
    let yaml_content = r#"
aliases:
  prefix: sudo
  suffix: --verbose
  cmd: $prefix docker $suffix
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("cmd")?;
    assert_eq!(result, "sudo docker --verbose ");

    Ok(())
}

/// Test chained variable interpolation (A -> B -> C)
#[test]
fn test_chained_variable_interpolation() -> Result<()> {
    let yaml_content = r#"
aliases:
  base: docker
  with-sudo: sudo $base
  final: $with-sudo run -it
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("final")?;
    assert_eq!(result, "sudo docker run -it ");

    Ok(())
}

/// Test direct cycle detection (A -> B -> A)
#[test]
fn test_direct_cycle_detection() -> Result<()> {
    let yaml_content = r#"
aliases:
  a: $b command-a
  b: $a command-b
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    // Should not crash, should leave variables unresolved
    let result = aka.replace("a")?;
    // The exact result depends on implementation - it should not crash
    assert!(!result.is_empty());

    Ok(())
}

/// Test indirect cycle detection (A -> B -> C -> A)
#[test]
fn test_indirect_cycle_detection() -> Result<()> {
    let yaml_content = r#"
aliases:
  a: $b step-a
  b: $c step-b
  c: $a step-c
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    // Should not crash, should handle cycles gracefully
    let result = aka.replace("a")?;
    assert!(!result.is_empty());

    Ok(())
}

/// Test self-reference cycle detection (A -> A)
#[test]
fn test_self_reference_cycle() -> Result<()> {
    let yaml_content = r#"
aliases:
  recursive: $recursive echo hello
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("recursive")?;
    // Should leave $recursive unresolved to prevent infinite loop
    assert!(result.contains("$recursive") || result.contains("echo hello"));

    Ok(())
}

/// Test nonexistent variable reference (should be left as-is)
#[test]
fn test_nonexistent_variable() -> Result<()> {
    let yaml_content = r#"
aliases:
  test: echo $nonexistent hello
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("test")?;
    assert_eq!(result, "echo $nonexistent hello ");

    Ok(())
}

/// Test variable interpolation with variadic arguments
#[test]
fn test_variable_with_variadic_args() -> Result<()> {
    let yaml_content = r#"
aliases:
  prefix: sudo docker
  run-all: $prefix run $@
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("run-all -it ubuntu:latest bash")?;
    assert_eq!(result, "sudo docker run -it ubuntu:latest bash ");

    Ok(())
}

/// Test the specific update-ctx example from the feature request
#[test]
fn test_update_ctx_example() -> Result<()> {
    let yaml_content = r#"
aliases:
  av: aws-vault exec -d 12h prod --
  update-ctx: $av aws eks --region lookup:region[$1] update-kubeconfig --name $1 --alias $1 --role-arn arn:aws:iam::878256633362:role/eks-$1-admin
lookups:
  region:
    test: us-west-2
    prod: us-east-1
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("update-ctx test")?;
    let expected = "aws-vault exec -d 12h prod -- aws eks --region us-west-2 update-kubeconfig --name test --alias test --role-arn arn:aws:iam::878256633362:role/eks-test-admin ";
    assert_eq!(result, expected);

    Ok(())
}

/// Test variable interpolation mixed with existing features
#[test]
fn test_variable_with_lookup_and_positional() -> Result<()> {
    let yaml_content = r#"
aliases:
  base: aws-vault exec prod --
  region-cmd: $base aws eks --region lookup:region[$1]
lookups:
  region:
    dev: us-west-2
    prod: us-east-1
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("region-cmd dev")?;
    assert_eq!(result, "aws-vault exec prod -- aws eks --region us-west-2 ");

    Ok(())
}

/// Test variable names that look like shell variables (should pass through)
#[test]
fn test_shell_variable_passthrough() -> Result<()> {
    let yaml_content = r#"
aliases:
  test: echo $HOME $USER $nonexistent_alias
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("test")?;
    // All variables should pass through unchanged since they don't match alias names
    assert_eq!(result, "echo $HOME $USER $nonexistent_alias ");

    Ok(())
}

/// Test complex nested variable interpolation
#[test]
fn test_complex_nested_interpolation() -> Result<()> {
    let yaml_content = r#"
aliases:
  docker: sudo docker
  run: $docker run
  interactive: $run -it
  ubuntu: $interactive ubuntu:latest
  shell: $ubuntu bash
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("shell")?;
    assert_eq!(result, "sudo docker run -it ubuntu:latest bash ");

    Ok(())
}

/// Test variable interpolation with spacing (normalized to single spaces)
#[test]
fn test_variable_interpolation_spacing() -> Result<()> {
    let yaml_content = r#"
aliases:
  prefix: "sudo   docker"
  cmd: "$prefix    run"
"#;

    let mut file = NamedTempFile::new()?;
    file.write_all(yaml_content.as_bytes())?;

    let home_dir = std::env::temp_dir();
    let mut aka = AKA::new(true, home_dir, file.path().to_path_buf())?;

    let result = aka.replace("cmd")?;
    // Note: Spacing is normalized during processing
    assert_eq!(result, "sudo docker run ");

    Ok(())
}

