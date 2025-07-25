use eyre::Result;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use super::alias::Alias;

type Aliases = HashMap<String, Alias>;

const fn default_version() -> i32 {
    1
}

const fn default_defaults() -> Defaults {
    Defaults {
        version: default_version(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Defaults {
    #[serde(default = "default_version")]
    pub version: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Spec {
    #[serde(default = "default_defaults")]
    pub defaults: Defaults,

    #[serde(default, deserialize_with = "deserialize_alias_map")]
    pub aliases: Aliases,

    #[serde(default)]
    pub lookups: HashMap<String, HashMap<String, String>>,
}

fn deserialize_alias_map<'de, D>(deserializer: D) -> Result<Aliases, D::Error>
where
    D: Deserializer<'de>,
{
    struct AliasMap;

    struct AliasVisitor;
    impl<'de> Visitor<'de> for AliasVisitor {
        type Value = Alias;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string or map")
        }

        fn visit_str<E>(self, string: &str) -> Result<Alias, E>
        where
            E: de::Error,
        {
            Alias::from_str(string).map_err(|_| E::custom("Unexpected Error"))
        }

        fn visit_map<M>(self, map: M) -> Result<Alias, M::Error>
        where
            M: MapAccess<'de>,
        {
            Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }

    fn alias_string_or_struct<'de, D>(deserializer: D) -> Result<Alias, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(AliasVisitor)
    }

    #[derive(Debug, Deserialize)]
    struct AliasStringOrStruct(#[serde(deserialize_with = "alias_string_or_struct")] Alias);

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
                let names = if name.starts_with('|') || name.ends_with('|') {
                    vec![&name[..]]
                } else {
                    name.split('|').collect::<Vec<&str>>()
                };

                for name in names {
                    let name = name.to_string();
                    alias.name = name.clone();
                    aliases.insert(name.clone(), alias.clone());
                }
            }
            Ok(aliases)
        }
    }
    deserializer.deserialize_map(AliasMap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_alias_map_success() -> Result<(), eyre::Error> {
        let yaml = r#"
defaults:
  version: 1
aliases:
  alias1:
    value: "echo Hello World"
    space: true
    global: false
lookups:
  region:
    prod|apps: us-east-1
    staging|test|dev|ops: us-west-2
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.defaults.version, 1);
        assert_eq!(spec.aliases.len(), 1);
        assert_eq!(spec.aliases.get("alias1").ok_or_else(|| eyre::eyre!("alias1 not found"))?.value, "echo Hello World");
        assert_eq!(spec.lookups["region"]["prod|apps"], "us-east-1");
        assert_eq!(spec.lookups["region"]["staging|test|dev|ops"], "us-west-2");

        Ok(())
    }

    #[test]
    fn test_deserialize_empty_file() -> Result<(), eyre::Error> {
        let yaml = r#"{}"#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.defaults.version, 1); // default value
        assert_eq!(spec.aliases.len(), 0);
        assert!(spec.lookups.is_empty());

        Ok(())
    }
}
