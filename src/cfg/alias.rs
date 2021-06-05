use anyhow::Result;
use serde::Deserialize;
use void::Void;
use std::str::FromStr;

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Alias {
    #[serde(skip_deserializing)]
    pub name: String,

    pub value: String,

    #[serde(default = "default_false")]
    pub first: bool,

    #[serde(default = "default_true")]
    pub expand: bool,

    #[serde(default = "default_true")]
    pub space: bool,
}

impl FromStr for Alias {
    type Err = Void;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Alias {
            name: "".to_owned(),
            value: s.to_owned(),
            first: false,
            expand: true,
            space: true,
        })
    }
}

