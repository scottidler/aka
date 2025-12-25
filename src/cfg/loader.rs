use eyre::{Error, Result, WrapErr};
use std::collections::HashMap;
use std::fs;

use super::alias::Alias;
use super::spec::Spec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loader {}

impl Loader {
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Load the configuration from a file with comprehensive validation
    ///
    /// # Errors
    ///
    /// Will return `Err` if `filename` does not exist, the user does not have permission to read it,
    /// or the configuration contains validation errors.
    pub fn load(&self, filename: &std::path::Path) -> Result<Spec, Error> {
        use log::debug;
        use std::time::Instant;

        let start_total = Instant::now();

        // Validate file accessibility first
        self.validate_file_accessibility(filename)?;

        // Time file reading
        let start_read = Instant::now();
        let content = fs::read_to_string(filename).context(format!("Can't load filename={filename:?}"))?;
        let read_duration = start_read.elapsed();

        // Time YAML deserialization
        let start_yaml = Instant::now();
        let mut spec: Spec =
            serde_yaml::from_str(&content).context(format!("Can't parse YAML from file={filename:?}"))?;
        let yaml_duration = start_yaml.elapsed();

        // Time validation
        let start_validation = Instant::now();
        self.validate_config(&spec, filename)?;
        let validation_duration = start_validation.elapsed();

        // Initialize usage counts for aliases that don't have them
        for alias in spec.aliases.values_mut() {
            if alias.count == 0 {
                // count is already 0 due to skip_deserializing, but be explicit
                alias.count = 0;
            }
        }

        let total_duration = start_total.elapsed();

        debug!("📊 Config loading timing breakdown:");
        debug!("  📁 File read: {:.3}ms", read_duration.as_secs_f64() * 1000.0);
        debug!("  🔧 YAML parse: {:.3}ms", yaml_duration.as_secs_f64() * 1000.0);
        debug!("  ✅ Validation: {:.3}ms", validation_duration.as_secs_f64() * 1000.0);
        debug!("  ⏱️  Total load: {:.3}ms", total_duration.as_secs_f64() * 1000.0);
        debug!("  📦 Aliases loaded: {}", spec.aliases.len());
        debug!("  🔍 Lookups loaded: {}", spec.lookups.len());

        Ok(spec)
    }

    /// Validate file accessibility and permissions
    fn validate_file_accessibility(&self, config_path: &std::path::Path) -> Result<()> {
        // Check if file exists
        if !config_path.exists() {
            return Err(eyre::eyre!("Config file does not exist: {:?}", config_path));
        }

        // Check if it's a regular file
        let metadata = fs::metadata(config_path).context(format!("Failed to get metadata for {config_path:?}"))?;

        if !metadata.is_file() {
            return Err(eyre::eyre!("Config path is not a regular file: {:?}", config_path));
        }

        // Check file size (reasonable limit)
        const MAX_CONFIG_SIZE: u64 = 10 * 1024 * 1024; // 10MB
        if metadata.len() > MAX_CONFIG_SIZE {
            return Err(eyre::eyre!(
                "Config file too large: {:?} ({} bytes, max {} bytes)",
                config_path,
                metadata.len(),
                MAX_CONFIG_SIZE
            ));
        }

        Ok(())
    }

    /// Comprehensive configuration validation with enhanced error context
    fn validate_config(&self, spec: &Spec, config_path: &std::path::Path) -> Result<()> {
        // Validate aliases
        self.validate_aliases(&spec.aliases, config_path)?;

        // Validate lookups
        self.validate_lookups(&spec.lookups, config_path)?;

        // Validate cross-references
        self.validate_cross_references(spec, config_path)?;

        Ok(())
    }

    /// Validate alias definitions
    fn validate_aliases(&self, aliases: &HashMap<String, Alias>, config_path: &std::path::Path) -> Result<()> {
        let mut errors = Vec::new();

        if aliases.is_empty() {
            errors.push(format!(
                "No aliases defined in {config_path:?}. Add at least one alias to make the configuration useful."
            ));
        }

        for (name, alias) in aliases {
            // Validate alias name
            if name.is_empty() {
                errors.push("Empty alias name found. Alias names must be non-empty.".to_string());
                continue;
            }

            if name.contains(' ') {
                errors.push(format!(
                    "Alias name '{name}' contains spaces. Use underscores or hyphens instead."
                ));
            }

            if name.starts_with('-') {
                errors.push(format!(
                    "Alias name '{name}' starts with hyphen. This may conflict with command flags."
                ));
            }

            // Validate alias value
            if alias.value.is_empty() {
                errors.push(format!(
                    "Alias '{name}' has empty value. Provide a command or value for the alias."
                ));
            }

            // Note: Users should have full control over their aliases
            // Dangerous command detection removed to avoid restricting legitimate use cases

            // Variadic aliases use $@ to capture remaining arguments (validated by is_variadic() method)
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(eyre::eyre!("Alias validation failed:\n{}", errors.join("\n")))
        }
    }

    /// Validate lookup definitions
    fn validate_lookups(
        &self,
        lookups: &HashMap<String, HashMap<String, String>>,
        _config_path: &std::path::Path,
    ) -> Result<()> {
        let mut errors = Vec::new();

        for (lookup_name, lookup_map) in lookups {
            if lookup_name.is_empty() {
                errors.push("Empty lookup name found. Lookup names must be non-empty.".to_string());
                continue;
            }

            if lookup_map.is_empty() {
                errors.push(format!(
                    "Lookup '{lookup_name}' is empty. Add key-value pairs or remove it."
                ));
            }

            for (key, value) in lookup_map {
                if key.is_empty() {
                    errors.push(format!(
                        "Empty key in lookup '{lookup_name}'. Lookup keys must be non-empty."
                    ));
                }

                if value.is_empty() {
                    errors.push(format!(
                        "Empty value for key '{key}' in lookup '{lookup_name}'. Lookup values must be non-empty."
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(eyre::eyre!("Lookup validation failed:\n{}", errors.join("\n")))
        }
    }

    /// Validate cross-references between aliases and lookups
    fn validate_cross_references(&self, spec: &Spec, _config_path: &std::path::Path) -> Result<()> {
        let mut errors = Vec::new();

        // Check for lookup references in aliases
        for (alias_name, alias) in &spec.aliases {
            if alias.value.contains("lookup:") {
                // Extract lookup references
                let lookup_refs = self.extract_lookup_references(&alias.value);

                for lookup_ref in lookup_refs {
                    if !spec.lookups.contains_key(&lookup_ref) {
                        errors.push(format!(
                            "Alias '{alias_name}' references undefined lookup '{lookup_ref}'. Define the lookup or fix the reference."
                        ));
                    }
                }
            }
        }

        // Note: Circular reference detection is complex and often produces false positives
        // (e.g., alias 'ls' with value 'ls --color=auto' is valid and common)
        // Skipping this validation to avoid breaking valid configurations

        if errors.is_empty() {
            Ok(())
        } else {
            Err(eyre::eyre!("Cross-reference validation failed:\n{}", errors.join("\n")))
        }
    }

    /// Extract lookup references from alias values
    fn extract_lookup_references(&self, value: &str) -> Vec<String> {
        let mut references = Vec::new();

        // Look for patterns like "lookup:name[key]"
        let chars = value.chars().peekable();
        let mut current_pos = 0;

        for ch in chars {
            if ch == 'l' && value[current_pos..].starts_with("lookup:") {
                // Found a lookup reference
                let start = current_pos + 7; // Skip "lookup:"
                let rest = &value[start..];

                if let Some(bracket_pos) = rest.find('[') {
                    let lookup_name = &rest[..bracket_pos];
                    if !lookup_name.is_empty() {
                        references.push(lookup_name.to_string());
                    }
                }
            }
            current_pos += ch.len_utf8();
        }

        references
    }
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    use crate::Alias;

    #[test]
    fn test_load_success() -> Result<(), Error> {
        // Create a mock spec file.
        let mut file = NamedTempFile::new()?;
        let content = r#"
defaults:
  version: 1
aliases:
  alias1:
    value: "echo Hello World"
    space: true
    global: false
lookups:
  region:
    prod|apps: us-east-1
    staging|test|dev|ops: us-west-2
"#;
        file.write_all(content.as_bytes())?;

        // Use the loader to load the file.
        let loader = Loader::new();
        let spec = loader.load(file.path())?;

        // Assert the spec was loaded correctly.
        let expected_aliases = {
            let mut map = HashMap::new();
            map.insert(
                "alias1".to_string(),
                Alias {
                    name: "alias1".to_string(),
                    value: "echo Hello World".to_string(),
                    space: true,
                    global: false,
                    count: 0,
                },
            );
            map
        };

        assert_eq!(spec.aliases, expected_aliases);
        assert_eq!(spec.defaults.version, 1);
        assert_eq!(spec.lookups["region"]["prod|apps"], "us-east-1");

        Ok(())
    }

    #[test]
    fn test_load_nonexistent_file() {
        // Create a path to a file that doesn't exist.
        let path = PathBuf::from("/path/to/nonexistent/file");

        let loader = Loader::new();
        let result = loader.load(&path);

        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_content() -> Result<(), Error> {
        // Create a mock spec file with invalid content.
        let mut file = NamedTempFile::new()?;
        writeln!(file, "This is not valid YAML content")?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_loader_default() {
        let loader1 = Loader::new();
        let loader2 = Loader::default();
        assert_eq!(loader1, loader2);
    }

    #[test]
    fn test_loader_clone() {
        let loader1 = Loader::new();
        let loader2 = loader1.clone();
        assert_eq!(loader1, loader2);
    }

    #[test]
    fn test_validate_file_not_regular_file() -> Result<(), Error> {
        // Create a directory instead of a file
        let temp_dir = tempfile::tempdir()?;

        let loader = Loader::new();
        let result = loader.load(temp_dir.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a regular file"));

        Ok(())
    }

    #[test]
    fn test_validate_alias_with_empty_name() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        // This YAML creates an alias with empty name by having empty key
        let content = r#"
aliases:
  "": "echo empty"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty alias name"));

        Ok(())
    }

    #[test]
    fn test_validate_alias_with_spaces_in_name() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  "my alias": "echo test"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("contains spaces"));

        Ok(())
    }

    #[test]
    fn test_validate_alias_starting_with_hyphen() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  -myalias: "echo test"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("starts with hyphen"));

        Ok(())
    }

    #[test]
    fn test_validate_alias_with_empty_value() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  myalias:
    value: ""
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty value"));

        Ok(())
    }

    #[test]
    fn test_validate_empty_lookup_name() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  test: "echo test"
lookups:
  "":
    key: value
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty lookup name"));

        Ok(())
    }

    #[test]
    fn test_validate_empty_lookup_map() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  test: "echo test"
lookups:
  myLookup: {}
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("is empty"));

        Ok(())
    }

    #[test]
    fn test_validate_empty_lookup_key() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  test: "echo test"
lookups:
  myLookup:
    "": "value"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty key in lookup"));

        Ok(())
    }

    #[test]
    fn test_validate_empty_lookup_value() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  test: "echo test"
lookups:
  myLookup:
    key: ""
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty value for key"));

        Ok(())
    }

    #[test]
    fn test_validate_undefined_lookup_reference() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  deploy: "kubectl apply -f lookup:undefined[key]"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("undefined lookup"));

        Ok(())
    }

    #[test]
    fn test_validate_valid_lookup_reference() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  deploy: "kubectl apply -f lookup:env[prod]"
lookups:
  env:
    prod: "production.yaml"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let result = loader.load(file.path());

        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_extract_lookup_references() {
        let loader = Loader::new();

        // Single lookup reference
        let refs = loader.extract_lookup_references("kubectl apply -f lookup:env[prod]");
        assert_eq!(refs, vec!["env"]);

        // Multiple lookup references
        let refs = loader.extract_lookup_references("echo lookup:first[a] and lookup:second[b]");
        assert_eq!(refs, vec!["first", "second"]);

        // No lookup references
        let refs = loader.extract_lookup_references("echo hello world");
        assert!(refs.is_empty());

        // Incomplete lookup reference (no bracket)
        let refs = loader.extract_lookup_references("lookup:incomplete");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_load_with_usage_count_initialization() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  test:
    value: "echo test"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let spec = loader.load(file.path())?;

        // Count should be initialized to 0
        assert_eq!(spec.aliases.get("test").unwrap().count, 0);

        Ok(())
    }

    #[test]
    fn test_load_multiple_aliases() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  ls: "eza -la"
  cat: "bat -p"
  gc: "git commit"
  gp: "git push"
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let spec = loader.load(file.path())?;

        assert_eq!(spec.aliases.len(), 4);
        assert!(spec.aliases.contains_key("ls"));
        assert!(spec.aliases.contains_key("cat"));
        assert!(spec.aliases.contains_key("gc"));
        assert!(spec.aliases.contains_key("gp"));

        Ok(())
    }

    #[test]
    fn test_load_with_global_aliases() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  "|c":
    value: "| xclip -sel clip"
    global: true
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let spec = loader.load(file.path())?;

        let alias = spec.aliases.get("|c").unwrap();
        assert!(alias.global);

        Ok(())
    }

    #[test]
    fn test_load_with_space_false() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  ping10:
    value: "ping 10.10.10."
    space: false
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let spec = loader.load(file.path())?;

        let alias = spec.aliases.get("ping10").unwrap();
        assert!(!alias.space);

        Ok(())
    }

    #[test]
    fn test_load_with_complex_lookups() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
aliases:
  deploy: "kubectl apply"
lookups:
  env:
    prod: production
    dev: development
  region:
    east: us-east-1
    west: us-west-2
        "#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let spec = loader.load(file.path())?;

        assert_eq!(spec.lookups.len(), 2);
        assert_eq!(spec.lookups["env"]["prod"], "production");
        assert_eq!(spec.lookups["region"]["east"], "us-east-1");

        Ok(())
    }
}
