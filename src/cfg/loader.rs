use eyre::{Result, WrapErr, Error};
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
        let content =
            fs::read_to_string(filename).context(format!("Can't load filename={filename:?}"))?;
        let spec: Spec =
            serde_yaml::from_str(&content).context(format!("Can't load content={content:?}"))?;
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
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;
    use std::collections::HashMap;
    use crate::vos;
    use crate::Alias;

    #[test]
    fn test_load_success() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        let content = r#"
defaults:
  version: 1
aliases:
  alias1:
    value: "echo Hello World"
    space: true
    global: false
"#;
        file.write_all(content.as_bytes())?;

        let loader = Loader::new();
        let spec = loader.load(&file.path().to_path_buf())?;

        let expected_aliases = {
            let mut map = HashMap::new();
            map.insert("alias1".to_string(), Alias {
                name: "alias1".to_string(),
                value: vos!["echo", "Hello", "World"],
                space: true,
                global: false,
            });
            map
        };

        assert_eq!(spec.aliases, expected_aliases);
        assert_eq!(spec.defaults.version, 1);

        Ok(())
    }

    #[test]
    fn test_load_nonexistent_file() {
        let path = PathBuf::from("/path/to/nonexistent/file");

        let loader = Loader::new();
        let result = loader.load(&path);

        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_content() -> Result<(), Error> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, "This is not valid YAML content")?;

        let loader = Loader::new();
        let result = loader.load(&file.path().to_path_buf());

        assert!(result.is_err());

        Ok(())
    }
}
