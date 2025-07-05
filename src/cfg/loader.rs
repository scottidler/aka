use eyre::{Error, Result, WrapErr};
use std::fs;
use std::path::PathBuf;

use super::spec::Spec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loader {}

impl Loader {
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Load the configuration from a file
    ///
    /// # Errors
    ///
    /// Will return `Err` if `filename` does not exist, or the user does not have permission to read it.
    pub fn load(&self, filename: &PathBuf) -> Result<Spec, Error> {
        use std::time::Instant;
        use log::debug;

        let start_total = Instant::now();

        // Time file reading
        let start_read = Instant::now();
        let content = fs::read_to_string(filename).context(format!("Can't load filename={filename:?}"))?;
        let read_duration = start_read.elapsed();

        // Time YAML deserialization
        let start_yaml = Instant::now();
        let spec: Spec = serde_yaml::from_str(&content).context(format!("Can't load content={content:?}"))?;
        let yaml_duration = start_yaml.elapsed();

        let total_duration = start_total.elapsed();

        debug!("📊 Config loading timing breakdown:");
        debug!("  📁 File read: {:.3}ms", read_duration.as_secs_f64() * 1000.0);
        debug!("  🔧 YAML parse: {:.3}ms", yaml_duration.as_secs_f64() * 1000.0);
        debug!("  ⏱️  Total load: {:.3}ms", total_duration.as_secs_f64() * 1000.0);
        debug!("  📦 Aliases loaded: {}", spec.aliases.len());
        debug!("  🔍 Lookups loaded: {}", spec.lookups.len());

        Ok(spec)
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
