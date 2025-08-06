use eyre::Result;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use std::collections::{HashMap, HashSet};
use log::debug;

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

const fn default_zero() -> u64 {
    0
}

fn deserialize_trimmed_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(s.trim().to_string())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Alias {
    #[serde(skip_deserializing)]
    pub name: String,

    #[serde(deserialize_with = "deserialize_trimmed_string")]
    pub value: String,

    #[serde(default = "default_true")]
    pub space: bool,

    #[serde(default = "default_false")]
    pub global: bool,

    #[serde(default = "default_zero")]
    pub count: u64,
}

impl Alias {
    /// Return the positional arguments
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem in processing the positional arguments.
    pub fn positionals(&self) -> Result<Vec<String>> {
        let re = Regex::new(r"(\$[1-9])")?;
        let mut items: Vec<String> = re
            .find_iter(&self.value)
            .filter_map(|m| m.as_str().parse().ok())
            .collect();
        items.sort();
        items.dedup();
        Ok(items)
    }

    /// Return variable references (alias names referenced with $aliasname)
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem in processing the variable references.
    pub fn variable_references(&self) -> Result<Vec<String>> {
        // Match $word but exclude $1-9 and $@
        let re = Regex::new(r"\$([A-Za-z][A-Za-z0-9_-]*)")?;
        let mut items: Vec<String> = re
            .captures_iter(&self.value)
            .map(|cap| cap[1].to_string()) // Get the variable name without $
            .collect();
        items.sort();
        items.dedup();
        Ok(items)
    }

    #[must_use]
    pub fn is_variadic(&self) -> bool {
        self.value.contains("$@")
    }

    /// Interpolate variable references in the alias value
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem resolving variable references.
    fn interpolate_variables(
        &self,
        alias_map: &HashMap<String, Alias>,
        resolution_stack: &mut HashSet<String>,
    ) -> Result<String> {
        let mut result = self.value.clone();
        let variable_refs = self.variable_references()?;

        for var_name in variable_refs {
            let placeholder = format!("${}", var_name);

            // Check for cycle
            if resolution_stack.contains(&var_name) {
                debug!("🔄 CYCLE DETECTED: {} -> {} (skipping)", self.name, var_name);
                continue; // Leave $var_name as-is
            }

            // Find the referenced alias
            if let Some(target_alias) = alias_map.get(&var_name) {
                // Add to resolution stack
                resolution_stack.insert(var_name.clone());

                // Recursively resolve the target alias
                let resolved_value = target_alias.interpolate_variables(alias_map, resolution_stack)?;

                // Replace the placeholder
                result = result.replace(&placeholder, &resolved_value);

                // Remove from resolution stack
                resolution_stack.remove(&var_name);

                debug!("🔧 VARIABLE INTERPOLATION: '{}' -> '{}'", placeholder, resolved_value);
            } else {
                debug!("🔍 VARIABLE NOT FOUND: {} (leaving as-is)", var_name);
                // Leave $var_name as-is if alias doesn't exist
            }
        }

        Ok(result)
    }

    /// Replace the remainder of the arguments.
    ///
    /// # Errors
    ///
    /// Returns `Err` under the following conditions:
    /// - If there was a problem retrieving positional parameters.
    /// - If the alias is not variadic and the number of positional parameters doesn't match the number of remaining arguments.
    /// - If there was a problem with variable interpolation.
    pub fn replace(&self, remainders: &mut Vec<String>, alias_map: &HashMap<String, Alias>, eol: bool) -> Result<(String, usize)> {
        // Step 1: Variable interpolation (always happens)
        let mut resolution_stack = HashSet::new();
        resolution_stack.insert(self.name.clone());
        let mut result = self.interpolate_variables(alias_map, &mut resolution_stack)?;

        // Step 2: Positional argument replacement
        let mut count = 0;
        let positionals = self.positionals()?;
        if positionals.len() > 0 {
            if positionals.len() == remainders.len() {
                for positional in &positionals {
                    result = result.replace(positional, &remainders.swap_remove(0));
                }
                count = positionals.len();
            } else {
                result = self.name.clone();
            }
        } else if result.contains("$@") && eol {
            // Step 3: Variadic argument replacement (only when eol=true)
            result = result.replace("$@", &remainders.join(" "));
            count = remainders.len();
            remainders.drain(0..remainders.len());
        }

        Ok((result, count))
    }
}

impl FromStr for Alias {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            name: String::new(),
            value: s.trim().to_owned(),
            space: true,
            global: false,
            count: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positionals() -> Result<()> {
        let alias = Alias {
            name: "alias1".to_string(),
            value: "echo $1 $2".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        assert_eq!(alias.positionals()?, vec!["$1", "$2"]);
        Ok(())
    }

    #[test]
    fn test_variable_references() -> Result<()> {
        let alias = Alias {
            name: "alias2".to_string(),
            value: "echo $name $location".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        assert_eq!(alias.variable_references()?, vec!["location", "name"]);
        Ok(())
    }

    #[test]
    fn test_is_variadic() {
        let alias = Alias {
            name: "alias3".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        assert!(alias.is_variadic());
    }

    #[test]
    fn test_replace() -> Result<()> {
        let alias = Alias {
            name: "alias4".to_string(),
            value: "echo $1 $2".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        let aliases = HashMap::new();
        assert_eq!(alias.replace(&mut remainders, &aliases, true)?, ("echo Hello World".to_string(), 2));
        assert_eq!(remainders, Vec::<String>::new()); // Corrected this line

        let alias_variadic = Alias {
            name: "alias5".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        let mut remainders_variadic = vec!["Hello".to_string(), "from".to_string(), "Rust".to_string()];
        assert_eq!(
            alias_variadic.replace(&mut remainders_variadic, &aliases, true)?,
            ("echo Hello from Rust".to_string(), 3)
        );
        assert_eq!(remainders_variadic, Vec::<String>::new()); // Corrected this line

        Ok(())
    }

    #[test]
    fn test_from_str() -> Result<()> {
        let s = "echo Hello World";
        let alias = s.parse::<Alias>()?;
        assert_eq!(alias.name, "");
        assert_eq!(alias.value, s);
        assert!(alias.space);
        assert!(!alias.global);
        assert_eq!(alias.count, 0);
        Ok(())
    }

    #[test]
    fn test_no_arguments() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo Hello World".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        assert_eq!(alias.positionals()?, Vec::<String>::new());
        assert_eq!(alias.variable_references()?, Vec::<String>::new());
        assert_eq!(alias.count, 0);
        Ok(())
    }

    #[test]
    fn test_replace_no_positional_with_variadic() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        let aliases = HashMap::new();
        assert_eq!(alias.replace(&mut remainders, &aliases, true)?, ("echo Hello World".to_string(), 2));
        assert_eq!(remainders, Vec::<String>::new());
        assert_eq!(alias.count, 0); // Count is not modified by the replace method itself
        Ok(())
    }

    #[test]
    fn test_replace_mismatch_remainders() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo $1 $2 $3".to_string(),
            space: true,
            global: false,
            count: 0,
        };

        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        let aliases = HashMap::new();
        assert_eq!(alias.replace(&mut remainders, &aliases, true)?, ("alias".to_string(), 0)); // Alias name is returned when not enough arguments.
        assert_eq!(alias.count, 0);
        Ok(())
    }
}
