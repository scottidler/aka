//! Shell integration module for aka
//!
//! This module provides shell initialization scripts that are embedded into the binary
//! at compile time. The `.zsh` files in this directory are included via `include_str!()`
//! so they can be edited with proper syntax highlighting while still being distributed
//! as part of the single binary.
//!
//! # Usage
//!
//! Users add this to their `.zshrc`:
//! ```zsh
//! if hash aka 2>/dev/null; then
//!     eval "$(aka shell-init zsh)"
//! fi
//! ```

/// ZSH initialization script embedded at compile time
///
/// This script is loaded from `src/shell/init.zsh` and provides:
/// - Space key alias expansion while typing
/// - Accept-line alias expansion before execution
/// - Ctrl+t fuzzy alias search (requires sk or fzf)
/// - Killswitch support via ~/aka-killswitch file
/// - Completion integration
pub const ZSH_INIT_SCRIPT: &str = include_str!("init.zsh");

/// Generate shell initialization script for the specified shell
///
/// # Arguments
/// * `shell` - Shell type (currently only "zsh" is supported)
///
/// # Returns
/// * `Some(&str)` - The initialization script for the requested shell
/// * `None` - If the shell is not supported
///
/// # Example
/// ```
/// use aka_lib::shell::generate_init_script;
///
/// if let Some(script) = generate_init_script("zsh") {
///     print!("{}", script);
/// }
/// ```
pub fn generate_init_script(shell: &str) -> Option<&'static str> {
    match shell.to_lowercase().as_str() {
        "zsh" => Some(ZSH_INIT_SCRIPT),
        // Future: add bash, fish support
        // "bash" => Some(BASH_INIT_SCRIPT),
        // "fish" => Some(FISH_INIT_SCRIPT),
        _ => None,
    }
}

/// List of supported shells
pub fn supported_shells() -> &'static [&'static str] {
    &["zsh"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::const_is_empty)]
    fn test_zsh_init_script_not_empty() {
        // ZSH_INIT_SCRIPT is a const, so we verify it has content
        assert!(!ZSH_INIT_SCRIPT.is_empty());
    }

    #[test]
    fn test_zsh_init_script_contains_key_functions() {
        let script = ZSH_INIT_SCRIPT;

        // Must have killswitch function
        assert!(script.contains("_aka_killswitch"));

        // Must have space expansion
        assert!(script.contains("_aka_expand_space"));

        // Must have accept-line handler
        assert!(script.contains("_aka_accept_line"));

        // Must have search function
        assert!(script.contains("_aka_search"));
    }

    #[test]
    fn test_zsh_init_script_has_key_bindings() {
        let script = ZSH_INIT_SCRIPT;

        // Space key binding
        assert!(script.contains(r#"bindkey " " _aka_expand_space"#));

        // Ctrl+t binding for search
        assert!(script.contains(r#"bindkey "^t" _aka_search"#));
    }

    #[test]
    fn test_zsh_init_script_registers_widgets() {
        let script = ZSH_INIT_SCRIPT;

        assert!(script.contains("zle -N _aka_expand_space"));
        assert!(script.contains("zle -N _aka_accept_line"));
        assert!(script.contains("zle -N _aka_search"));
        assert!(script.contains("zle -N accept-line _aka_accept_line"));
    }

    #[test]
    fn test_zsh_init_script_has_completion_support() {
        let script = ZSH_INIT_SCRIPT;

        assert!(script.contains("_aka_complete_commands"));
        assert!(script.contains("aka __complete_aliases"));
        assert!(script.contains("compdef"));
    }

    #[test]
    fn test_generate_init_script_zsh() {
        let script = generate_init_script("zsh");
        assert!(script.is_some());
        assert_eq!(script.unwrap(), ZSH_INIT_SCRIPT);
    }

    #[test]
    fn test_generate_init_script_zsh_case_insensitive() {
        assert!(generate_init_script("ZSH").is_some());
        assert!(generate_init_script("Zsh").is_some());
        assert!(generate_init_script("ZsH").is_some());
    }

    #[test]
    fn test_generate_init_script_unsupported() {
        assert!(generate_init_script("bash").is_none());
        assert!(generate_init_script("fish").is_none());
        assert!(generate_init_script("").is_none());
        assert!(generate_init_script("invalid").is_none());
    }

    #[test]
    fn test_supported_shells() {
        let shells = supported_shells();
        assert!(!shells.is_empty());
        assert!(shells.contains(&"zsh"));
    }

    #[test]
    fn test_zsh_init_script_has_header_comment() {
        let script = ZSH_INIT_SCRIPT;

        // Should have usage instructions
        assert!(script.contains("eval \"$(aka shell-init zsh)\""));
    }

    #[test]
    fn test_zsh_init_script_killswitch_path() {
        let script = ZSH_INIT_SCRIPT;

        // Should reference the killswitch file
        assert!(script.contains("~/aka-killswitch"));
    }

    #[test]
    fn test_zsh_init_script_calls_aka_binary() {
        let script = ZSH_INIT_SCRIPT;

        // Should call aka query
        assert!(script.contains("aka query"));

        // Should call aka --eol query
        assert!(script.contains("aka --eol query"));

        // Should call aka ls for search
        assert!(script.contains("aka ls"));
    }
}
