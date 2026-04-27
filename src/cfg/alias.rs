use eyre::Result;
use log::debug;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

// Matches all bash positional expansion forms for $1-$9 (no capture groups).
// Used for detection in positionals(); colorize_value() extends this with [@] for $@.
pub(crate) const POSITIONAL_RE: &str = r"\$(?:\{#?[1-9][^}]*\}|[1-9])";

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

const fn default_zero() -> u64 {
    0
}

fn deserialize_trimmed_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(s.trim().to_string())
}

/// Extract the positional digit character from a token matched by POSITIONAL_RE.
/// e.g. "$1" → '1', "${1:-.}" → '1', "${#1}" → '1'
fn positional_digit(token: &str) -> Option<char> {
    let rest = token.strip_prefix('$')?;
    if let Some(inner) = rest.strip_prefix('{') {
        let inner = inner.strip_prefix('#').unwrap_or(inner);
        inner.chars().next()
    } else {
        rest.chars().next()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Alias {
    #[serde(skip_deserializing)]
    pub name: String,

    #[serde(deserialize_with = "deserialize_trimmed_string")]
    pub value: String,

    #[serde(default = "default_true")]
    pub space: bool,

    #[serde(default = "default_false")]
    pub global: bool,

    #[serde(default = "default_zero")]
    pub count: u64,
}

impl Alias {
    /// Return the positional arguments
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem in processing the positional arguments.
    pub fn positionals(&self) -> Result<Vec<String>> {
        let re = Regex::new(POSITIONAL_RE)?;
        let mut by_digit: std::collections::BTreeMap<char, String> = std::collections::BTreeMap::new();
        for m in re.find_iter(&self.value) {
            let start = m.start();
            // Exclude $$N (escaped dollar, e.g. awk field references)
            if start > 0 && self.value.chars().nth(start - 1) == Some('$') {
                continue;
            }
            let token = m.as_str();
            let Some(digit) = positional_digit(token) else { continue };
            // Longest token per digit wins: prevents $1 inside ${1:-.} from being
            // corrupted by a naive str::replace on the shorter bare form.
            by_digit
                .entry(digit)
                .and_modify(|existing| {
                    if token.len() > existing.len() {
                        *existing = token.to_string();
                    }
                })
                .or_insert_with(|| token.to_string());
        }
        Ok(by_digit.into_values().collect())
    }

    /// Return variable references (alias names referenced with $aliasname)
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem in processing the variable references.
    pub fn variable_references(&self) -> Result<Vec<String>> {
        // Match $word but exclude $1-9 and $@
        let re = Regex::new(r"\$([A-Za-z][A-Za-z0-9_-]*)")?;
        let mut items: Vec<String> = re
            .captures_iter(&self.value)
            .map(|cap| cap[1].to_string()) // Get the variable name without $
            .collect();
        items.sort();
        items.dedup();
        Ok(items)
    }

    #[must_use]
    pub fn is_variadic(&self) -> bool {
        self.value.contains("$@")
    }

    /// Interpolate variable references in the alias value
    ///
    /// # Errors
    ///
    /// Will return `Err` if there was a problem resolving variable references.
    fn interpolate_variables(
        &self,
        alias_map: &HashMap<String, Alias>,
        resolution_stack: &mut HashSet<String>,
    ) -> Result<String> {
        let mut result = self.value.clone();
        let variable_refs = self.variable_references()?;

        for var_name in variable_refs {
            let placeholder = format!("${var_name}");

            // Check for cycle
            if resolution_stack.contains(&var_name) {
                debug!("🔄 CYCLE DETECTED: {} -> {} (skipping)", self.name, var_name);
                continue; // Leave $var_name as-is
            }

            // Find the referenced alias
            if let Some(target_alias) = alias_map.get(&var_name) {
                // Add to resolution stack
                resolution_stack.insert(var_name.clone());

                // Recursively resolve the target alias
                let resolved_value = target_alias.interpolate_variables(alias_map, resolution_stack)?;

                // Replace the placeholder
                result = result.replace(&placeholder, &resolved_value);

                // Remove from resolution stack
                resolution_stack.remove(&var_name);

                debug!("🔧 VARIABLE INTERPOLATION: '{placeholder}' -> '{resolved_value}'");
            } else {
                debug!("🔍 VARIABLE NOT FOUND: {var_name} (leaving as-is)");
                // Leave $var_name as-is if alias doesn't exist
            }
        }

        Ok(result)
    }

    /// Replace the remainder of the arguments.
    ///
    /// # Errors
    ///
    /// Returns `Err` under the following conditions:
    /// - If there was a problem retrieving positional parameters.
    /// - If the alias is not variadic and the number of positional parameters doesn't match the number of remaining arguments.
    /// - If there was a problem with variable interpolation.
    pub fn replace(
        &self,
        remainders: &mut Vec<String>,
        alias_map: &HashMap<String, Alias>,
        eol: bool,
    ) -> Result<(String, usize)> {
        // Step 1: Variable interpolation (always happens)
        let mut resolution_stack = HashSet::new();
        resolution_stack.insert(self.name.clone());
        let mut result = self.interpolate_variables(alias_map, &mut resolution_stack)?;

        // Step 2: Positional argument replacement
        let mut count = 0;
        let positionals = self.positionals()?;
        if !positionals.is_empty() {
            if positionals.len() == remainders.len() {
                for positional in &positionals {
                    result = result.replace(positional, &remainders.swap_remove(0));
                }
                count = positionals.len();
            } else {
                result = self.name.clone();
            }
        } else if result.contains("$@") {
            if eol {
                // Step 3: Variadic argument replacement (only when eol=true)
                result = result.replace("$@", &remainders.join(" "));
                count = remainders.len();
                remainders.drain(0..remainders.len());
            } else {
                // For variadic aliases when eol=false, return the original alias name to indicate no expansion
                result = self.name.clone();
                count = 0;
            }
        }

        // Step 4: Unescape $$ to $ (for shell variable/field references)
        result = result.replace("$$", "$");

        Ok((result, count))
    }
}

impl FromStr for Alias {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            name: String::new(),
            value: s.trim().to_owned(),
            space: true,
            global: false,
            count: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(value: &str) -> Alias {
        Alias {
            name: "a".to_string(),
            value: value.to_string(),
            space: true,
            global: false,
            count: 0,
        }
    }

    fn pos(value: &str) -> Vec<String> {
        mk(value).positionals().unwrap()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── Positive: bare forms ──────────────────────────────────────────────────

    #[test]
    fn test_positionals_bare_all_digits() -> Result<()> {
        for d in 1..=9u8 {
            let tok = format!("${d}");
            assert_eq!(pos(&format!("echo {tok}")), s(&[&tok]), "bare ${d}");
        }
        Ok(())
    }

    #[test]
    fn test_positionals_bare_multiple() -> Result<()> {
        assert_eq!(pos("cmd $1 $2"), s(&["$1", "$2"]));
        assert_eq!(pos("cmd $3 $1 $2"), s(&["$1", "$2", "$3"])); // sorted ascending
        Ok(())
    }

    #[test]
    fn test_positionals_bare_all_nine() -> Result<()> {
        assert_eq!(
            pos("$1 $2 $3 $4 $5 $6 $7 $8 $9"),
            s(&["$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9"])
        );
        Ok(())
    }

    // ── Positive: braced no-modifier ─────────────────────────────────────────

    #[test]
    fn test_positionals_braced_bare() -> Result<()> {
        assert_eq!(pos("cmd ${1}"), s(&["${1}"]));
        assert_eq!(pos("cmd ${9}"), s(&["${9}"]));
        Ok(())
    }

    // ── Positive: default-value operators ────────────────────────────────────

    #[test]
    fn test_positionals_braced_default_colon_dash() -> Result<()> {
        assert_eq!(pos("cmd ${1:-.}"), s(&["${1:-.}"]));
        assert_eq!(pos("cmd ${1:-}"), s(&["${1:-}"]));
        assert_eq!(pos("cmd ${1:-default}"), s(&["${1:-default}"]));
        Ok(())
    }

    #[test]
    fn test_positionals_braced_default_no_colon() -> Result<()> {
        // ${N-word}: use default only when unset (not merely null)
        assert_eq!(pos("cmd ${1-default}"), s(&["${1-default}"]));
        assert_eq!(pos("cmd ${1-}"), s(&["${1-}"]));
        Ok(())
    }

    // ── Positive: alternate-value operators ──────────────────────────────────

    #[test]
    fn test_positionals_braced_alternate_colon_plus() -> Result<()> {
        assert_eq!(pos("cmd ${1:+foo}"), s(&["${1:+foo}"]));
        assert_eq!(pos("cmd ${1:+}"), s(&["${1:+}"]));
        Ok(())
    }

    #[test]
    fn test_positionals_braced_alternate_no_colon() -> Result<()> {
        assert_eq!(pos("cmd ${1+alt}"), s(&["${1+alt}"]));
        Ok(())
    }

    // ── Positive: error operators ─────────────────────────────────────────────

    #[test]
    fn test_positionals_braced_error_colon_question() -> Result<()> {
        assert_eq!(pos("cmd ${1:?msg}"), s(&["${1:?msg}"]));
        assert_eq!(pos("cmd ${1:?}"), s(&["${1:?}"]));
        Ok(())
    }

    #[test]
    fn test_positionals_braced_error_no_colon() -> Result<()> {
        assert_eq!(pos("cmd ${1?msg}"), s(&["${1?msg}"]));
        Ok(())
    }

    // ── Positive: assign-default operators ───────────────────────────────────

    #[test]
    fn test_positionals_braced_assign_colon_equals() -> Result<()> {
        assert_eq!(pos("cmd ${1:=val}"), s(&["${1:=val}"]));
        Ok(())
    }

    #[test]
    fn test_positionals_braced_assign_no_colon() -> Result<()> {
        assert_eq!(pos("cmd ${1=val}"), s(&["${1=val}"]));
        Ok(())
    }

    // ── Positive: length operator ─────────────────────────────────────────────

    #[test]
    fn test_positionals_braced_length() -> Result<()> {
        assert_eq!(pos("cmd ${#1}"), s(&["${#1}"]));
        assert_eq!(pos("cmd ${#9}"), s(&["${#9}"]));
        Ok(())
    }

    // ── Positive: substring operators ────────────────────────────────────────

    #[test]
    fn test_positionals_braced_substring() -> Result<()> {
        assert_eq!(pos("cmd ${1:2}"), s(&["${1:2}"]));
        assert_eq!(pos("cmd ${1:2:3}"), s(&["${1:2:3}"]));
        Ok(())
    }

    // ── Positive: pattern substitution ───────────────────────────────────────

    #[test]
    fn test_positionals_braced_substitution() -> Result<()> {
        assert_eq!(pos("cmd ${1/x/y}"), s(&["${1/x/y}"]));
        assert_eq!(pos("cmd ${1//x/y}"), s(&["${1//x/y}"])); // global
        assert_eq!(pos("cmd ${1/#x/y}"), s(&["${1/#x/y}"])); // prefix anchor
        assert_eq!(pos("cmd ${1/%x/y}"), s(&["${1/%x/y}"])); // suffix anchor
        Ok(())
    }

    // ── Positive: pattern removal ────────────────────────────────────────────

    #[test]
    fn test_positionals_braced_removal() -> Result<()> {
        assert_eq!(pos("cmd ${1#pat}"), s(&["${1#pat}"]));   // remove shortest prefix
        assert_eq!(pos("cmd ${1##pat}"), s(&["${1##pat}"])); // remove longest prefix
        assert_eq!(pos("cmd ${1%pat}"), s(&["${1%pat}"]));   // remove shortest suffix
        assert_eq!(pos("cmd ${1%%pat}"), s(&["${1%%pat}"])); // remove longest suffix
        Ok(())
    }

    // ── Positive: case modification ──────────────────────────────────────────

    #[test]
    fn test_positionals_braced_case() -> Result<()> {
        assert_eq!(pos("cmd ${1^}"), s(&["${1^}"]));   // uppercase first
        assert_eq!(pos("cmd ${1^^}"), s(&["${1^^}"])); // uppercase all
        assert_eq!(pos("cmd ${1,}"), s(&["${1,}"]));   // lowercase first
        assert_eq!(pos("cmd ${1,,}"), s(&["${1,,}"])); // lowercase all
        Ok(())
    }

    // ── Positive: mixed braced positionals ───────────────────────────────────

    #[test]
    fn test_positionals_braced_multiple() -> Result<()> {
        assert_eq!(
            pos("cargo metadata --manifest-path ${1:-.}/Cargo.toml --filter-platform ${2}"),
            s(&["${1:-.}", "${2}"])
        );
        Ok(())
    }

    // ── Dedup: same positional number, different forms ────────────────────────

    #[test]
    fn test_positionals_dedup_same_bare() -> Result<()> {
        // Same bare token repeated → one entry
        assert_eq!(pos("echo $1 $1"), s(&["$1"]));
        Ok(())
    }

    #[test]
    fn test_positionals_dedup_longest_wins_bare_first() -> Result<()> {
        // $1 appears before ${1:-.}: longest (braced) wins; bare $1 is NOT the
        // replacement target, preventing corruption via naive str::replace
        assert_eq!(pos("echo $1 ${1:-.}"), s(&["${1:-.}"]));
        Ok(())
    }

    #[test]
    fn test_positionals_dedup_longest_wins_braced_first() -> Result<()> {
        // ${1:-.} appears before $1: longest still wins
        assert_eq!(pos("echo ${1:-.} $1"), s(&["${1:-.}"]));
        Ok(())
    }

    // ── Negative: $0 is not a positional ─────────────────────────────────────

    #[test]
    fn test_positionals_zero_not_detected() -> Result<()> {
        assert_eq!(pos("echo $0"), s(&[]));
        assert_eq!(pos("echo ${0}"), s(&[]));
        assert_eq!(pos("echo ${0:-default}"), s(&[]));
        Ok(())
    }

    // ── Negative: special shell variables ────────────────────────────────────

    #[test]
    fn test_positionals_special_vars_not_detected() -> Result<()> {
        assert_eq!(pos("echo $@"), s(&[])); // all args — handled by is_variadic()
        assert_eq!(pos("echo $*"), s(&[])); // all args (IFS-joined)
        assert_eq!(pos("echo $#"), s(&[])); // argument count
        assert_eq!(pos("echo $?"), s(&[])); // exit status
        assert_eq!(pos("echo $$"), s(&[])); // shell PID
        assert_eq!(pos("echo $!"), s(&[])); // last background PID
        Ok(())
    }

    // ── Negative: named variables ─────────────────────────────────────────────

    #[test]
    fn test_positionals_named_vars_not_detected() -> Result<()> {
        assert_eq!(pos("echo $HOME"), s(&[]));
        assert_eq!(pos("echo ${HOME}"), s(&[]));
        assert_eq!(pos("echo ${HOME:-default}"), s(&[]));
        assert_eq!(pos("echo ${#HOME}"), s(&[])); // length of named var
        assert_eq!(pos("cd $dir && ls $path"), s(&[]));
        Ok(())
    }

    // ── Negative: $$N escape ─────────────────────────────────────────────────

    #[test]
    fn test_positionals_double_dollar_excluded() -> Result<()> {
        // $$1 is awk/perl-style escaped dollar — not a positional
        assert_eq!(pos("awk '{print $$1}'"), s(&[]));
        assert_eq!(pos("echo $$1 $$2"), s(&[]));
        Ok(())
    }

    #[test]
    fn test_positionals_double_dollar_mixed() -> Result<()> {
        // $$1 excluded but $2 is a real positional
        assert_eq!(pos("echo $$1 $2"), s(&["$2"]));
        Ok(())
    }

    // ── Negative: no positionals ─────────────────────────────────────────────

    #[test]
    fn test_positionals_none_plain_command() -> Result<()> {
        assert_eq!(pos("git status"), s(&[]));
        assert_eq!(pos("echo Hello World"), s(&[]));
        Ok(())
    }

    #[test]
    fn test_positionals_empty_value() -> Result<()> {
        assert_eq!(pos(""), s(&[]));
        Ok(())
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn test_positionals_dollar10_matches_dollar1() -> Result<()> {
        // $10 in bash is ${1} followed by literal 0 — aka matches $1 (same as today)
        assert_eq!(pos("echo $10"), s(&["$1"]));
        Ok(())
    }

    #[test]
    fn test_positionals_nested_brace_default() -> Result<()> {
        // ${1:-${2}} — [^}]* stops at the inner }, producing ${1:-${2} as the token.
        // This is a degenerate alias; we assert it does not crash and detects $1.
        let result = pos("cmd ${1:-${2}}");
        assert!(result.len() == 1 || result.len() == 2, "should detect at least $1");
        assert!(result.iter().any(|t| t.contains('1')), "positional 1 must be present");
        Ok(())
    }

    #[test]
    fn test_positionals_named_var_with_positional_default() -> Result<()> {
        // ${HOME:-${1}} — outer is a named var (not matched); inner ${1} IS a positional
        assert_eq!(pos("cd ${HOME:-${1}}"), s(&["${1}"]));
        Ok(())
    }

    // ── Replacement: braced forms ─────────────────────────────────────────────

    #[test]
    fn test_replace_braced_bare() -> Result<()> {
        let alias = mk("cmd ${1}");
        let (result, count) = alias.replace(&mut vec!["foo".into()], &HashMap::new(), true)?;
        assert_eq!(result, "cmd foo");
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn test_replace_braced_default_with_arg() -> Result<()> {
        // When an arg is supplied, aka replaces the full token; the default is ignored
        let alias = mk("cargo metadata --manifest-path ${1:-.}/Cargo.toml");
        let (result, count) = alias.replace(&mut vec!["./proj".into()], &HashMap::new(), true)?;
        assert_eq!(result, "cargo metadata --manifest-path ./proj/Cargo.toml");
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn test_replace_braced_default_no_arg() -> Result<()> {
        // No arg → positionals.len() != remainders.len() → alias name returned
        let alias = mk("cargo metadata --manifest-path ${1:-.}/Cargo.toml");
        let alias = Alias { name: "cip".to_string(), ..alias };
        let (result, count) = alias.replace(&mut vec![], &HashMap::new(), true)?;
        assert_eq!(result, "cip");
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn test_replace_length_form() -> Result<()> {
        // ${#1} is replaced with the supplied arg (aka substitutes the token, not its length)
        let alias = mk("cmd ${#1}");
        let (result, count) = alias.replace(&mut vec!["hello".into()], &HashMap::new(), true)?;
        assert_eq!(result, "cmd hello");
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn test_replace_degenerate_mixed_forms() -> Result<()> {
        // Alias has both $1 and ${1:-.} — longest wins → ${1:-.} is the replacement
        // target; bare $1 is left literal in output (benign, not corrupted syntax)
        let alias = mk("echo $1 ${1:-.}");
        let (result, count) = alias.replace(&mut vec!["foo".into()], &HashMap::new(), true)?;
        assert_eq!(result, "echo $1 foo"); // ${1:-.} replaced; bare $1 remains literal
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn test_replace_degenerate_mixed_forms_reversed() -> Result<()> {
        // Same alias with token order reversed — longest wins regardless of position
        let alias = mk("echo ${1:-.} $1");
        let (result, count) = alias.replace(&mut vec!["foo".into()], &HashMap::new(), true)?;
        assert_eq!(result, "echo foo $1"); // ${1:-.} replaced; bare $1 remains literal
        assert_eq!(count, 1);
        Ok(())
    }

    // ── Unchanged behaviour: variadic, escaped dollar, mismatch ──────────────

    #[test]
    fn test_replace_variadic() -> Result<()> {
        let alias = mk("echo $@");
        let mut rem = vec!["Hello".into(), "from".into(), "Rust".into()];
        let (result, count) = alias.replace(&mut rem, &HashMap::new(), true)?;
        assert_eq!(result, "echo Hello from Rust");
        assert_eq!(count, 3);
        assert!(rem.is_empty());
        Ok(())
    }

    #[test]
    fn test_replace_variadic_eol_false() -> Result<()> {
        let alias = Alias { name: "a".to_string(), ..mk("echo $@") };
        let mut rem = vec!["Hello".into(), "World".into()];
        let (result, count) = alias.replace(&mut rem, &HashMap::new(), false)?;
        assert_eq!(result, "a");
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn test_replace_mismatch_remainders() -> Result<()> {
        let alias = Alias { name: "cmd".to_string(), ..mk("echo $1 $2 $3") };
        let mut rem = vec!["Hello".into(), "World".into()];
        let (result, count) = alias.replace(&mut rem, &HashMap::new(), true)?;
        assert_eq!(result, "cmd"); // alias name returned when count mismatch
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn test_replace_escaped_dollar_unescaped() -> Result<()> {
        // $$ → $ after all replacements
        let alias = mk("echo $$HOME");
        let (result, _) = alias.replace(&mut vec![], &HashMap::new(), true)?;
        assert_eq!(result, "echo $HOME");
        Ok(())
    }

    #[test]
    fn test_positionals_double_dollar_regression() -> Result<()> {
        // Regression: $$1 must never be treated as positional, even with new regex
        let alias = mk("awk '{print $$1}' $2");
        assert_eq!(alias.positionals()?, s(&["$2"]));
        Ok(())
    }

    // ── Other Alias methods (unchanged) ──────────────────────────────────────

    #[test]
    fn test_variable_references() -> Result<()> {
        assert_eq!(
            mk("echo $name $location").variable_references()?,
            s(&["location", "name"])
        );
        Ok(())
    }

    #[test]
    fn test_variable_references_empty_value() -> Result<()> {
        assert_eq!(mk("").variable_references()?, s(&[]));
        Ok(())
    }

    #[test]
    fn test_is_variadic_true() {
        assert!(mk("echo $@").is_variadic());
    }

    #[test]
    fn test_is_variadic_false() {
        assert!(!mk("echo hello").is_variadic());
    }

    #[test]
    fn test_from_str() -> Result<()> {
        let alias = "echo Hello World".parse::<Alias>()?;
        assert_eq!(alias.name, "");
        assert_eq!(alias.value, "echo Hello World");
        assert!(alias.space);
        assert!(!alias.global);
        assert_eq!(alias.count, 0);
        Ok(())
    }

    #[test]
    fn test_alias_default() {
        let alias = Alias::default();
        assert!(alias.name.is_empty());
        assert!(alias.value.is_empty());
        assert!(!alias.space);
        assert!(!alias.global);
        assert_eq!(alias.count, 0);
    }

    #[test]
    fn test_alias_clone_and_eq() {
        let alias = Alias { name: "t".to_string(), value: "echo t".to_string(), space: false, global: true, count: 10 };
        assert_eq!(alias, alias.clone());
    }

    #[test]
    fn test_alias_debug_contains_fields() {
        let alias = Alias { name: "t".to_string(), value: "echo t".to_string(), space: true, global: false, count: 0 };
        let s = format!("{alias:?}");
        assert!(s.contains("t"));
        assert!(s.contains("echo t"));
    }

    #[test]
    fn test_alias_serialize_contains_fields() {
        let alias = Alias { name: "t".to_string(), value: "echo t".to_string(), space: true, global: false, count: 5 };
        let json = serde_json::to_string(&alias).unwrap();
        assert!(json.contains("echo t"));
        assert!(json.contains("space"));
    }
}
