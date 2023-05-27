use eyre::Result;
use serde::Deserialize;
use void::Void;
use std::str::FromStr;
use itertools::Itertools;
use regex::Regex;

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
    pub fn positionals(&self) -> Result<Vec<String>> {
        let re = Regex::new(r"(\$[1-9])")?;
        let items = re.find_iter(&self.value)
            .filter_map(|m| m.as_str().parse().ok())
            .unique()
            .sorted()
            .collect();
        Ok(items)
    }

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

    pub fn replace(&self, remainders: &mut Vec<String>) -> Result<(String, usize)> {
        let mut result = self.value.clone();
        let mut count = 0;
        let positionals = self.positionals()?;
        if positionals.len() == remainders.len() {
            for positional in &positionals {
                result = result.replace(positional, &remainders.swap_remove(0));
            count = positionals.len();
            }
        }
        else if self.is_variadic() {
            result = result.replace("$@", &remainders.join(" "));
            count = remainders.len();
            remainders.drain(0..remainders.len());
        }
        else {
            result = self.name.clone();
        }
        Ok((result, count))
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

