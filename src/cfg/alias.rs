//use anyhow::Result;
use eyre::Result;
use serde::Deserialize;
use void::Void;
use std::str::FromStr;
use itertools::Itertools;
use regex::Regex;

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
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
    pub fn positionals(&self) -> Vec<String> {
        let re = Regex::new(r"(\$[1-9])").unwrap();
        re.find_iter(&self.value)
            .filter_map(|m| m.as_str().parse().ok())
            .unique()
            .sorted()
            .collect()
    }

    pub fn keywords(&self) -> Vec<String> {
        let re = Regex::new(r"(\$[A-z]+)").unwrap();
        re.find_iter(&self.value)
            .filter_map(|m| m.as_str().parse().ok())
            .unique()
            .sorted()
            .collect()
    }

    pub fn is_variadic(&self) -> bool {
        self.value.contains("$@")
    }

    pub fn replace(&self, remainders: &mut Vec<String>) -> (String, usize) {
        let mut result = self.value.to_owned();
        let mut count = 0;
        if self.positionals().len() == remainders.len() {
            for positional in self.positionals().iter() {
                result = result.replace(positional, &remainders.swap_remove(0));
            count = self.positionals().len();
            }
        }
        else if self.is_variadic() {
            result = result.replace("$@", &remainders.join(" "));
            count = remainders.len();
            remainders.drain(0..remainders.len());
        }
        else {
            result = self.name.to_owned();
        }
        (result, count)
    }
}

impl FromStr for Alias {
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Alias {
            name: "".to_owned(),
            value: s.to_owned(),
            space: true,
            global: false,
        })
    }
}

