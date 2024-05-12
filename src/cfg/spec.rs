// src/cfg/spec.rs

use eyre::Result;
use serde::{Deserialize, Deserializer};
use serde::de::{MapAccess,Visitor, Error as SerdeError};
use std::collections::HashMap;
use std::fmt;
use shlex::split;

use super::alias::Alias;

static FIELDS: &'static [&'static str] = &["value", "space", "global"];
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
}


fn deserialize_alias_map<'de, D>(deserializer: D) -> Result<Aliases, D::Error>
where
    D: Deserializer<'de>,
{
    struct AliasMap;

    impl<'de> Visitor<'de> for AliasMap {
        type Value = Aliases;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of names to aliases")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut aliases = Aliases::new();
            while let Some((name, alias)) = map.next_entry::<String, Alias>()? {
                aliases.insert(name.clone(), alias);
            }
            Ok(aliases)
        }
    }

    deserializer.deserialize_map(AliasMap)
}

impl<'de> Deserialize<'de> for Alias {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AliasVisitor;

        impl<'de> Visitor<'de> for AliasVisitor {
            type Value = Alias;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or a map")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                Ok(Alias {
                    name: String::new(),
                    value: split(value).unwrap_or_else(|| vec![value.to_string()]),
                    space: true,
                    global: false,
                })
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut value = None;
                let mut space = true;
                let mut global = false;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "value" => {
                            let v: String = map.next_value()?;
                            value = Some(shlex::split(&v).unwrap_or_else(|| v.split_whitespace().map(String::from).collect()));
                        },
                        "space" => space = map.next_value()?,
                        "global" => global = map.next_value()?,
                        _ => return Err(M::Error::unknown_field(&key, FIELDS)),
                    }
                }

                Ok(Alias {
                    name: String::new(),
                    value: value.ok_or_else(|| M::Error::missing_field("value"))?,
                    space,
                    global,
                })
            }
        }

        deserializer.deserialize_any(AliasVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vos;

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
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;
        let expect = vos!["echo", "Hello", "World"];
        assert_eq!(spec.defaults.version, 1);
        assert_eq!(spec.aliases.len(), 1);
        assert_eq!(spec.aliases.get("alias1").unwrap().value, expect);

        Ok(())
    }

    #[test]
    fn test_deserialize_alias_map_empty_file() -> Result<(), eyre::Error> {
        let yaml = r#"{}"#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.defaults.version, 1);
        assert_eq!(spec.aliases.len(), 0);

        Ok(())
    }

    #[test]
    fn test_deserialize_alias_map_invalid_content() {
        let yaml = r#"invalid YAML content"#;
        let result: Result<Spec, _> = serde_yaml::from_str(yaml);

        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_alias_map_with_string_alias() -> Result<(), eyre::Error> {
        let yaml = r#"
defaults:
  version: 1
aliases:
  alias1: "echo Hello World"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;
        let expect = vos!["echo", "Hello", "World"];
        assert_eq!(spec.defaults.version, 1);
        assert_eq!(spec.aliases.len(), 1);
        assert_eq!(spec.aliases.get("alias1").unwrap().value, expect);

        Ok(())
    }

    #[test]
    fn test_gc_alias_with_quotation() -> Result<(), eyre::Error> {
        let yaml = r#"
    defaults:
        version: 1
    aliases:
        gc:
            value: 'git commit -m"'
            space: false
        "#;

        let spec: Spec = serde_yaml::from_str(yaml)?;
        let gc_alias = spec.aliases.get("gc").unwrap();

        assert_eq!(gc_alias.value, vec!["git", "commit", "-m\""]);
        assert_eq!(gc_alias.space, false);
        Ok(())
    }

    #[test]
    fn test_alias_with_unmatched_quote() -> Result<()> {
        let yaml = r#"---
            value: "git commit -m\""
            space: false
        "#;
        let alias: Alias = serde_yaml::from_str(yaml)?;
        assert_eq!(alias.value, vec!["git", "commit", "-m\""]);
        Ok(())
    }

    #[test]
    fn test_alias_without_quotes() -> Result<()> {
        let yaml = r#"---
            value: "git commit -a -m Fix the issue"
            space: true
        "#;
        let alias: Alias = serde_yaml::from_str(yaml)?;
        assert_eq!(alias.value, vec!["git", "commit", "-a", "-m", "Fix", "the", "issue"]);
        Ok(())
    }
}
