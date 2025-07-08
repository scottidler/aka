use eyre::{Error, Result, WrapErr};
use std::fs;
use std::path::PathBuf;
use std::collections::HashMap;

use super::spec::Spec;
use super::alias::Alias;

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
    pub fn load(&self, filename: &PathBuf) -> Result<Spec, Error> {
        use std::time::Instant;
        use log::debug;

        let start_total = Instant::now();

        // Validate file accessibility first
        self.validate_file_accessibility(filename)?;

        // Time file reading
        let start_read = Instant::now();
        let content = fs::read_to_string(filename).context(format!("Can't load filename={filename:?}"))?;
        let read_duration = start_read.elapsed();

        // Time YAML deserialization
        let start_yaml = Instant::now();
        let mut spec: Spec = serde_yaml::from_str(&content).context(format!("Can't parse YAML from file={filename:?}"))?;
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
    fn validate_file_accessibility(&self, config_path: &PathBuf) -> Result<()> {
        // Check if file exists
        if !config_path.exists() {
            return Err(eyre::eyre!("Config file does not exist: {:?}", config_path));
        }

        // Check if it's a regular file
        let metadata = fs::metadata(config_path)
            .context(format!("Failed to get metadata for {:?}", config_path))?;

        if !metadata.is_file() {
            return Err(eyre::eyre!("Config path is not a regular file: {:?}", config_path));
        }

        // Check file size (reasonable limit)
        const MAX_CONFIG_SIZE: u64 = 10 * 1024 * 1024; // 10MB
        if metadata.len() > MAX_CONFIG_SIZE {
            return Err(eyre::eyre!("Config file too large: {:?} ({} bytes, max {} bytes)",
                config_path, metadata.len(), MAX_CONFIG_SIZE));
        }

        Ok(())
    }

    /// Comprehensive configuration validation
    fn validate_config(&self, spec: &Spec, config_path: &PathBuf) -> Result<()> {
        let mut errors = Vec::new();

        // Validate aliases
        if let Err(alias_errors) = self.validate_aliases(&spec.aliases, config_path) {
            errors.extend(alias_errors);
        }

        // Validate lookups
        if let Err(lookup_errors) = self.validate_lookups(&spec.lookups, config_path) {
            errors.extend(lookup_errors);
        }

        // Validate cross-references
        if let Err(ref_errors) = self.validate_cross_references(spec, config_path) {
            errors.extend(ref_errors);
        }

        if !errors.is_empty() {
            return Err(eyre::eyre!("Config validation failed:\n{}", errors.join("\n")));
        }

        Ok(())
    }

    /// Validate alias definitions
    fn validate_aliases(&self, aliases: &HashMap<String, Alias>, config_path: &PathBuf) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if aliases.is_empty() {
            errors.push(format!("No aliases defined in {:?}. Add at least one alias to make the configuration useful.", config_path));
        }

        for (name, alias) in aliases {
            // Validate alias name
            if name.is_empty() {
                errors.push("Empty alias name found. Alias names must be non-empty.".to_string());
                continue;
            }

            if name.contains(' ') {
                errors.push(format!("Alias name '{}' contains spaces. Use underscores or hyphens instead.", name));
            }

            if name.starts_with('-') {
                errors.push(format!("Alias name '{}' starts with hyphen. This may conflict with command flags.", name));
            }

            // Validate alias value
            if alias.value.is_empty() {
                errors.push(format!("Alias '{}' has empty value. Provide a command or value for the alias.", name));
            }

            // Check for potentially dangerous commands
            if alias.value.contains("rm -rf") || alias.value.contains("sudo rm") {
                errors.push(format!("Alias '{}' contains potentially dangerous command. Be careful with destructive commands.", name));
            }

            // Validate variadic usage
            if alias.is_variadic() && !alias.value.contains("...") {
                errors.push(format!("Alias '{}' marked as variadic but doesn't contain '...'. Add '...' to indicate where arguments go.", name));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate lookup definitions
    fn validate_lookups(&self, lookups: &HashMap<String, HashMap<String, String>>, _config_path: &PathBuf) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        for (lookup_name, lookup_map) in lookups {
            if lookup_name.is_empty() {
                errors.push("Empty lookup name found. Lookup names must be non-empty.".to_string());
                continue;
            }

            if lookup_map.is_empty() {
                errors.push(format!("Lookup '{}' is empty. Add key-value pairs or remove it.", lookup_name));
            }

            for (key, value) in lookup_map {
                if key.is_empty() {
                    errors.push(format!("Empty key in lookup '{}'. Lookup keys must be non-empty.", lookup_name));
                }

                if value.is_empty() {
                    errors.push(format!("Empty value for key '{}' in lookup '{}'. Lookup values must be non-empty.", key, lookup_name));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate cross-references between aliases and lookups
    fn validate_cross_references(&self, spec: &Spec, _config_path: &PathBuf) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Check for lookup references in aliases
        for (alias_name, alias) in &spec.aliases {
            if alias.value.contains("lookup:") {
                // Extract lookup references
                let lookup_refs = self.extract_lookup_references(&alias.value);

                for lookup_ref in lookup_refs {
                    if !spec.lookups.contains_key(&lookup_ref) {
                        errors.push(format!("Alias '{}' references undefined lookup '{}'. Define the lookup or fix the reference.", alias_name, lookup_ref));
                    }
                }
            }
        }

        // Check for circular references (alias referencing itself)
        for (alias_name, alias) in &spec.aliases {
            if alias.value.contains(alias_name) {
                // Simple check - could be enhanced for deeper analysis
                errors.push(format!("Alias '{}' may contain circular reference. Avoid aliases that reference themselves.", alias_name));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Extract lookup references from alias values
    fn extract_lookup_references(&self, value: &str) -> Vec<String> {
        let mut references = Vec::new();

        // Look for patterns like "lookup:name[key]"
        let mut chars = value.chars().peekable();
        let mut current_pos = 0;

        while let Some(ch) = chars.next() {
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
        let spec = loader.load(&file.path().to_path_buf())?;

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
        let result = loader.load(&file.path().to_path_buf());

        assert!(result.is_err());

        Ok(())
    }
}
