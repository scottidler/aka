#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use eyre::Result;
use serde::Deserialize;
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Alias {
    #[serde(skip_deserializing)]
    pub name: String,

    pub value: String,

    #[serde(default = "default_true")]
    pub space: bool,

    #[serde(default = "default_false")]
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
        let items = re.find_iter(&self.value)
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
        let re = Regex::new(r"(\$[A-z]+)")?;
        let items = re.find_iter(&self.value)
            .filter_map(|m| m.as_str().parse().ok())
            .unique()
            .sorted()
            .collect();
        Ok(items)
    }

    #[must_use]
    pub fn is_variadic(&self) -> bool {
        self.value.contains("$@")
    }

    /// Replace the remainder of the arguments.
    ///
    /// # Errors
    ///
    /// Returns `Err` under the following conditions:
    /// - If there was a problem retrieving positional parameters.
    /// - If the alias is not variadic and the number of positional parameters doesn't match the number of remaining arguments.
    pub fn replace(&self, remainders: &mut Vec<String>) -> Result<(String, usize)> {
        let mut result = self.value.clone();
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
        }
        else if self.is_variadic() {
            result = result.replace("$@", &remainders.join(" "));
            count = remainders.len();
            remainders.drain(0..remainders.len());
        }
        Ok((result, count))
    }

    pub fn replace2(&self, remainders: &mut Vec<String>) -> Result<(Vec<String>, usize)> {
        debug!("Alias::replace2: remainders={:?}", remainders);
        let mut value: Vec<String> = split(&self.value).expect("Failed to split the alias value.");
        let mut count = 0;
        debug!("value={:?}", value);
        let positionals = self.positionals()?;
        debug!("positionals={:?}", positionals);

        if positionals.len() > 0 {
            if positionals.len() == remainders.len() {
                debug!("Positionals and remainders are of equal length.");
                for (index, positional) in positionals.iter().enumerate() {
                    debug!("index={} positional={}", index, positional);
                    let indices = value.iter().enumerate().filter_map(|(i, item)| {
                        debug!("i={} item={}", i, item);
                        if item == positional { Some(i) } else { None }
                    }).collect::<Vec<usize>>();
                    debug!("indices={:?}", indices);
                    let item = remainders[index].clone();
                    debug!("item={}", item);
                    for i in indices {
                        debug!("i={}", i);
                        value[i] = item.clone();
                        debug!("value={:?}", value);
                    }
                }
                count = positionals.len();
                debug!("count={}", count);
                debug!("remainders={:?} to be cleared", remainders);
                remainders.clear();
                debug!("Alias: (value={:?} count={}) to be returned", value, count);
            } else {
                debug!("Mismatch in the number of positionals and remainders.");
                value = vec![self.name.clone()];
                debug!("Defaulting to alias name due to mismatch, value={:?}", value);
            }
        } else if self.is_variadic() {
            debug!("is_variadic");
            let index = value.iter().position(|r| r == "$@").unwrap();
            debug!("index={}", index);
            value.remove(index);
            value.splice(index..index, remainders.iter().cloned());
            debug!("after removing '$@' and splicing remainders={:?} into value={:?}", remainders, value);
            count = remainders.len();
            debug!("count={}", count);
            remainders.clear();
            debug!("(value={:?} count={}) to be returned", value, count);
        }
        Ok((value, count))
    }
}

impl FromStr for Alias {
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            name: String::new(),
            value: s.to_owned(),
            space: true,
            global: false,
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
        };

        assert_eq!(alias.positionals()?, vec!["$1", "$2"]);
        Ok(())
    }

    #[test]
    fn test_keywords() -> Result<()> {
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
    fn test_is_variadic() {
        let alias = Alias {
            name: "alias3".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
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
        };
    
        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        assert_eq!(alias.replace(&mut remainders)?, ("echo Hello World".to_string(), 2));
        assert_eq!(remainders, Vec::<String>::new());
    
        let alias_variadic = Alias {
            name: "alias5".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
        };
    
        let mut remainders_variadic = vec!["Hello".to_string(), "from".to_string(), "Rust".to_string()];
        assert_eq!(alias_variadic.replace(&mut remainders_variadic)?, ("echo Hello from Rust".to_string(), 3));
        assert_eq!(remainders_variadic, Vec::<String>::new());
    
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
        Ok(())
    }
   
    #[test]
    fn test_no_arguments() -> Result<()> {
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
    fn test_replace_no_positional_with_variadic() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo $@".to_string(),
            space: true,
            global: false,
        };
    
        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        assert_eq!(alias.replace(&mut remainders)?, ("echo Hello World".to_string(), 2));
        assert_eq!(remainders, Vec::<String>::new());
        Ok(())
    }
   
    #[test]
    fn test_replace_mismatch_remainders() -> Result<()> {
        let alias = Alias {
            name: "alias".to_string(),
            value: "echo $1 $2 $3".to_string(),
            space: true,
            global: false,
        };
    
        let mut remainders = vec!["Hello".to_string(), "World".to_string()];
        assert_eq!(alias.replace(&mut remainders)?, ("alias".to_string(), 0));
        Ok(())
    }
}

#[cfg(test)]
mod tests2 {
    use super::*;

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
}
