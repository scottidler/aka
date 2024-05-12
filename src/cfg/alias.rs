// src/cfg/alias.rs

#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use eyre::Result;
use serde::{Deserialize, Deserializer};
use serde::de::{self, Visitor};
use std::fmt;
use void::Void;
use std::str::FromStr;
use itertools::Itertools;
use regex::Regex;
use shlex::split;
use log::{info, debug, warn, error};

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

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

    pub fn replace2(&self, remainders: &mut Vec<String>) -> Result<(Vec<String>, usize)> {
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
mod tests2 {
    use super::*;
    use pretty_assertions::assert_eq;
    use crate::vos;

    #[test]
    fn test_positionals2() -> Result<()> {
        let alias = Alias {
            name: "alias1".to_string(),
            value: "echo $1 $2".to_string(),
            space: true,
            global: false,
        };

        assert_eq!(alias.positionals()?, vec!["$1", "$2"]);
        Ok(())
    }

    #[test]
    fn test_keywords2() -> Result<()> {
        let alias = Alias {
            name: "alias2".to_string(),
            value: "echo $name $location".to_string(),
            space: true,
            global: false,
        };

        assert_eq!(alias.keywords()?, vec!["$location", "$name"]);
        Ok(())
    }

    #[test]
    fn test_is_variadic2() {
        let alias = Alias {
            name: "alias3".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
        };

        assert!(alias.is_variadic());
    }

    #[test]
    fn test_replace2_new() -> Result<()> {
        let alias = Alias {
            name: "alias4".to_string(),
            value: "echo $1 $2".to_string(),
            space: true,
            global: false,
        };

        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        let (result, count) = alias.replace2(&mut remainders)?;
        assert_eq!(result, vec!["echo", "Hello", "World"]);
        assert_eq!(count, 2);
        assert_eq!(remainders, Vec::<String>::new());

        let alias_variadic = Alias {
            name: "alias5".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
        };

        let mut remainders_variadic = vec!["Hello".to_string(), "from".to_string(), "Rust".to_string()];
        let (result_variadic, count_variadic) = alias_variadic.replace2(&mut remainders_variadic)?;
        assert_eq!(result_variadic, vec!["echo", "Hello", "from", "Rust"]);
        assert_eq!(count_variadic, 3);
        assert_eq!(remainders_variadic, Vec::<String>::new());

        Ok(())
    }

    #[test]
    fn test_from_str2() -> Result<()> {
        let s = "echo Hello World";
        let alias = s.parse::<Alias>()?;
        assert_eq!(alias.name, "");
        assert_eq!(alias.value, s);
        assert!(alias.space);
        assert!(!alias.global);
        Ok(())
    }

    #[test]
    fn test_no_arguments2() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo Hello World".to_string(),
            space: true,
            global: false,
        };

        assert_eq!(alias.positionals()?, Vec::<String>::new());
        assert_eq!(alias.keywords()?, Vec::<String>::new());
        Ok(())
    }

    #[test]
    fn test_replace_no_positional_with_variadic2() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
        };

        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        let (result, count) = alias.replace2(&mut remainders)?;
        assert_eq!(result, vec!["echo", "Hello", "World"]);
        assert_eq!(count, 2);
        assert_eq!(remainders, Vec::<String>::new());
        Ok(())
    }

    #[test]
    fn test_replace_mismatch_remainders2() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo $1 $2 $3".to_string(),
            space: true,
            global: false,
        };

        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        let (result, count) = alias.replace2(&mut remainders)?;
        assert_eq!(result, vec!["alias"]);
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn test_single_simple_substitution() -> Result<()> {
        let alias = Alias {
            name: "printr".to_string(),
            value: "print -r -- =$1".to_string(),
            space: true,
            global: false,
        };

        let cmdline = vos!["printr", "variable"];
        let expected_result = vos!["print", "-r", "--", "=variable"];
        let result = alias.replace2(&mut cmdline[1..].to_vec())?;
        assert_eq!(result, (expected_result, 1));
        Ok(())
    }

    #[test]
    fn test_double_positional_substitution() -> Result<()> {
        let alias = Alias {
            name: "pp".to_string(),
            value: "prepend $1 $2".to_string(),
            space: true,
            global: false,
        };

        let cmdline = vos!["pp", "text", "file.txt"];
        let expected_result = vos!["prepend", "text", "file.txt"];
        let result = alias.replace2(&mut cmdline[1..].to_vec())?;
        assert_eq!(result, (expected_result, 2));
        Ok(())
    }

    #[test]
    fn test_repeated_positional_substitution() -> Result<()> {
        let alias = Alias {
            name: "sorted".to_string(),
            value: "bat -p $1 | sort -u > $1.new; mv $1.new $1".to_string(),
            space: true,
            global: false,
        };

        let cmdline = vos!["sorted", "file.txt"];
        let expected_result = vos!["bat", "-p", "file.txt", "|", "sort", "-u", ">", "file.txt.new;", "mv", "file.txt.new", "file.txt"];
        let result = alias.replace2(&mut cmdline[1..].to_vec())?;
        assert_eq!(result, (expected_result, 1));
        Ok(())
    }
}
