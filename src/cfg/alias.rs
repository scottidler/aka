use anyhow::Result;
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
            .collect()
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

