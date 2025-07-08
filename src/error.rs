use std::path::PathBuf;
use std::fmt;

/// Enhanced error types with rich context for better debugging and user experience
#[derive(Debug, Clone)]
pub enum AkaError {
    /// Configuration file not found
    ConfigNotFound {
        attempted_paths: Vec<PathBuf>,
        home_dir: PathBuf,
        custom_path: Option<PathBuf>,
    },

    /// Configuration file parsing error
    ConfigParseError {
        file_path: PathBuf,
        line: Option<usize>,
        column: Option<usize>,
        context: String,
        underlying_error: String,
    },

    /// Configuration validation error
    ConfigValidationError {
        file_path: PathBuf,
        errors: Vec<ValidationError>,
    },

    /// File operation error
    FileOperationError {
        file_path: PathBuf,
        operation: String,
        underlying_error: String,
        context: String,
    },

    /// Alias processing error
    AliasProcessingError {
        alias_name: String,
        command_line: String,
        operation: String,
        underlying_error: String,
        context: String,
    },

    /// Lookup resolution error
    LookupError {
        lookup_name: String,
        key: String,
        available_lookups: Vec<String>,
        available_keys: Vec<String>,
        context: String,
    },

    /// Circular reference error
    CircularReferenceError {
        alias_chain: Vec<String>,
        context: String,
    },

    /// Runtime error during operation
    RuntimeError {
        operation: String,
        context: String,
        underlying_error: String,
        suggestions: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub error_type: String,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub context: String,
}

impl fmt::Display for AkaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AkaError::ConfigNotFound { attempted_paths, home_dir, custom_path } => {
                write!(f, "Configuration file not found\n")?;
                if let Some(custom) = custom_path {
                    write!(f, "  Custom config path: {}\n", custom.display())?;
                } else {
                    write!(f, "  Home directory: {}\n", home_dir.display())?;
                    write!(f, "  Attempted paths:\n")?;
                    for path in attempted_paths {
                        write!(f, "    - {}\n", path.display())?;
                    }
                    write!(f, "  \n")?;
                    write!(f, "  To create a config file, run:\n")?;
                    write!(f, "    mkdir -p {}\n", home_dir.join(".config/aka").display())?;
                    write!(f, "    echo 'aliases: {{}}' > {}\n", home_dir.join(".config/aka/aka.yml").display())?;
                }
                Ok(())
            }

            AkaError::ConfigParseError { file_path, line, column, context, underlying_error } => {
                write!(f, "Configuration parsing error in {}\n", file_path.display())?;
                if let (Some(line), Some(column)) = (line, column) {
                    write!(f, "  Location: line {}, column {}\n", line, column)?;
                } else if let Some(line) = line {
                    write!(f, "  Location: line {}\n", line)?;
                }
                write!(f, "  Context: {}\n", context)?;
                write!(f, "  Error: {}\n", underlying_error)?;
                write!(f, "  \n")?;
                write!(f, "  Check the YAML syntax and ensure all quotes are properly closed.")?;
                Ok(())
            }

            AkaError::ConfigValidationError { file_path, errors } => {
                write!(f, "Configuration validation failed in {}\n", file_path.display())?;
                write!(f, "  Found {} validation error(s):\n", errors.len())?;
                for (i, error) in errors.iter().enumerate() {
                    write!(f, "    {}. {}: {}\n", i + 1, error.error_type, error.message)?;
                    if let Some(line) = error.line {
                        write!(f, "       Location: line {}\n", line)?;
                    }
                    if !error.context.is_empty() {
                        write!(f, "       Context: {}\n", error.context)?;
                    }
                }
                Ok(())
            }

            AkaError::FileOperationError { file_path, operation, underlying_error, context } => {
                write!(f, "File operation failed\n")?;
                write!(f, "  File: {}\n", file_path.display())?;
                write!(f, "  Operation: {}\n", operation)?;
                write!(f, "  Context: {}\n", context)?;
                write!(f, "  Error: {}\n", underlying_error)?;
                write!(f, "  \n")?;
                write!(f, "  Check file permissions and disk space.")?;
                Ok(())
            }

            AkaError::AliasProcessingError { alias_name, command_line, operation, underlying_error, context } => {
                write!(f, "Alias processing failed\n")?;
                write!(f, "  Alias: {}\n", alias_name)?;
                write!(f, "  Command: {}\n", command_line)?;
                write!(f, "  Operation: {}\n", operation)?;
                write!(f, "  Context: {}\n", context)?;
                write!(f, "  Error: {}\n", underlying_error)?;
                Ok(())
            }

            AkaError::LookupError { lookup_name, key, available_lookups, available_keys, context } => {
                write!(f, "Lookup resolution failed\n")?;
                write!(f, "  Lookup: {}\n", lookup_name)?;
                write!(f, "  Key: {}\n", key)?;
                write!(f, "  Context: {}\n", context)?;
                write!(f, "  \n")?;
                if available_lookups.is_empty() {
                    write!(f, "  No lookups are defined in the configuration.\n")?;
                } else {
                    write!(f, "  Available lookups:\n")?;
                    for lookup in available_lookups {
                        write!(f, "    - {}\n", lookup)?;
                    }
                }
                if !available_keys.is_empty() {
                    write!(f, "  Available keys in {}:\n", lookup_name)?;
                    for key in available_keys {
                        write!(f, "    - {}\n", key)?;
                    }
                }
                Ok(())
            }

            AkaError::CircularReferenceError { alias_chain, context } => {
                write!(f, "Circular reference detected\n")?;
                write!(f, "  Context: {}\n", context)?;
                write!(f, "  Alias chain: {}\n", alias_chain.join(" -> "))?;
                write!(f, "  \n")?;
                write!(f, "  To fix this, modify one of the aliases to break the circular dependency.")?;
                Ok(())
            }

            AkaError::RuntimeError { operation, context, underlying_error, suggestions } => {
                write!(f, "Runtime error during {}\n", operation)?;
                write!(f, "  Context: {}\n", context)?;
                write!(f, "  Error: {}\n", underlying_error)?;
                if !suggestions.is_empty() {
                    write!(f, "  \n")?;
                    write!(f, "  Suggestions:\n")?;
                    for suggestion in suggestions {
                        write!(f, "    - {}\n", suggestion)?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AkaError {}

/// Context builder for creating enhanced errors
pub struct ErrorContext {
    operation: String,
    file_path: Option<PathBuf>,
    alias_name: Option<String>,
    command_line: Option<String>,
    additional_context: Vec<String>,
}

impl ErrorContext {
    pub fn new(operation: &str) -> Self {
        Self {
            operation: operation.to_string(),
            file_path: None,
            alias_name: None,
            command_line: None,
            additional_context: Vec::new(),
        }
    }

    pub fn with_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.file_path = Some(path.into());
        self
    }

    pub fn with_alias(mut self, alias: &str) -> Self {
        self.alias_name = Some(alias.to_string());
        self
    }

    pub fn with_command(mut self, command: &str) -> Self {
        self.command_line = Some(command.to_string());
        self
    }

    pub fn with_context(mut self, context: &str) -> Self {
        self.additional_context.push(context.to_string());
        self
    }

    pub fn to_config_not_found_error(self, attempted_paths: Vec<PathBuf>, home_dir: PathBuf, custom_path: Option<PathBuf>) -> AkaError {
        AkaError::ConfigNotFound {
            attempted_paths,
            home_dir,
            custom_path,
        }
    }

    pub fn to_config_parse_error(self, underlying_error: eyre::Error, line: Option<usize>, column: Option<usize>) -> AkaError {
        AkaError::ConfigParseError {
            file_path: self.file_path.unwrap_or_else(|| PathBuf::from("unknown")),
            line,
            column,
            context: self.additional_context.join("; "),
            underlying_error: underlying_error.to_string(),
        }
    }

    pub fn to_config_validation_error(self, errors: Vec<ValidationError>) -> AkaError {
        AkaError::ConfigValidationError {
            file_path: self.file_path.unwrap_or_else(|| PathBuf::from("unknown")),
            errors,
        }
    }

    pub fn to_file_operation_error(self, underlying_error: eyre::Error) -> AkaError {
        AkaError::FileOperationError {
            file_path: self.file_path.unwrap_or_else(|| PathBuf::from("unknown")),
            operation: self.operation,
            underlying_error: underlying_error.to_string(),
            context: self.additional_context.join("; "),
        }
    }

    pub fn to_alias_processing_error(self, underlying_error: eyre::Error) -> AkaError {
        AkaError::AliasProcessingError {
            alias_name: self.alias_name.unwrap_or_else(|| "unknown".to_string()),
            command_line: self.command_line.unwrap_or_else(|| "unknown".to_string()),
            operation: self.operation,
            underlying_error: underlying_error.to_string(),
            context: self.additional_context.join("; "),
        }
    }

    pub fn to_lookup_error(self, lookup_name: &str, key: &str, available_lookups: Vec<String>, available_keys: Vec<String>) -> AkaError {
        AkaError::LookupError {
            lookup_name: lookup_name.to_string(),
            key: key.to_string(),
            available_lookups,
            available_keys,
            context: self.additional_context.join("; "),
        }
    }

    pub fn to_circular_reference_error(self, alias_chain: Vec<String>) -> AkaError {
        AkaError::CircularReferenceError {
            alias_chain,
            context: self.additional_context.join("; "),
        }
    }

    pub fn to_runtime_error(self, underlying_error: eyre::Error, suggestions: Vec<String>) -> AkaError {
        AkaError::RuntimeError {
            operation: self.operation,
            context: self.additional_context.join("; "),
            underlying_error: underlying_error.to_string(),
            suggestions,
        }
    }
}

/// Helper function to extract line and column information from YAML parsing errors
pub fn extract_yaml_position(error: &str) -> (Option<usize>, Option<usize>) {
    // Try to parse line and column from YAML error messages
    let mut line = None;
    let mut column = None;

    // Look for patterns like "at line 5, column 10" or "line 5 column 10"
    if let Some(captures) = regex::Regex::new(r"line (\d+)").unwrap().captures(error) {
        if let Ok(line_num) = captures[1].parse::<usize>() {
            line = Some(line_num);
        }
    }

    if let Some(captures) = regex::Regex::new(r"column (\d+)").unwrap().captures(error) {
        if let Ok(col_num) = captures[1].parse::<usize>() {
            column = Some(col_num);
        }
    }

    (line, column)
}

/// Convert standard eyre errors to enhanced AkaError with context
pub fn enhance_error(error: eyre::Error, context: ErrorContext) -> AkaError {
    let error_str = error.to_string();

    // Try to extract position information for YAML errors
    let (line, column) = extract_yaml_position(&error_str);

    if error_str.contains("YAML") || error_str.contains("yaml") {
        context.to_config_parse_error(error, line, column)
    } else if error_str.contains("permission") || error_str.contains("Permission") {
        context.to_file_operation_error(error)
    } else if error_str.contains("not found") || error_str.contains("No such file") {
        context.to_file_operation_error(error)
    } else {
        context.to_runtime_error(error, vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eyre::eyre;
    use std::path::PathBuf;

    #[test]
    fn test_config_not_found_error_display() {
        let error = AkaError::ConfigNotFound {
            attempted_paths: vec![
                PathBuf::from("/home/user/.config/aka/aka.yml"),
                PathBuf::from("/home/user/.aka.yml"),
            ],
            home_dir: PathBuf::from("/home/user"),
            custom_path: None,
        };

        let display = error.to_string();
        assert!(display.contains("Configuration file not found"));
        assert!(display.contains("/home/user/.config/aka/aka.yml"));
        assert!(display.contains("/home/user/.aka.yml"));
        assert!(display.contains("mkdir -p"));
    }

    #[test]
    fn test_config_parse_error_display() {
        let error = AkaError::ConfigParseError {
            file_path: PathBuf::from("/home/user/.config/aka/aka.yml"),
            line: Some(10),
            column: Some(5),
            context: "parsing aliases section".to_string(),
            underlying_error: "missing closing quote".to_string(),
        };

        let display = error.to_string();
        assert!(display.contains("Configuration parsing error"));
        assert!(display.contains("line 10, column 5"));
        assert!(display.contains("parsing aliases section"));
        assert!(display.contains("missing closing quote"));
    }

    #[test]
    fn test_validation_error_display() {
        let validation_errors = vec![
            ValidationError {
                error_type: "Empty alias name".to_string(),
                message: "Alias name cannot be empty".to_string(),
                line: Some(5),
                column: None,
                context: "aliases section".to_string(),
            },
            ValidationError {
                error_type: "Dangerous command".to_string(),
                message: "Command contains 'rm -rf /'".to_string(),
                line: Some(8),
                column: None,
                context: "alias 'dangerous'".to_string(),
            },
        ];

        let error = AkaError::ConfigValidationError {
            file_path: PathBuf::from("/home/user/.config/aka/aka.yml"),
            errors: validation_errors,
        };

        let display = error.to_string();
        assert!(display.contains("Configuration validation failed"));
        assert!(display.contains("Found 2 validation error(s)"));
        assert!(display.contains("Empty alias name"));
        assert!(display.contains("Dangerous command"));
        assert!(display.contains("line 5"));
        assert!(display.contains("line 8"));
    }

    #[test]
    fn test_lookup_error_display() {
        let error = AkaError::LookupError {
            lookup_name: "env".to_string(),
            key: "missing_key".to_string(),
            available_lookups: vec!["env".to_string(), "paths".to_string()],
            available_keys: vec!["home".to_string(), "path".to_string()],
            context: "processing alias 'test'".to_string(),
        };

        let display = error.to_string();
        assert!(display.contains("Lookup resolution failed"));
        assert!(display.contains("Lookup: env"));
        assert!(display.contains("Key: missing_key"));
        assert!(display.contains("Available lookups:"));
        assert!(display.contains("Available keys in env:"));
        assert!(display.contains("home"));
        assert!(display.contains("path"));
    }

    #[test]
    fn test_circular_reference_error_display() {
        let error = AkaError::CircularReferenceError {
            alias_chain: vec!["alias1".to_string(), "alias2".to_string(), "alias1".to_string()],
            context: "processing command 'alias1'".to_string(),
        };

        let display = error.to_string();
        assert!(display.contains("Circular reference detected"));
        assert!(display.contains("alias1 -> alias2 -> alias1"));
        assert!(display.contains("processing command 'alias1'"));
    }

    #[test]
    fn test_error_context_builder() {
        let context = ErrorContext::new("loading configuration")
            .with_file("/home/user/.config/aka/aka.yml")
            .with_context("during startup");

        let error = context.to_file_operation_error(eyre!("permission denied"));

        match error {
            AkaError::FileOperationError { operation, context, .. } => {
                assert_eq!(operation, "loading configuration");
                assert_eq!(context, "during startup");
            }
            _ => panic!("Wrong error type"),
        }
    }

    #[test]
    fn test_yaml_position_extraction() {
        let error_msg = "YAML parsing error at line 10, column 5";
        let (line, column) = extract_yaml_position(error_msg);
        assert_eq!(line, Some(10));
        assert_eq!(column, Some(5));

        let error_msg2 = "Invalid YAML on line 3";
        let (line2, column2) = extract_yaml_position(error_msg2);
        assert_eq!(line2, Some(3));
        assert_eq!(column2, None);
    }
}
