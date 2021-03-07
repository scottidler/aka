use anyhow::{
    anyhow,
    Result,
};
use super::error::ConfigError;
use std::marker::PhantomData;
use serde::{Deserialize, Deserializer};
use serde::de::{self, MapAccess, SeqAccess, Visitor, Error};
use std::collections::HashMap;
use std::fmt;
use std::vec::Vec;
use std::num::ParseIntError;
use std::str::FromStr;
use void::Void;

type Aliases = HashMap<String, Alias>;

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

fn default_version() -> i32 {
    1
}

fn default_defaults() -> Defaults {
    Defaults {
        version: default_version(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Defaults {
    #[serde(default = "default_version")]
    pub version: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Spec {
    #[serde(default = "default_defaults")]
    pub defaults: Defaults,

    #[serde(default, deserialize_with = "deserialize_alias_map")]
    pub aliases: Aliases,
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
/*
struct StringOrStruct<T>(PhantomData<fn ()-> T>);
impl <'de, T> Visitor<'de> for StringOrStruct<T>
where
    T: Deserialize<'de> + FromStr<Err = Void>,
{
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("Alias: string or struct")
    }

    fn visit_str<E>(self, value: &str) -> Result<T, E>
    where
        E: de::Error,
    {
        Ok(FromStr::from_str(value).unwrap())
    }

    fn visit_map<M>(self, map: M) -> Result<T, M::Error>
    where
        M: MapAccess<'de>,
    {
        Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))
    }
}
*/
fn deserialize_alias_map<'de, D>(deserializer: D) -> Result<Aliases, D::Error>
where
    D: Deserializer<'de>,
{
    struct AliasMap;

    struct AliasVisitor;
    impl<'de> Visitor<'de> for AliasVisitor
    {
        type Value = Alias;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string or map")
        }

        fn visit_str<E>(self, string: &str) -> Result<Alias, E>
        where
            E: de::Error,
        {
            Ok(FromStr::from_str(string).unwrap())
        }

        fn visit_map<M>(self, map: M) -> Result<Alias, M::Error>
        where
            M: MapAccess<'de>
        {
            Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }

    fn alias_string_or_struct<'de, D>(deserializer: D) -> Result<Alias, D::Error>
        where D: Deserializer<'de> {
        deserializer.deserialize_any(AliasVisitor)
    }

    #[derive(Debug, Deserialize)]
    struct AliasStringOrStruct(#[serde(deserialize_with="alias_string_or_struct")] Alias);

    impl<'de> Visitor<'de> for AliasMap {
        type Value = Aliases;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of name to Alias")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut aliases = Aliases::new();
            while let Some((name, AliasStringOrStruct(mut alias))) = map.next_entry::<String, AliasStringOrStruct>()? {
                alias.name = name.to_owned();
                aliases.insert(name.to_owned(), alias);
            }
            Ok(aliases)
        }
    }
    deserializer.deserialize_map(AliasMap)
}
