// src/cfg/alias.rs

use eyre::Result;
use itertools::Itertools;
use regex::Regex;
use log::debug;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alias {
    pub name: String,
    pub value: Vec<String>,
    pub space: bool,
    pub global: bool,
}

impl Alias {
    /// Return the positional arguments
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem in processing the positional arguments.
    pub fn positionals(&self) -> Result<Vec<String>> {
        let re = Regex::new(r"(\$[1-9])")?;
        let items = self.value.iter()
            .flat_map(|s| re.find_iter(s))
            .filter_map(|m| m.as_str().parse().ok())
            .unique()
            .sorted()
            .collect();
        Ok(items)
    }

    /// Return the keyword arguments
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem in processing the keyword arguments.
    pub fn keywords(&self) -> Result<Vec<String>> {
        let re = Regex::new(r"(\$[A-Za-z]+)")?;
        let items = self.value.iter()
            .flat_map(|s| re.find_iter(s))
            .filter_map(|m| m.as_str().parse().ok())
            .unique()
            .sorted()
            .collect();
        Ok(items)
    }

    #[must_use]
    pub fn is_variadic(&self) -> bool {
        self.value.iter().any(|s| s.contains("$@"))
    }

    pub fn replace(&self, remainders: &mut Vec<String>) -> Result<(Vec<String>, usize)> {
        let mut value = self.value.clone();
        let mut count = 0;
        let positionals = self.positionals()?;

        if positionals.len() > 0 {
            if positionals.len() == remainders.len() {
                value = value.into_iter().map(|part| {
                    let mut modified_part = part.clone();
                    for (positional, replacement) in positionals.iter().zip(remainders.iter()) {
                        modified_part = modified_part.replace(positional, replacement);
                    }
                    modified_part
                }).collect();
                count = positionals.len();
                remainders.clear();
            } else {
                value = vec![self.name.clone()];
            }
        } else if self.is_variadic() {
            let mut i = 0;
            while i < value.len() {
                if value[i] == "$@" {
                    value.splice(i..i + 1, remainders.iter().cloned());
                    count = remainders.len();
                    remainders.clear();
                    break;
                }
                i += 1;
            }
        }
        debug!("about return (value={:?}, count={}) remainders={:?}", value, count, remainders);
        Ok((value, count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use crate::vos;

    #[test]
    fn test_positionals() -> Result<()> {
        let alias = Alias {
            name: "alias1".to_string(),
            value: vos!["echo", "$1", "$2"],
            space: true,
            global: false,
        };
        assert_eq!(alias.positionals()?, vec!["$1", "$2"]);
        Ok(())
    }
}
