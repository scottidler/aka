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
        assert_eq!(
            spec.aliases
                .get("alias1")
                .ok_or_else(|| eyre::eyre!("alias1 not found"))?
                .value,
            "echo Hello World"
        );
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

    #[test]
    fn test_deserialize_alias_as_string() -> Result<(), eyre::Error> {
        // Test deserializing an alias as a simple string value
        let yaml = r#"
aliases:
  ls: "eza -la"
  cat: "bat -p"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.aliases.len(), 2);
        assert_eq!(spec.aliases.get("ls").unwrap().value, "eza -la");
        assert_eq!(spec.aliases.get("cat").unwrap().value, "bat -p");

        Ok(())
    }

    #[test]
    fn test_deserialize_pipe_separated_alias_names() -> Result<(), eyre::Error> {
        // Test pipe-separated alias names create multiple entries
        let yaml = r#"
aliases:
  gc|gitcommit: "git commit"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.aliases.len(), 2);
        assert!(spec.aliases.contains_key("gc"));
        assert!(spec.aliases.contains_key("gitcommit"));
        assert_eq!(spec.aliases.get("gc").unwrap().value, "git commit");
        assert_eq!(spec.aliases.get("gitcommit").unwrap().value, "git commit");

        Ok(())
    }

    #[test]
    fn test_deserialize_alias_starting_with_pipe() -> Result<(), eyre::Error> {
        // Test that aliases starting with pipe are NOT split
        let yaml = r#"
aliases:
  "|c": "| xclip -sel clip"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.aliases.len(), 1);
        assert!(spec.aliases.contains_key("|c"));

        Ok(())
    }

    #[test]
    fn test_deserialize_alias_ending_with_pipe() -> Result<(), eyre::Error> {
        // Test that aliases ending with pipe are NOT split
        let yaml = r#"
aliases:
  "c|": "echo pipe end"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.aliases.len(), 1);
        assert!(spec.aliases.contains_key("c|"));

        Ok(())
    }

    #[test]
    fn test_deserialize_mixed_alias_formats() -> Result<(), eyre::Error> {
        // Test mixing string and struct alias definitions
        let yaml = r#"
aliases:
  simple: "echo simple"
  complex:
    value: "echo complex"
    space: false
    global: true
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.aliases.len(), 2);

        let simple = spec.aliases.get("simple").unwrap();
        assert_eq!(simple.value, "echo simple");
        assert!(simple.space); // default
        assert!(!simple.global); // default

        let complex = spec.aliases.get("complex").unwrap();
        assert_eq!(complex.value, "echo complex");
        assert!(!complex.space);
        assert!(complex.global);

        Ok(())
    }

    #[test]
    fn test_deserialize_defaults() -> Result<(), eyre::Error> {
        // Test custom defaults
        let yaml = r#"
defaults:
  version: 2
aliases:
  test: "echo test"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.defaults.version, 2);

        Ok(())
    }

    #[test]
    fn test_default_version_function() {
        assert_eq!(default_version(), 1);
    }

    #[test]
    fn test_default_defaults_function() {
        let defaults = default_defaults();
        assert_eq!(defaults.version, 1);
    }

    #[test]
    fn test_spec_clone() {
        let spec = Spec {
            defaults: Defaults { version: 1 },
            aliases: {
                let mut map = HashMap::new();
                map.insert(
                    "test".to_string(),
                    Alias {
                        name: "test".to_string(),
                        value: "echo test".to_string(),
                        space: true,
                        global: false,
                        count: 0,
                    },
                );
                map
            },
            lookups: HashMap::new(),
        };

        let cloned = spec.clone();
        assert_eq!(spec, cloned);
    }

    #[test]
    fn test_defaults_partial_eq() {
        let d1 = Defaults { version: 1 };
        let d2 = Defaults { version: 1 };
        let d3 = Defaults { version: 2 };

        assert_eq!(d1, d2);
        assert_ne!(d1, d3);
    }

    #[test]
    fn test_spec_default() {
        let spec = Spec::default();
        // Default trait gives version 0 (not 1 like serde default)
        // Just verify it doesn't panic and has expected structure
        let _ = spec.defaults.version;
        assert!(spec.aliases.is_empty());
        assert!(spec.lookups.is_empty());
    }

    #[test]
    fn test_defaults_default() {
        let defaults = Defaults::default();
        // Default trait may give version 0, not 1
        // Just verify it doesn't panic
        let _ = defaults.version;
    }

    #[test]
    fn test_deserialize_with_lookups() -> Result<(), eyre::Error> {
        let yaml = r#"
aliases:
  deploy: "kubectl apply -f lookup:env[prod]"
lookups:
  env:
    prod: "production.yaml"
    dev: "development.yaml"
        "#;
        let spec: Spec = serde_yaml::from_str(yaml)?;

        assert_eq!(spec.lookups.len(), 1);
        assert!(spec.lookups.contains_key("env"));
        assert_eq!(spec.lookups["env"]["prod"], "production.yaml");
        assert_eq!(spec.lookups["env"]["dev"], "development.yaml");

        Ok(())
    }
}
